//! A provider that replays a script, for tests of the agent loop.

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};

use futures_util::stream::{self, StreamExt};
use tokio_util::sync::CancellationToken;

use super::error::ProviderError;
use super::remote::error_stream;
use super::types::{Event, Request};
use super::{EventStream, Provider};

/// The answer to one request: events and errors, in order.
pub type Answer = Vec<Result<Event, ProviderError>>;

#[derive(Debug, Default)]
pub struct Fake {
    answers: Mutex<VecDeque<Answer>>,
    requests: Mutex<Vec<Request>>,
}

impl Fake {
    /// One answer per request, used in order.
    pub fn new(answers: Vec<Answer>) -> Fake {
        Fake {
            answers: Mutex::new(answers.into()),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// The requests received so far.
    pub fn requests(&self) -> Vec<Request> {
        lock(&self.requests).clone()
    }
}

/// A poisoned lock only means another test thread panicked; the data is fine.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Provider for Fake {
    fn stream(&self, request: Request, cancel: CancellationToken) -> EventStream {
        lock(&self.requests).push(request);
        if cancel.is_cancelled() {
            return error_stream(ProviderError::cancelled());
        }
        match lock(&self.answers).pop_front() {
            Some(answer) => stream::iter(answer).boxed(),
            None => error_stream(ProviderError::protocol("fake provider: no more answers")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::types::{Message, StopReason};

    fn request(text: &str) -> Request {
        Request {
            model: "m".into(),
            system: String::new(),
            messages: vec![Message::user(text)],
            tools: Vec::new(),
            max_output_tokens: None,
            temperature: None,
        }
    }

    #[tokio::test]
    async fn replays_and_records() {
        let done = Event::Done {
            stop_reason: StopReason::EndTurn,
        };
        let fake = Fake::new(vec![vec![
            Ok(Event::TextDelta { text: "hi".into() }),
            Ok(done.clone()),
        ]]);
        let events: Vec<_> = fake
            .stream(request("one"), CancellationToken::new())
            .collect()
            .await;
        assert_eq!(
            events,
            [Ok(Event::TextDelta { text: "hi".into() }), Ok(done)]
        );

        let second: Vec<_> = fake
            .stream(request("two"), CancellationToken::new())
            .collect()
            .await;
        assert!(matches!(second.as_slice(), [Err(e)] if e.message.contains("no more answers")));
        assert_eq!(fake.requests(), [request("one"), request("two")]);
    }
}
