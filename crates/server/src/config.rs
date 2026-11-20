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
    /// One judge worker per isolate box. Box `i` is pinned to the `i`-th CPU of
    /// `JUDGE_CPUS` by the container entrypoint.
    pub judge_boxes: Vec<u32>,
    /// Path of the isolate binary.
    pub isolate: PathBuf,
    /// Times each accepted test runs; the best time and memory count.
    pub runs_per_test: u32,
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
        Ok(Config {
            listen,
            data_dir,
            admin_token,
            judge_boxes: (0..workers as u32).collect(),
            isolate: var("ARENA_ISOLATE")
                .unwrap_or_else(|| "isolate".into())
                .into(),
            runs_per_test,
        })
    }
}
