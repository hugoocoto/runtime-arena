//! Runtime Arena server: HTTP API, judging queue and live updates.

mod accounts;
pub mod auth;
pub mod backend;
mod boards;
pub mod config;
pub mod error;
mod login_limit;
pub mod net_limit;
mod problems;
mod queue;
mod rounds;
mod submissions;
mod updates;
mod users;
mod ws;

use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use arena_protocol::{ServerEvent, ServerInfo};
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use tokio::sync::{Mutex, Notify, Semaphore, broadcast};

pub use backend::{IsolateBackend, JudgeBackend};
pub use config::Config;

/// Largest problem archive accepted.
const MAX_PROBLEM_UPLOAD: usize = 256 * 1024 * 1024;
/// Largest submission request (executable plus source archive).
const MAX_SUBMISSION_UPLOAD: usize = 96 * 1024 * 1024;

/// An event for WebSocket clients: for everyone, or only for one participant.
#[derive(Debug, Clone)]
pub enum Event {
    Public(ServerEvent),
    ForUser(i64, ServerEvent),
}

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Arc<Config>,
    pub judge: Arc<dyn JudgeBackend>,
    pub events: broadcast::Sender<Event>,
    /// Wakes a worker when a submission is queued.
    pub queue: Arc<Notify>,
    /// Serialises submission intake so the cooldown can't be raced.
    submit_lock: Arc<Mutex<()>>,
    /// Serialises problem uploads (version numbers, lock check).
    upload_lock: Arc<Mutex<()>>,
    /// Latest published release, once an update check has succeeded.
    latest_version: Arc<RwLock<Option<String>>>,
    /// Failed password attempts.
    login_limit: Arc<login_limit::LoginLimiter>,
    /// Requests per second from each address.
    request_limit: Arc<net_limit::RequestLimiter>,
    /// Password hashes running at once.
    hash_slots: Arc<Semaphore>,
}

impl AppState {
    pub fn new(db: SqlitePool, config: Config, judge: Arc<dyn JudgeBackend>) -> Self {
        let request_limit = Arc::new(net_limit::RequestLimiter::new(config.rate_limit));
        AppState {
            db,
            config: Arc::new(config),
            judge,
            events: broadcast::channel(1024).0,
            queue: Arc::new(Notify::new()),
            submit_lock: Arc::new(Mutex::new(())),
            upload_lock: Arc::new(Mutex::new(())),
            latest_version: Arc::new(RwLock::new(None)),
            login_limit: Arc::default(),
            request_limit,
            hash_slots: Arc::new(Semaphore::new(accounts::MAX_PASSWORD_HASHES)),
        }
    }

    fn publish(&self, event: Event) {
        // No receivers is fine: nobody is connected.
        let _ = self.events.send(event);
    }
}

/// Opens (creating if needed) the database in `data_dir` and applies migrations.
pub async fn open_db(data_dir: &Path) -> Result<SqlitePool, sqlx::Error> {
    std::fs::create_dir_all(data_dir)?;
    let options = SqliteConnectOptions::new()
        .filename(data_dir.join("arena.db"))
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let db = SqlitePoolOptions::new().connect_with(options).await?;
    sqlx::migrate!().run(&db).await?;
    Ok(db)
}

/// Starts the judge workers and the round clock. Call once per process.
pub async fn start_background(state: &AppState) -> Result<(), sqlx::Error> {
    queue::requeue_interrupted(state).await?;
    for &box_id in &state.config.judge_boxes {
        tokio::spawn(queue::worker(state.clone(), box_id));
    }
    tokio::spawn(rounds::clock(state.clone()));
    if state.config.update_check {
        tokio::spawn(updates::clock(state.clone()));
    }
    Ok(())
}

pub fn app(state: AppState) -> Router {
    let limiter = state.clone();
    let admin = Router::new()
        .route("/update", get(updates::status))
        .route("/users", get(users::list))
        .route("/users/{name}/password", post(accounts::reset_password))
        .route("/problems", get(problems::list))
        .route(
            "/problems/{slug}",
            put(problems::upload).layer(DefaultBodyLimit::max(MAX_PROBLEM_UPLOAD)),
        )
        .route("/rounds", get(rounds::list).post(rounds::create))
        .route("/rounds/{id}/start", post(rounds::start))
        .route("/rounds/{id}/stop", post(rounds::stop))
        .route("/rounds/{id}/extend", post(rounds::extend))
        .route("/submissions", get(submissions::admin_list))
        .route("/submissions/{id}/source", get(submissions::source))
        .route("/submissions/{id}/rejudge", post(submissions::rejudge));

    Router::new()
        .route("/", get(projector))
        .route("/join", get(accounts::page))
        .route("/healthz", get(|| async { "ok" }))
        .route("/ws", get(ws::handler))
        .route("/api/info", get(info))
        .route("/api/me", get(users::me))
        .route("/api/signup", post(accounts::signup))
        .route("/api/login", post(accounts::login))
        .route("/api/logout", post(accounts::logout))
        .route("/api/me/submissions", get(submissions::mine))
        .route("/api/rounds/current", get(rounds::current))
        .route("/api/rounds/{id}/boards", get(boards::round_boards))
        .route(
            "/api/rounds/{id}/problems/{slug}/accepted",
            get(boards::accepted_history),
        )
        .route("/api/problems/{slug}", get(problems::statement))
        .route(
            "/api/submissions",
            post(submissions::create).layer(DefaultBodyLimit::max(MAX_SUBMISSION_UPLOAD)),
        )
        .route("/api/submissions/{id}", get(submissions::get))
        .nest("/api/admin", admin)
        .with_state(state)
        .layer(axum::middleware::from_fn_with_state(
            limiter,
            net_limit::limit_requests,
        ))
}

/// The read-only leaderboard page for a projector, built into the binary.
async fn projector() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../../../web/index.html"))
}

async fn info() -> Json<ServerInfo> {
    Json(ServerInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        source_url: env!("CARGO_PKG_REPOSITORY").to_owned(),
    })
}

/// Current Unix time in seconds.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Names and slugs: 1-40 characters of `a-z`, `0-9` and `-`.
fn valid_slug(s: &str) -> bool {
    (1..=40).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
