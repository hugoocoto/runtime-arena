use std::sync::Arc;

use arena_judge::JudgeConfig;
use arena_server::net_limit::{LimitedListener, Peer};
use arena_server::{AppState, Config, IsolateBackend, app, open_db, start_background};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("arena-server: {e}");
            std::process::exit(2);
        }
    };
    let db = open_db(&config.data_dir)
        .await
        .unwrap_or_else(|e| panic!("cannot open database in {}: {e}", config.data_dir.display()));
    let listener = tokio::net::TcpListener::bind(config.listen)
        .await
        .expect("cannot bind listen address");
    tracing::info!(addr = %config.listen, workers = config.judge_boxes.len(), "arena-server listening");

    let judge = Arc::new(IsolateBackend {
        config: JudgeConfig {
            isolate: config.isolate.clone(),
            runs_per_test: config.runs_per_test,
            ..JudgeConfig::default()
        },
    });
    let state = AppState::new(db, config, judge);
    if let Err(e) = start_background(&state).await {
        panic!("cannot start judge workers: {e}");
    }
    // The peer address is what the rate limits count by.
    axum::serve(
        LimitedListener::new(listener),
        app(state).into_make_service_with_connect_info::<Peer>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .expect("server error");
}

/// Ctrl-C, or SIGTERM from `docker compose stop` (the server is PID 1 in its container).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    tokio::select! {
        () = ctrl_c => {},
        () = term => {},
    }
    tracing::info!("shutting down");
}
