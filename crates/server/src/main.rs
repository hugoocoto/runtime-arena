use std::net::SocketAddr;

use arena_protocol::ServerInfo;
use axum::{Json, Router, routing::get};

const SOURCE_URL: &str = env!("CARGO_PKG_REPOSITORY");

fn app() -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/info", get(info))
}

async fn info() -> Json<ServerInfo> {
    Json(ServerInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        source_url: SOURCE_URL.to_owned(),
    })
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let addr: SocketAddr = std::env::var("ARENA_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_owned())
        .parse()
        .expect("ARENA_LISTEN must be a socket address");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("cannot bind listen address");
    tracing::info!(%addr, "arena-server listening");
    axum::serve(listener, app())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("server error");
}
