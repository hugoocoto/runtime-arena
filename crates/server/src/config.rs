//! Server settings, read from the environment.

use std::net::SocketAddr;
use std::path::PathBuf;

/// Shortest admin token accepted, so a guessable one is refused at startup.
const MIN_ADMIN_TOKEN_LEN: usize = 24;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    /// Database, problems and submissions live here.
    pub data_dir: PathBuf,
    pub admin_token: String,
    /// Event code that lets people register themselves on `/join`. `None` disables
    /// self-registration, so nobody new can join.
    pub join_code: Option<String>,
    /// One judge worker per isolate box. Box `i` is pinned to the `i`-th CPU of
    /// `JUDGE_CPUS` by the container entrypoint.
    pub judge_boxes: Vec<u32>,
    /// Path of the isolate binary.
    pub isolate: PathBuf,
    /// Times each accepted test runs; the best time and memory count.
    pub runs_per_test: u32,
    /// Processes and threads one submission may create (`ARENA_PROCESSES`).
    pub processes: u32,
    /// Stop judging a submission after this many failing tests; 0 runs all
    /// (`ARENA_MAX_FAILURES`).
    pub max_failures: u32,
    /// Isolate's cgroup root, read for microsecond CPU time (`ARENA_CG_ROOT`).
    pub cg_root: PathBuf,
    /// Root of the size-limited per-box directories (`ARENA_BOX_STORAGE`).
    pub box_storage: Option<PathBuf>,
    /// Periodically ask GitHub for the latest release (`ARENA_UPDATE_CHECK=off` disables).
    pub update_check: bool,
    /// Requests per second allowed from one address (`ARENA_RATE_LIMIT`; 0 disables).
    pub rate_limit: u32,
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());

        let listen = var("ARENA_LISTEN")
            .unwrap_or_else(|| "0.0.0.0:8080".into())
            .parse()
            .map_err(|e| format!("ARENA_LISTEN: {e}"))?;
        let data_dir = var("ARENA_DATA").unwrap_or_else(|| "data".into()).into();
        let admin_token = var("ARENA_ADMIN_TOKEN")
            .ok_or("ARENA_ADMIN_TOKEN is not set; generate one with `openssl rand -hex 32`")?;
        if admin_token.len() < MIN_ADMIN_TOKEN_LEN {
            return Err(format!(
                "ARENA_ADMIN_TOKEN must be at least {MIN_ADMIN_TOKEN_LEN} characters"
            ));
        }
        let join_code = var("ARENA_JOIN_CODE");
        if join_code.as_ref().is_some_and(|c| c.len() < 6) {
            return Err("ARENA_JOIN_CODE must be at least 6 characters".into());
        }
        // The same variable the entrypoint uses to pin boxes; empty means one unpinned worker.
        let workers = var("JUDGE_CPUS")
            .map(|cpus| cpus.split(',').filter(|c| !c.trim().is_empty()).count())
            .unwrap_or(0)
            .max(1);
        let runs_per_test = match var("ARENA_RUNS_PER_TEST") {
            Some(v) => v
                .parse()
                .ok()
                .filter(|n| *n > 0)
                .ok_or("ARENA_RUNS_PER_TEST must be a positive integer")?,
            None => 3,
        };
        let processes = match var("ARENA_PROCESSES") {
            Some(v) => v
                .parse()
                .ok()
                .filter(|n| (1..=4096).contains(n))
                .ok_or("ARENA_PROCESSES must be between 1 and 4096")?,
            None => 64,
        };
        let max_failures = match var("ARENA_MAX_FAILURES") {
            Some(v) => v
                .parse()
                .map_err(|_| "ARENA_MAX_FAILURES must be a whole number (0 runs every test)")?,
            None => 3,
        };
        let cg_root = var("ARENA_CG_ROOT")
            .unwrap_or_else(|| "/sys/fs/cgroup/isolate".into())
            .into();
        let box_storage = Some(
            var("ARENA_BOX_STORAGE")
                .unwrap_or_else(|| "/var/local/lib/arena-boxes".into())
                .into(),
        );
        let update_check = match var("ARENA_UPDATE_CHECK").as_deref() {
            None | Some("on" | "true" | "1") => true,
            Some("off" | "false" | "0") => false,
            Some(_) => return Err("ARENA_UPDATE_CHECK must be on or off".into()),
        };
        let rate_limit = match var("ARENA_RATE_LIMIT") {
            Some(v) => v
                .parse()
                .map_err(|_| "ARENA_RATE_LIMIT must be a whole number (0 disables it)")?,
            None => crate::net_limit::DEFAULT_RATE_LIMIT,
        };
        Ok(Config {
            listen,
            data_dir,
            admin_token,
            join_code,
            judge_boxes: (0..workers as u32).collect(),
            isolate: var("ARENA_ISOLATE")
                .unwrap_or_else(|| "isolate".into())
                .into(),
            runs_per_test,
            processes,
            max_failures,
            cg_root,
            box_storage,
            update_check,
            rate_limit,
        })
    }
}
