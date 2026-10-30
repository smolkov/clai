//! HTTP for the adapters: client, timeouts, status codes.
//!
//! `timeout` limits the wait for the response headers and every gap between
//! two chunks of the body; it does not limit the length of an answer.

use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, RETRY_AFTER};

use super::error::{ErrorKind, ProviderError};

pub(crate) struct Http {
    client: reqwest::Client,
    timeout: Duration,
}

/// A non-2xx answer, for the adapter to turn into a `ProviderError`.
#[derive(Debug)]
pub(crate) struct StatusError {
    pub status: u16,
    pub retry_after: Option<Duration>,
    pub body: String,
}

pub(crate) enum PostError {
    Provider(ProviderError),
    Status(StatusError),
}

impl From<ProviderError> for PostError {
    fn from(e: ProviderError) -> Self {
        PostError::Provider(e)
    }
}

/// Error bodies are cut to this length in messages.
const BODY_LIMIT: usize = 64 * 1024;

impl Http {
    pub fn new(timeout: Duration) -> Result<Http, ProviderError> {
        // The system's certificates, plus the bundled Mozilla roots for
        // systems without a CA store.
        let bundled = webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .filter_map(|der| reqwest::Certificate::from_der(der.as_ref()).ok());
        let client = reqwest::Client::builder()
            .connect_timeout(timeout)
            .tls_certs_merge(bundled)
            .user_agent(concat!("clai/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| ProviderError::new(ErrorKind::Network, format!("HTTP client: {e}")))?;
        Ok(Http { client, timeout })
    }

    /// Sends a JSON body; returns once the headers of a 2xx answer arrived.
    pub async fn post(
        &self,
        url: &str,
        headers: &[(&'static str, String)],
        body: Vec<u8>,
    ) -> Result<reqwest::Response, PostError> {
        let mut map = HeaderMap::new();
        map.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        for (name, value) in headers {
            let invalid =
                || ProviderError::new(ErrorKind::InvalidRequest, format!("invalid header {name}"));
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid())?;
            let value = HeaderValue::from_str(value).map_err(|_| invalid())?;
            map.insert(name, value);
        }
        let send = self.client.post(url).headers(map).body(body).send();
        let response = tokio::time::timeout(self.timeout, send)
            .await
            .map_err(|_| self.timed_out("waiting for the response"))?
            .map_err(|e| network(&e))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let body = self.error_body(response).await;
        Err(PostError::Status(StatusError {
            status: status.as_u16(),
            retry_after,
            body,
        }))
    }

    /// Next chunk of the body; `None` at the end.
    pub async fn chunk(
        &self,
        response: &mut reqwest::Response,
    ) -> Result<Option<Vec<u8>>, ProviderError> {
        tokio::time::timeout(self.timeout, response.chunk())
            .await
            .map_err(|_| self.timed_out("waiting for the next part of the answer"))?
            .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
            .map_err(|e| network(&e))
    }

    async fn error_body(&self, mut response: reqwest::Response) -> String {
        let mut body = Vec::new();
        while body.len() < BODY_LIMIT {
            match self.chunk(&mut response).await {
                Ok(Some(chunk)) => body.extend_from_slice(&chunk),
                _ => break,
            }
        }
        body.truncate(BODY_LIMIT);
        String::from_utf8_lossy(&body).into_owned()
    }

    fn timed_out(&self, what: &str) -> ProviderError {
        ProviderError::new(
            ErrorKind::Timeout,
            format!("no answer within {} s ({what})", self.timeout.as_secs_f32()),
        )
    }
}

fn network(e: &reqwest::Error) -> ProviderError {
    // reqwest errors name the URL but never the headers, so no key leaks.
    let mut message = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    let kind = if e.is_timeout() {
        ErrorKind::Timeout
    } else {
        ErrorKind::Network
    };
    ProviderError::new(kind, message)
}

/// Error kind for an HTTP status, before the adapter looks at the body.
pub(crate) fn status_kind(status: u16) -> ErrorKind {
    match status {
        401 | 403 => ErrorKind::Auth,
        408 => ErrorKind::Timeout,
        413 => ErrorKind::ContextTooLong,
        429 => ErrorKind::RateLimit,
        500..=599 => ErrorKind::Overloaded,
        _ => ErrorKind::InvalidRequest,
    }
}

/// `HTTP 429: <detail>`, with the detail cut to one readable line.
pub(crate) fn status_message(status: u16, detail: &str) -> String {
    let detail: String = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    let detail: String = detail.chars().take(300).collect();
    if detail.is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status}: {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(status_kind(401), ErrorKind::Auth);
        assert_eq!(status_kind(429), ErrorKind::RateLimit);
        assert_eq!(status_kind(529), ErrorKind::Overloaded);
        assert_eq!(status_kind(503), ErrorKind::Overloaded);
        assert_eq!(status_kind(413), ErrorKind::ContextTooLong);
        assert_eq!(status_kind(404), ErrorKind::InvalidRequest);
    }

    #[test]
    fn messages() {
        assert_eq!(status_message(500, ""), "HTTP 500");
        assert_eq!(
            status_message(400, " bad\n request "),
            "HTTP 400: bad request"
        );
        assert_eq!(
            status_message(400, &"x".repeat(500)).len(),
            "HTTP 400: ".len() + 300
        );
    }
}
