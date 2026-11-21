//! HTTP and WebSocket access to an arena server.

use anyhow::{Context, Result, bail};
use arena_protocol::{ApiError, ServerEvent, ServerInfo};
use futures_util::StreamExt;
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
}

/// A request that failed, kept typed so the CLI can pick a hint and an exit code.
#[derive(Debug)]
pub enum Failure {
    Unreachable {
        server: String,
        cause: String,
    },
    Rejected {
        status: StatusCode,
        message: String,
        path: String,
        server: String,
    },
    BadResponse {
        server: String,
    },
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Unreachable { server, cause } => {
                write!(f, "cannot reach the server at {server}: {cause}")
            }
            Failure::Rejected { message, .. } => f.write_str(message),
            Failure::BadResponse { server } => {
                write!(
                    f,
                    "unexpected response from {server}; is it an arena server?"
                )
            }
        }
    }
}

impl std::error::Error for Failure {}

/// The innermost cause, without reqwest's wrapping or the OS error number.
fn root_cause(e: &(dyn std::error::Error + 'static)) -> String {
    let mut cause = e;
    while let Some(next) = cause.source() {
        cause = next;
    }
    let text = cause.to_string();
    let text = text.split(" (os error").next().unwrap_or(&text);
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The server's `{"error": ...}` message. Other bodies (a proxy's HTML page, say)
/// are not shown: a sentence for the status says more.
fn rejection_message(status: StatusCode, body: &str) -> String {
    if let Ok(e) = serde_json::from_str::<ApiError>(body) {
        return e.error;
    }
    match status {
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => {
            "this server doesn't support that request".to_owned()
        }
        StatusCode::PAYLOAD_TOO_LARGE => "the upload is too large for this server".to_owned(),
        s if s.is_server_error() => format!("the server failed ({s})"),
        s => format!("the server refused the request ({s})"),
    }
}

/// A warning when versions differ in major or minor, which can change the API.
pub fn version_mismatch(cli: &str, server: &str) -> Option<String> {
    let series = |v: &str| v.split('.').take(2).map(str::to_owned).collect::<Vec<_>>();
    (series(cli) != series(server)).then(|| {
        format!(
            "warning: this CLI is version {cli} but the server is {server}; some commands may fail"
        )
    })
}

/// Uses the `ring` crypto provider for every TLS connection (no C toolchain needed).
pub fn init_tls() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// `arena.local:8080` → `http://arena.local:8080`, without a trailing slash.
pub fn normalise_server(server: &str) -> String {
    let s = server.trim().trim_end_matches('/');
    if s.is_empty() || s.starts_with("http://") || s.starts_with("https://") {
        s.to_owned()
    } else {
        format!("http://{s}")
    }
}

impl Client {
    pub fn new(server: &str, token: Option<String>) -> Result<Client> {
        init_tls();
        let base = normalise_server(server);
        if base.is_empty() {
            bail!("no server address given");
        }
        let http = reqwest::Client::builder()
            .user_agent(concat!("arena-cli/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Client { http, base, token })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        let req = self.http.request(method, format!("{}{path}", self.base));
        match &self.token {
            Some(t) => req.bearer_auth(t),
            None => req,
        }
    }

    async fn send(&self, req: RequestBuilder) -> Result<Response> {
        let resp = req.send().await.map_err(|e| self.unreachable(&e))?;
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status();
        let path = resp.url().path().to_owned();
        let text = resp.text().await.unwrap_or_default();
        Err(Failure::Rejected {
            status,
            message: rejection_message(status, &text),
            path,
            server: self.base.clone(),
        }
        .into())
    }

    fn unreachable(&self, e: &reqwest::Error) -> anyhow::Error {
        Failure::Unreachable {
            server: self.base.clone(),
            cause: root_cause(e),
        }
        .into()
    }

    async fn parse<T: DeserializeOwned>(&self, resp: Response) -> Result<T> {
        let bytes = resp.bytes().await.map_err(|e| self.unreachable(&e))?;
        serde_json::from_slice(&bytes).map_err(|_| {
            Failure::BadResponse {
                server: self.base.clone(),
            }
            .into()
        })
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self.send(self.request(Method::GET, path)).await?;
        self.parse(resp).await
    }

    pub async fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let resp = self.send(self.request(Method::GET, path)).await?;
        let bytes = resp.bytes().await.map_err(|e| self.unreachable(&e))?;
        Ok(bytes.to_vec())
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let resp = self
            .send(self.request(Method::POST, path).json(body))
            .await?;
        self.parse(resp).await
    }

    /// A POST whose answer has no body.
    pub async fn post_empty(&self, path: &str) -> Result<()> {
        self.send(self.request(Method::POST, path)).await?;
        Ok(())
    }

    pub async fn put_bytes<T: DeserializeOwned>(&self, path: &str, body: Vec<u8>) -> Result<T> {
        let req = self
            .request(Method::PUT, path)
            .header("content-type", "application/gzip")
            .body(body);
        let resp = self.send(req).await?;
        self.parse(resp).await
    }

    pub async fn post_multipart<T: DeserializeOwned>(
        &self,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> Result<T> {
        let resp = self
            .send(self.request(Method::POST, path).multipart(form))
            .await?;
        self.parse(resp).await
    }

    /// Warns on stderr if the server's version differs. Never fails or hangs.
    pub async fn warn_if_version_differs(&self) {
        let info = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            self.get::<ServerInfo>("/api/info"),
        )
        .await;
        if let Ok(Ok(info)) = info
            && let Some(warning) = version_mismatch(env!("CARGO_PKG_VERSION"), &info.version)
        {
            eprintln!("{warning}");
        }
    }

    /// Opens `/ws` and returns a stream of events.
    pub async fn events(&self) -> Result<EventStream> {
        let url = if let Some(rest) = self.base.strip_prefix("https://") {
            format!("wss://{rest}/ws")
        } else {
            format!("ws://{}/ws", self.base.trim_start_matches("http://"))
        };
        let mut req = url.as_str().into_client_request()?;
        if let Some(t) = &self.token {
            req.headers_mut()
                .insert("authorization", format!("Bearer {t}").parse()?);
        }
        let (ws, _) = tokio_tungstenite::connect_async(req)
            .await
            .context("cannot open the live connection")?;
        Ok(EventStream { ws })
    }
}

pub struct EventStream {
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
}

impl EventStream {
    /// The next event, or `None` when the server closed the connection.
    pub async fn next(&mut self) -> Result<Option<ServerEvent>> {
        while let Some(msg) = self.ws.next().await {
            match msg? {
                Message::Text(text) => return Ok(Some(serde_json::from_str(&text)?)),
                Message::Close(_) => return Ok(None),
                _ => {}
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_addresses_need_no_scheme() {
        assert_eq!(
            normalise_server("arena.local:8080/"),
            "http://arena.local:8080"
        );
        assert_eq!(normalise_server("https://a.example"), "https://a.example");
        assert_eq!(normalise_server(" "), "");
    }

    #[test]
    fn only_json_error_bodies_are_shown() {
        let status = StatusCode::NOT_FOUND;
        assert_eq!(
            rejection_message(status, r#"{"error":"no round 9"}"#),
            "no round 9"
        );
        let html = rejection_message(StatusCode::BAD_GATEWAY, "<html>nginx</html>");
        assert!(!html.contains("nginx"), "{html}");
        assert!(rejection_message(status, "").contains("doesn't support"));
    }

    #[test]
    fn patch_versions_may_differ() {
        assert!(version_mismatch("0.4.0", "0.4.7").is_none());
        assert!(version_mismatch("0.4.0", "0.5.0").is_some());
        assert!(version_mismatch("1.4.0", "2.4.0").is_some());
    }
}
