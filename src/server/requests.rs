//! Requests of one connection: sessions are created, resumed from disk
//! when a request names a saved one, killed, renamed, exported, imported.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::agent::{Agent, SessionEvent, Setup, State, now_secs, system_prompt, today};
use crate::config::{self, Config, LoadOptions};
use crate::permissions::{SessionRules, Workspace};
use crate::protocol::{
    ErrorReply, ModelInfo, Notice, Outcome, Payload, Request, SessionInfo, Status,
};
use crate::session::store::{self, Meta, Store};
use crate::session::{self, SessionHandle, Spec};
use crate::tools::Env;

use super::{Conn, Entry, Shared, Slot, broadcast, busy, check_name, error_lines, lock};

type Reply = std::result::Result<Payload, ErrorReply>;

/// A session found by name.
enum Found {
    Running(SessionHandle),
    Saved,
}

fn no_session(name: &str) -> ErrorReply {
    ErrorReply::failed(format!("no session named {name}"))
}

/// What a session needs from the configuration: its agent and where it
/// works.
struct Built {
    agent: Agent,
    place: String,
    path: PathBuf,
    /// `provider/model`.
    model: String,
}

impl Shared {
    /// The configuration for a workspace: the kept user file and the
    /// workspace's project file.
    fn config(&self, workspace: &Path) -> std::result::Result<Config, ErrorReply> {
        let (path, text) = {
            let user = lock(&self.config);
            (user.path.clone(), user.text.clone())
        };
        let options = LoadOptions {
            workspace: workspace.to_path_buf(),
            ..self.load.clone()
        };
        config::load_with_user_text(&path, &text, &options)
            .map_err(|e| ErrorReply::usage(error_lines(&e).join("\n")))
    }

    /// An agent for `workspace` with model `model` (`provider/model` or
    /// the short form; `None` for `default_model`).
    fn build(
        &self,
        workspace: &Path,
        model: Option<&str>,
    ) -> std::result::Result<Built, ErrorReply> {
        let config = self.config(workspace)?;
        let model_ref = match model {
            Some(text) => config.resolve_model(text).map_err(ErrorReply::usage)?,
            None => config.default_model.clone(),
        };
        let Some((provider_config, model_config)) = config.model(&model_ref) else {
            return Err(ErrorReply::usage(format!("unknown model `{model_ref}`")));
        };
        let provider = (self.providers)(provider_config)
            .map_err(|e| ErrorReply::failed(format!("provider {}: {e}", provider_config.name)))?;
        let ws = Workspace::new(
            workspace,
            &config.permissions.extra_dirs,
            self.load.env.home.clone(),
        )
        .map_err(|e| ErrorReply::failed(format!("{}: {e}", workspace.display())))?;
        let place = ws.tilde(ws.root());
        let path = ws.root().to_path_buf();
        let mut agent = Agent::new(Setup {
            provider,
            model: model_config.clone(),
            limits: config.limits,
            system: system_prompt(&ws, &today()),
            env: Env {
                permissions: Arc::new(config.permissions.clone()),
                session: Arc::new(SessionRules::default()),
                workspace: Arc::new(ws),
            },
        });
        agent.share_question_ids(self.questions.clone());
        Ok(Built {
            agent,
            place,
            path,
            model: model_ref.to_string(),
        })
    }

    /// Starts the session task. When it ends by itself (a failure), the
    /// session becomes saved again if its files are there.
    fn spawn(self: &Arc<Self>, spec: Spec) -> SessionHandle {
        let weak = Arc::downgrade(self);
        session::spawn(spec, move |id| {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            let mut sessions = lock(&shared.sessions);
            let Some(entry) = sessions.get_mut(id) else {
                return;
            };
            match entry.store.as_ref().map(Store::read_meta) {
                Some(Ok(meta)) => entry.slot = Slot::Saved(Box::new(meta)),
                _ => {
                    sessions.remove(id);
                }
            }
        })
    }

    fn find(&self, name: &str) -> std::result::Result<(u64, Found), ErrorReply> {
        let sessions = lock(&self.sessions);
        let (id, entry) = sessions.by_name(name).ok_or_else(|| no_session(name))?;
        let found = match &entry.slot {
            Slot::Running(handle) => Found::Running(handle.clone()),
            Slot::Saved(_) => Found::Saved,
        };
        Ok((id, found))
    }

    /// The running session `name`, resumed from disk if it is saved;
    /// `model` replaces its model.
    async fn running(
        self: &Arc<Self>,
        name: &str,
        model: Option<&str>,
    ) -> std::result::Result<SessionHandle, ErrorReply> {
        let (id, store, meta) = {
            let sessions = lock(&self.sessions);
            let (id, entry) = sessions.by_name(name).ok_or_else(|| no_session(name))?;
            match (&entry.slot, &entry.store) {
                (Slot::Running(handle), _) => return Ok(handle.clone()),
                (Slot::Saved(meta), Some(store)) => (id, store.clone(), (**meta).clone()),
                (Slot::Saved(_), None) => return Err(no_session(name)),
            }
        };
        let handle = self.resume(id, store, &meta, model)?;
        let placed = {
            let mut sessions = lock(&self.sessions);
            match sessions.get_mut(id) {
                Some(entry) if matches!(entry.slot, Slot::Saved(_)) => {
                    entry.slot = Slot::Running(handle.clone());
                    return Ok(handle);
                }
                Some(entry) => entry.handle().cloned(),
                None => None,
            }
        };
        // Resumed by another request meanwhile, or removed: this copy goes.
        handle.kill("resumed twice", true).await;
        placed.ok_or_else(|| no_session(name))
    }

    /// Loads a saved session and starts it.
    fn resume(
        self: &Arc<Self>,
        id: u64,
        store: Store,
        meta: &Meta,
        model: Option<&str>,
    ) -> std::result::Result<SessionHandle, ErrorReply> {
        let failed = |why: String| {
            ErrorReply::failed(format!(
                "cannot resume {}: {why}; it stays saved",
                meta.name
            ))
        };
        if !meta.workspace.is_dir() {
            return Err(failed(format!(
                "its workspace {} is not a directory",
                meta.workspace.display()
            )));
        }
        let loaded = store.load().map_err(failed)?;
        let built = match self.build(&meta.workspace, Some(model.unwrap_or(&meta.model))) {
            Ok(built) => built,
            Err(e) if model.is_none() => {
                return Err(ErrorReply::usage(format!(
                    "cannot resume {}: {}; choose another model: clai chat {} --model PROVIDER/MODEL",
                    meta.name, e.message, meta.name
                )));
            }
            Err(e) => return Err(e),
        };
        let mut agent = built.agent;
        let problems = agent.restore(
            loaded.history,
            loaded.meta.turns,
            self.load.env.home.as_deref(),
        );
        let mut events = loaded.events;
        for text in problems {
            let notice = SessionEvent::Notice { text };
            if let Err(e) = store.append_event(&notice) {
                tracing::warn!("saving a notice: {e}");
            }
            events.push(notice);
        }
        let mut meta = loaded.meta;
        if built.model != meta.model {
            let changed = SessionEvent::ModelChanged {
                from: std::mem::take(&mut meta.model),
                model: built.model.clone(),
            };
            if let Err(e) = store.append_event(&changed) {
                tracing::warn!("saving the model change: {e}");
            }
            events.push(changed);
        }
        meta.model = built.model;
        meta.place = built.place;
        if let Err(e) = store.write_meta(&meta) {
            tracing::warn!("saving {}: {e}", meta.name);
        }
        tracing::info!(session = %meta.name, model = %meta.model, "session resumed");
        Ok(self.spawn(Spec {
            id,
            meta,
            store: Some(store),
            agent,
            log: events,
        }))
    }

    /// The models of the current configuration, as the requests will use them.
    fn models(&self) -> Reply {
        // Models come only from the user file; any workspace will do.
        let config = self.config(&std::env::temp_dir())?;
        let getenv = |name: &str| std::env::var(name).ok();
        let mut models = Vec::new();
        for provider in config.providers.values() {
            let key = provider.key.status(getenv).name();
            for model in provider.models.values() {
                models.push(ModelInfo {
                    model: format!("{}/{}", provider.name, model.name),
                    provider: provider.name.clone(),
                    name: model.name.clone(),
                    id: model.id.clone(),
                    context_window: model.context_window,
                    default: config.default_model.provider == provider.name
                        && config.default_model.model == model.name,
                    key: key.to_owned(),
                });
            }
        }
        let path = lock(&self.config).path.clone();
        Ok(Payload::Models {
            models,
            config: path,
        })
    }

    async fn list(&self) -> Vec<SessionInfo> {
        let entries: Vec<(u64, Result<SessionHandle, SessionInfo>)> = lock(&self.sessions)
            .iter()
            .map(|(id, e)| {
                let slot = match &e.slot {
                    Slot::Running(handle) => Ok(handle.clone()),
                    Slot::Saved(meta) => Err(meta.info(id, State::Saved, meta.queued.len(), None)),
                };
                (id, slot)
            })
            .collect();
        let mut sessions = Vec::new();
        for (_, slot) in entries {
            match slot {
                Ok(handle) => {
                    if let Some(info) = handle.info().await {
                        sessions.push(info);
                    }
                }
                Err(info) => sessions.push(info),
            }
        }
        sessions
    }
}

impl Conn {
    /// Handles one request. `None` when the session sends the response
    /// itself (`attach`).
    pub(super) async fn request(&mut self, id: u64, request: Request) -> Option<Outcome> {
        let shared = self.shared.clone();
        let reply: Reply = match request {
            Request::Attach { session } => match shared.running(&session, None).await {
                Ok(handle) => match handle.attach(self.outbox.clone(), self.answers, id) {
                    Ok(()) => {
                        self.attached.insert(handle.id);
                        return None;
                    }
                    Err(e) => Err(ErrorReply::failed(e)),
                },
                Err(e) => Err(e),
            },
            Request::New {
                workspace,
                name,
                model,
            } => self.new_session(workspace, name, model).await,
            Request::List => Ok(Payload::Sessions {
                sessions: shared.list().await,
            }),
            Request::Kill { session } => self.kill(&session).await,
            Request::Rename { session, name } => self.rename(&session, name),
            Request::Detach { session } => shared.find(&session).map(|(id, found)| {
                if let Found::Running(handle) = found {
                    handle.detach(self.outbox.id);
                }
                self.attached.remove(&id);
                Payload::Done
            }),
            Request::Send { session, text } => {
                if text.trim().is_empty() {
                    Err(ErrorReply::usage("the message is empty"))
                } else {
                    match shared.running(&session, None).await {
                        Ok(handle) => handle
                            .message(text)
                            .map(|()| Payload::Done)
                            .map_err(ErrorReply::failed),
                        Err(e) => Err(e),
                    }
                }
            }
            Request::SetModel { session, model } => self.set_model(&session, &model).await,
            Request::Cancel { session } => match shared.find(&session) {
                Ok((_, Found::Running(handle))) => handle
                    .cancel()
                    .map(|()| Payload::Done)
                    .map_err(ErrorReply::failed),
                Ok((_, Found::Saved)) => Ok(Payload::Done),
                Err(e) => Err(e),
            },
            Request::Answer {
                session,
                question,
                answer,
            } => match shared.find(&session) {
                Ok((_, Found::Running(handle))) => handle
                    .answer(question, answer, self.outbox.name.clone())
                    .await
                    .map(|()| Payload::Done)
                    .map_err(ErrorReply::failed),
                Ok((_, Found::Saved)) => Err(ErrorReply::failed(format!(
                    "{session} has no open question"
                ))),
                Err(e) => Err(e),
            },
            Request::Export { session, path } => self.export(&session, &path),
            Request::Import { path, name } => self.import(&path, name),
            Request::Status => {
                let (sessions, running) = {
                    let all = lock(&shared.sessions);
                    (all.len(), all.handles().len())
                };
                Ok(Payload::Status(Status {
                    pid: std::process::id(),
                    sessions,
                    running,
                    busy: busy(&shared).await,
                    clients: lock(&shared.clients).len(),
                    version: env!("CARGO_PKG_VERSION").to_owned(),
                }))
            }
            Request::Models => shared.models(),
            Request::Stop => {
                tracing::info!(client = %self.outbox.name, "stop requested");
                shared.stop.cancel();
                Ok(Payload::Done)
            }
        };
        Some(match reply {
            Ok(payload) => Outcome::Ok(payload),
            Err(e) => Outcome::Error(e),
        })
    }

    async fn new_session(
        &mut self,
        workspace: PathBuf,
        name: Option<String>,
        model: Option<String>,
    ) -> Reply {
        let shared = self.shared.clone();
        if !workspace.is_absolute() || !workspace.is_dir() {
            return Err(ErrorReply::usage(format!(
                "{}: not a directory",
                workspace.display()
            )));
        }
        let built = shared.build(&workspace, model.as_deref())?;
        // A session that cannot be saved still runs, in memory.
        let (store, problem) = match Store::create(&shared.sessions_dir) {
            Ok(store) => (Some(store), None),
            Err(e) => (None, Some(e)),
        };
        let id = shared.next_session.fetch_add(1, Ordering::Relaxed) + 1;
        let handle = {
            let mut sessions = lock(&shared.sessions);
            let name = match name {
                Some(name) => {
                    check_name(&name)?;
                    if sessions.is_taken(&name) {
                        if let Some(store) = &store {
                            let _ = store.remove();
                        }
                        return Err(ErrorReply::failed(format!("the name {name} is taken")));
                    }
                    name
                }
                None => sessions.unique_name(&super::default_name(&built.path)),
            };
            let meta = Meta::new(
                name.clone(),
                built.path,
                built.place,
                built.model,
                now_secs(),
            );
            let mut log = Vec::new();
            let mut store = store;
            if let Some(s) = &store
                && let Err(e) = s.write_meta(&meta)
            {
                let _ = s.remove();
                store = None;
                log.push(SessionEvent::Notice {
                    text: format!("cannot save the session: {e}; it lives in memory only"),
                });
            }
            if let Some(e) = problem {
                tracing::error!("cannot save a new session: {e}");
                log.push(SessionEvent::Notice {
                    text: format!("cannot save the session: {e}; it lives in memory only"),
                });
            }
            let handle = shared.spawn(Spec {
                id,
                meta,
                store: store.clone(),
                agent: built.agent,
                log,
            });
            sessions.insert(
                id,
                Entry {
                    name,
                    store,
                    slot: Slot::Running(handle.clone()),
                },
            );
            handle
        };
        let info = handle.info().await.ok_or_else(session::gone)?;
        tracing::info!(session = %info.name, model = %info.model, "session created");
        broadcast(
            &shared,
            Notice::SessionCreated {
                session: Box::new(info.clone()),
            },
        );
        Ok(Payload::Session {
            session: Box::new(info),
        })
    }

    /// Ends the session and deletes its files.
    async fn kill(&mut self, name: &str) -> Reply {
        let shared = self.shared.clone();
        let (id, found) = shared.find(name)?;
        if let Found::Running(handle) = found {
            handle.kill("killed", false).await;
        }
        let entry = lock(&shared.sessions).remove(id);
        if let Some(store) = entry.and_then(|e| e.store) {
            store
                .remove()
                .map_err(|e| ErrorReply::failed(format!("{name} ended, but {e}")))?;
        }
        Ok(Payload::Done)
    }

    fn rename(&self, session: &str, name: String) -> Reply {
        check_name(&name)?;
        let mut sessions = lock(&self.shared.sessions);
        if sessions.is_taken(&name) {
            return Err(ErrorReply::failed(format!("the name {name} is taken")));
        }
        let (_, entry) = sessions
            .by_name_mut(session)
            .ok_or_else(|| no_session(session))?;
        match &mut entry.slot {
            Slot::Running(handle) => handle.rename(name.clone()).map_err(ErrorReply::failed)?,
            Slot::Saved(meta) => {
                meta.name.clone_from(&name);
                if let Some(store) = &entry.store {
                    store.write_meta(meta).map_err(ErrorReply::failed)?;
                }
            }
        }
        entry.name = name;
        Ok(Payload::Done)
    }

    /// Changes the model; a saved session is resumed with the new one,
    /// even if its old model is no longer configured.
    async fn set_model(&self, session: &str, model: &str) -> Reply {
        let shared = self.shared.clone();
        match shared.find(session)? {
            (_, Found::Saved) => {
                shared.running(session, Some(model)).await?;
                Ok(Payload::Done)
            }
            (_, Found::Running(handle)) => {
                // Models come only from the user file; any workspace will do.
                let config = shared.config(&std::env::temp_dir())?;
                let model_ref = config.resolve_model(model).map_err(ErrorReply::usage)?;
                let Some((provider_config, model_config)) = config.model(&model_ref) else {
                    return Err(ErrorReply::usage(format!("unknown model `{model_ref}`")));
                };
                let provider = (shared.providers)(provider_config).map_err(|e| {
                    ErrorReply::failed(format!("provider {}: {e}", provider_config.name))
                })?;
                handle
                    .set_model(model_ref.to_string(), provider, model_config.clone())
                    .map_err(ErrorReply::failed)?;
                Ok(Payload::Done)
            }
        }
    }

    fn export(&self, session: &str, path: &Path) -> Reply {
        if !path.is_absolute() {
            return Err(ErrorReply::usage(format!(
                "{}: not an absolute path",
                path.display()
            )));
        }
        let store = {
            let sessions = lock(&self.shared.sessions);
            let (_, entry) = sessions
                .by_name(session)
                .ok_or_else(|| no_session(session))?;
            entry.store.clone().ok_or_else(|| {
                ErrorReply::failed(format!("{session} is not saved, so it cannot be exported"))
            })?
        };
        let bundle = store.bundle().map_err(ErrorReply::failed)?;
        store::write_bundle_file(path, &bundle).map_err(ErrorReply::failed)?;
        Ok(Payload::Done)
    }

    fn import(&self, path: &Path, name: Option<String>) -> Reply {
        let shared = &self.shared;
        let bundle = store::read_bundle(path).map_err(ErrorReply::failed)?;
        if let Some(name) = &name {
            check_name(name)?;
        }
        let store = Store::create(&shared.sessions_dir).map_err(ErrorReply::failed)?;
        let id = shared.next_session.fetch_add(1, Ordering::Relaxed) + 1;
        let mut sessions = lock(&shared.sessions);
        let name = match name {
            Some(name) if sessions.is_taken(&name) => {
                let _ = store.remove();
                return Err(ErrorReply::failed(format!("the name {name} is taken")));
            }
            Some(name) => name,
            None => sessions.unique_name(&bundle.meta.name),
        };
        let mut meta = bundle.meta.clone();
        meta.name.clone_from(&name);
        meta.format = store::FORMAT;
        meta.updated = now_secs();
        if let Err(e) = store.write_bundle(&bundle, &meta) {
            let _ = store.remove();
            return Err(ErrorReply::failed(e));
        }
        let info = meta.info(id, State::Saved, meta.queued.len(), None);
        sessions.insert(
            id,
            Entry {
                name,
                store: Some(store),
                slot: Slot::Saved(Box::new(meta)),
            },
        );
        Ok(Payload::Session {
            session: Box::new(info),
        })
    }
}
