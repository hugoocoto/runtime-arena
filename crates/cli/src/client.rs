//! HTTP and WebSocket access to an arena server.

use anyhow::{Context, Result, anyhow, bail};
use arena_protocol::{ApiError, ServerEvent};
use futures_util::StreamExt;
use reqwest::{Method, RequestBuilder, Response};
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

/// Uses the `ring` crypto provider for every TLS connection (no C toolchain needed).
pub fn init_tls() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

impl Client {
    pub fn new(server: &str, token: Option<String>) -> Result<Client> {
        init_tls();
        let base = server.trim_end_matches('/').to_owned();
        if !(base.starts_with("http://") || base.starts_with("https://")) {
            bail!("server must start with http:// or https://, got {server}");
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

    async fn send(req: RequestBuilder) -> Result<Response> {
        let resp = req.send().await.context("cannot reach the server")?;
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let message = serde_json::from_str::<ApiError>(&text)
            .map(|e| e.error)
            .unwrap_or(text);
        Err(anyhow!("{message} ({status})"))
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        Ok(Self::send(self.request(Method::GET, path))
            .await?
            .json()
            .await?)
    }

    pub async fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        Ok(Self::send(self.request(Method::GET, path))
            .await?
            .bytes()
            .await?
            .to_vec())
    }

    pub async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        Ok(Self::send(self.request(Method::POST, path).json(body))
            .await?
            .json()
            .await?)
    }

    pub async fn put_bytes<T: DeserializeOwned>(&self, path: &str, body: Vec<u8>) -> Result<T> {
        let req = self
            .request(Method::PUT, path)
            .header("content-type", "application/gzip")
            .body(body);
        Ok(Self::send(req).await?.json().await?)
    }

    pub async fn post_multipart<T: DeserializeOwned>(
        &self,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> Result<T> {
        Ok(Self::send(self.request(Method::POST, path).multipart(form))
            .await?
            .json()
            .await?)
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
