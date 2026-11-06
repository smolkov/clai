//! A session as an actor (see `spec/sessions.md`): one task owns its agent,
//! state, message queue, event log and open question. Clients reach it
//! through a [`SessionHandle`]; it publishes events to every attached
//! client.

mod outbox;
mod questions;
pub mod store;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use futures_util::future::BoxFuture;
use tokio::sync::{mpsc, oneshot};
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::agent::{
    Agent, Answer, Asker, CancelReason, EventSink, ModelHandle, Question, Reply, SessionEvent,
    SessionRef, State,
};
use crate::config::Model;
use crate::protocol::{ErrorReply, Outcome, Payload, ServerMessage, SessionInfo, TokenTotals};
use crate::provider::Provider;

use questions::{Pending, Questions};

pub use outbox::{ClientId, MAX_QUEUED, Outbox, OutboxReceiver};
pub use store::{Meta, Store};

const GONE: &str = "the session has ended";

/// What a new or resumed session starts with.
pub struct Spec {
    pub id: u64,
    /// Name, workspace, model, turns, queued messages.
    pub meta: Meta,
    /// Where it is saved; `None` keeps it in memory only.
    pub store: Option<Store>,
    /// With its history and session rules, if resumed.
    pub agent: Agent,
    /// The event log so far, if resumed.
    pub log: Vec<SessionEvent>,
}

/// Commands to a session.
enum Msg {
    Attach {
        outbox: Outbox,
        answers: bool,
        /// The id of the attach request; the session sends its response
        /// before the log, so the client gets them in order.
        request: u64,
    },
    Detach {
        client: ClientId,
    },
    Send {
        text: String,
    },
    Cancel,
    SetModel {
        model: String,
        provider: Arc<dyn Provider>,
        config: Model,
    },
    Rename {
        name: String,
    },
    Answer {
        question: String,
        answer: Answer,
        by: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Info {
        reply: oneshot::Sender<SessionInfo>,
    },
    Kill {
        reason: String,
        /// Keep the saved files and leave `ended` out of them: the server
        /// stops, the session can be resumed.
        keep: bool,
        reply: oneshot::Sender<()>,
    },
}

/// From the agent's asker and journal to the session.
enum AskMsg {
    /// Writing the history failed.
    SaveFailed(String),
    Ask {
        question: String,
        reply: oneshot::Sender<Reply>,
    },
    Rejected {
        question: String,
        message: String,
    },
}

/// The way to a running session. Cheap to clone; every call fails with
/// "the session has ended" once it has.
#[derive(Clone)]
pub struct SessionHandle {
    pub id: u64,
    tx: mpsc::UnboundedSender<Msg>,
}

impl SessionHandle {
    fn send(&self, msg: Msg) -> Result<(), String> {
        self.tx.send(msg).map_err(|_| GONE.to_owned())
    }

    /// Attaches a client: the session sends the response to request
    /// `request`, then its event log, then live events.
    pub fn attach(&self, outbox: Outbox, answers: bool, request: u64) -> Result<(), String> {
        self.send(Msg::Attach {
            outbox,
            answers,
            request,
        })
    }

    pub fn detach(&self, client: ClientId) {
        let _ = self.send(Msg::Detach { client });
    }

    /// A user message: starts a turn, or is queued while one runs.
    pub fn message(&self, text: String) -> Result<(), String> {
        self.send(Msg::Send { text })
    }

    pub fn cancel(&self) -> Result<(), String> {
        self.send(Msg::Cancel)
    }

    pub fn set_model(
        &self,
        model: String,
        provider: Arc<dyn Provider>,
        config: Model,
    ) -> Result<(), String> {
        self.send(Msg::SetModel {
            model,
            provider,
            config,
        })
    }

    pub fn rename(&self, name: String) -> Result<(), String> {
        self.send(Msg::Rename { name })
    }

    /// Answers question `question`; `Ok` once the answer counted, `Err`
    /// with the reason it did not.
    pub async fn answer(&self, question: String, answer: Answer, by: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.send(Msg::Answer {
            question,
            answer,
            by,
            reply,
        })?;
        rx.await.unwrap_or_else(|_| Err(GONE.to_owned()))
    }

    pub async fn info(&self) -> Option<SessionInfo> {
        let (reply, rx) = oneshot::channel();
        self.send(Msg::Info { reply }).ok()?;
        rx.await.ok()
    }

    /// Ends the session, cancelling a running turn first; returns when it
    /// has ended. With `keep`, the saved files stay valid for a resume.
    pub async fn kill(&self, reason: &str, keep: bool) {
        let (reply, rx) = oneshot::channel();
        if self
            .send(Msg::Kill {
                reason: reason.to_owned(),
                keep,
                reply,
            })
            .is_ok()
        {
            let _ = rx.await;
        }
    }
}

/// Starts the session task. `on_end` runs once it has ended, however it
/// ended (killed, failed, or a bug in the task itself).
pub fn spawn(spec: Spec, on_end: impl FnOnce(u64) + Send + 'static) -> SessionHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    let id = spec.id;
    let actor = Actor::new(spec, rx);
    tracing::info!(session = %actor.meta.name, "session started");
    let task = tokio::spawn(actor.run());
    tokio::spawn(async move {
        if let Err(e) = task.await {
            tracing::error!(session = id, "session task failed: {e}");
        }
        on_end(id);
    });
    SessionHandle { id, tx }
}

/// Answers questions with what clients send, through the session task.
struct SessionAsker {
    tx: mpsc::UnboundedSender<AskMsg>,
}

impl Asker for SessionAsker {
    fn ask<'a>(&'a self, question: &'a Question) -> BoxFuture<'a, Reply> {
        let (reply, rx) = oneshot::channel();
        let sent = self.tx.send(AskMsg::Ask {
            question: question.id.clone(),
            reply,
        });
        Box::pin(async move {
            if sent.is_err() {
                return std::future::pending().await;
            }
            // The session never answers by itself; if it is gone, the
            // turn is being cancelled anyway.
            match rx.await {
                Ok(reply) => reply,
                Err(_) => std::future::pending().await,
            }
        })
    }

    fn reject(&self, question: &Question, _by: &str, message: &str) {
        let _ = self.tx.send(AskMsg::Rejected {
            question: question.id.clone(),
            message: message.to_owned(),
        });
    }
}

struct Turn {
    join: JoinHandle<Agent>,
    cancel: CancellationToken,
}

struct Client {
    outbox: Outbox,
    answers: bool,
}

enum Wake {
    Msg(Option<Msg>),
    Ask(Option<AskMsg>),
    Event(SessionEvent),
    TurnDone(Result<Agent, JoinError>),
}

enum Flow {
    Go,
    Stop,
}

struct Actor {
    id: u64,
    meta: Meta,
    store: Option<Store>,
    /// Writing failed last time; said once until it works again.
    save_failed: bool,
    model_handle: ModelHandle,
    cancel_reason: CancelReason,
    /// `None` while a turn has it.
    agent: Option<Agent>,
    turn: Option<Turn>,
    state: State,
    queue: VecDeque<String>,
    log: Vec<SessionEvent>,
    clients: HashMap<ClientId, Client>,
    questions: Questions,
    rx: mpsc::UnboundedReceiver<Msg>,
    ask_rx: mpsc::UnboundedReceiver<AskMsg>,
    events: mpsc::UnboundedReceiver<SessionEvent>,
    sink: EventSink,
    asker: Arc<SessionAsker>,
}

impl Actor {
    fn new(spec: Spec, rx: mpsc::UnboundedReceiver<Msg>) -> Actor {
        let (sink, events) = EventSink::channel();
        let (ask_tx, ask_rx) = mpsc::unbounded_channel();
        let mut agent = spec.agent;
        if let Some(store) = spec.store.clone() {
            let tx = ask_tx.clone();
            agent.set_journal(Arc::new(move |record| {
                if let Err(e) = store.append_record(&record) {
                    let _ = tx.send(AskMsg::SaveFailed(e));
                }
            }));
        }
        // A turn a stopped server left open gets results for its calls.
        agent.repair();
        let queue = spec.meta.queued.iter().cloned().collect();
        // The log is the truth: this covers a session of an older `clai`, and a
        // crash between writing an event and writing the meta.
        let meta = Meta {
            usage: TokenTotals::of_log(&spec.log),
            ..spec.meta
        };
        Actor {
            id: spec.id,
            meta,
            store: spec.store,
            save_failed: false,
            model_handle: agent.model(),
            cancel_reason: agent.cancel_reason(),
            agent: Some(agent),
            turn: None,
            state: State::Idle,
            queue,
            log: spec.log,
            clients: HashMap::new(),
            questions: Questions::default(),
            rx,
            ask_rx,
            events,
            sink,
            asker: Arc::new(SessionAsker { tx: ask_tx }),
        }
    }

    async fn run(mut self) {
        // Messages queued before a stop run now.
        self.next_turn();
        loop {
            let wake = tokio::select! {
                msg = self.rx.recv() => Wake::Msg(msg),
                msg = self.ask_rx.recv() => Wake::Ask(msg),
                Some(event) = self.events.recv() => Wake::Event(event),
                done = join(&mut self.turn) => Wake::TurnDone(done),
            };
            let flow = match wake {
                Wake::Msg(Some(msg)) => self.handle(msg).await,
                // Every handle is gone: the server is stopping.
                Wake::Msg(None) => {
                    self.end("server stopped", true).await;
                    Flow::Stop
                }
                Wake::Ask(Some(msg)) => {
                    self.handle_ask(msg);
                    Flow::Go
                }
                Wake::Ask(None) => Flow::Go,
                Wake::Event(event) => {
                    self.publish(event);
                    Flow::Go
                }
                Wake::TurnDone(done) => {
                    self.turn = None;
                    self.turn_done(done)
                }
            };
            if let Flow::Stop = flow {
                return;
            }
        }
    }

    async fn handle(&mut self, msg: Msg) -> Flow {
        match msg {
            Msg::Attach {
                outbox,
                answers,
                request,
            } => self.attach(outbox, answers, request),
            Msg::Detach { client } => {
                self.clients.remove(&client);
            }
            Msg::Send { text } => {
                self.queue.push_back(text);
                if !self.next_turn() {
                    self.save_meta();
                }
            }
            Msg::Cancel => {
                if let Some(turn) = &self.turn {
                    turn.cancel.cancel();
                }
            }
            Msg::SetModel {
                model,
                provider,
                config,
            } => {
                // The same name may come with a reloaded configuration.
                self.model_handle.set(provider, config);
                if model != self.meta.model {
                    let from = std::mem::replace(&mut self.meta.model, model.clone());
                    self.save_meta();
                    self.publish(SessionEvent::ModelChanged { from, model });
                }
            }
            Msg::Rename { name } => {
                self.meta.name.clone_from(&name);
                self.save_meta();
                self.questions.rename(&name);
                self.publish(SessionEvent::Renamed { name });
            }
            Msg::Answer {
                question,
                answer,
                by,
                reply,
            } => self.questions.answer(
                Pending {
                    question,
                    answer,
                    by,
                    reply,
                },
                &self.meta.name,
            ),
            Msg::Info { reply } => {
                let _ = reply.send(self.info());
            }
            Msg::Kill {
                reason,
                keep,
                reply,
            } => {
                if !keep {
                    // The files are about to go; nothing more to write.
                    self.store = None;
                }
                self.end(&reason, keep).await;
                let _ = reply.send(());
                return Flow::Stop;
            }
        }
        Flow::Go
    }

    fn info(&self) -> SessionInfo {
        self.meta.info(
            self.id,
            self.state,
            self.queue.len(),
            self.questions.current().cloned(),
        )
    }

    fn attach(&mut self, outbox: Outbox, answers: bool, request: u64) {
        outbox.send_always(ServerMessage::Response {
            id: request,
            result: Outcome::Ok(Payload::Session {
                session: Box::new(self.info()),
            }),
        });
        for event in &self.log {
            outbox.send_always(ServerMessage::Event {
                session: self.id,
                replay: true,
                event: for_client(event, answers),
            });
        }
        self.clients.insert(outbox.id, Client { outbox, answers });
    }

    /// Adds an event to the log and sends it to every attached client.
    fn publish(&mut self, mut event: SessionEvent) {
        match &mut event {
            SessionEvent::State { state } => self.state = *state,
            SessionEvent::Question(question) => {
                question.session = SessionRef {
                    name: self.meta.name.clone(),
                    workspace: self.meta.place.clone(),
                };
                self.questions.opened((**question).clone());
            }
            SessionEvent::QuestionClosed { id, closed } => {
                self.questions.closed(id, closed);
            }
            _ => {}
        }
        for client in self.clients.values() {
            client.outbox.send(ServerMessage::Event {
                session: self.id,
                replay: false,
                event: for_client(&event, client.answers),
            });
        }
        // First the event, then the meta: after a crash between the two, the
        // log has the count and the next resume sums it.
        self.save_event(&event);
        if matches!(event, SessionEvent::Usage { .. }) {
            self.meta.usage.count(&event);
            self.save_meta();
        }
        self.log.push(event);
    }

    /// Appends an event to the saved log. `ended` is never saved: a
    /// killed session's files are deleted, a stopped server's sessions
    /// are resumed.
    fn save_event(&mut self, event: &SessionEvent) {
        if matches!(event, SessionEvent::Ended { .. }) {
            return;
        }
        let result = match &self.store {
            Some(store) => store.append_event(event),
            None => return,
        };
        self.saved(result);
    }

    /// Writes `session.json` with the current name, model, turns, queue and
    /// token totals.
    fn save_meta(&mut self) {
        let Some(store) = &self.store else {
            return;
        };
        self.meta.queued = self.queue.iter().cloned().collect();
        self.meta.updated = crate::agent::now_secs();
        let result = store.write_meta(&self.meta);
        self.saved(result);
    }

    fn saved(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => self.save_failed = false,
            Err(e) => self.save_failed(&e),
        }
    }

    /// Says once that saving fails; the session goes on in memory.
    fn save_failed(&mut self, error: &str) {
        tracing::error!(session = %self.meta.name, "cannot save: {error}");
        if std::mem::replace(&mut self.save_failed, true) {
            return;
        }
        let event = SessionEvent::Notice {
            text: format!("cannot save the session: {error}; it goes on in memory"),
        };
        for client in self.clients.values() {
            client.outbox.send(ServerMessage::Event {
                session: self.id,
                replay: false,
                event: event.clone(),
            });
        }
        self.log.push(event);
    }

    fn handle_ask(&mut self, msg: AskMsg) {
        match msg {
            AskMsg::SaveFailed(e) => self.save_failed(&e),
            AskMsg::Ask { question, reply } => self.questions.ask(question, reply),
            AskMsg::Rejected { question, message } => self.questions.rejected(&question, message),
        }
    }

    /// Starts a turn with the next queued message, if none runs; returns
    /// whether it did.
    fn next_turn(&mut self) -> bool {
        if self.turn.is_some() {
            return false;
        }
        let Some(mut agent) = self.agent.take() else {
            return false;
        };
        let Some(text) = self.queue.pop_front() else {
            self.agent = Some(agent);
            return false;
        };
        self.meta.turns += 1;
        self.save_meta();
        let cancel = CancellationToken::new();
        let sink = self.sink.clone();
        let asker = self.asker.clone();
        let token = cancel.clone();
        let join = tokio::spawn(async move {
            agent.run_turn(text, &sink, asker.as_ref(), token).await;
            agent
        });
        self.turn = Some(Turn { join, cancel });
        true
    }

    fn turn_done(&mut self, done: Result<Agent, JoinError>) -> Flow {
        self.drain_events();
        match done {
            Ok(agent) => {
                self.agent = Some(agent);
                self.next_turn();
                Flow::Go
            }
            Err(e) => {
                tracing::error!(session = %self.meta.name, "turn task failed: {e}");
                self.publish(SessionEvent::Error {
                    message: format!("the session failed: {}", join_error(&e)),
                });
                self.finish("failed");
                Flow::Stop
            }
        }
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.publish(event);
        }
    }

    /// Cancels a running turn, waits for it, and ends the session.
    async fn end(&mut self, reason: &str, keep: bool) {
        if let Some(turn) = self.turn.take() {
            if keep {
                self.cancel_reason.set("the server stopped");
            }
            turn.cancel.cancel();
            let done = turn.join.await;
            self.drain_events();
            if let Err(e) = done {
                tracing::error!(session = %self.meta.name, "turn task failed: {e}");
            }
        }
        if keep {
            self.save_meta();
        }
        self.finish(reason);
    }

    fn finish(&mut self, reason: &str) {
        self.questions.end(GONE);
        self.publish(SessionEvent::Ended {
            reason: reason.to_owned(),
        });
        tracing::info!(session = %self.meta.name, reason, "session ended");
    }
}

/// The event as a client gets it: clients that only watch see a question
/// without choices.
fn for_client(event: &SessionEvent, answers: bool) -> SessionEvent {
    match event {
        SessionEvent::Question(question) if !answers => {
            let mut question = question.clone();
            question.choices.clear();
            SessionEvent::Question(question)
        }
        other => other.clone(),
    }
}

async fn join(turn: &mut Option<Turn>) -> Result<Agent, JoinError> {
    match turn {
        Some(turn) => (&mut turn.join).await,
        None => std::future::pending().await,
    }
}

fn join_error(e: &JoinError) -> String {
    if e.is_panic() {
        "internal error (panic); see the log".to_owned()
    } else {
        e.to_string()
    }
}

/// The error reply for a request to a session that has ended.
pub fn gone() -> ErrorReply {
    ErrorReply::failed(GONE)
}
