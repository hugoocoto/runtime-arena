//! `~/.config/arena/config.toml`: server URL and tokens. Environment variables
//! `ARENA_SERVER`, `ARENA_TOKEN` and `ARENA_ADMIN_TOKEN` override it.
//! `arena login --admin` also reads the host's `./.env` (see [`dotenv`]).

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub server: Option<String>,
    pub token: Option<String>,
    pub admin_token: Option<String>,
}

pub fn path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("ARENA_CONFIG") {
        return Ok(p.into());
    }
    let dir = dirs::config_dir().ok_or_else(|| anyhow!("no config directory on this system"))?;
    Ok(dir.join("arena").join("config.toml"))
}

impl Config {
    /// The saved file, with environment overrides applied.
    pub fn load() -> Result<Config> {
        let mut c = Self::load_file()?;
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        c.server = env("ARENA_SERVER").or(c.server);
        c.token = env("ARENA_TOKEN").or(c.token);
        c.admin_token = env("ARENA_ADMIN_TOKEN").or(c.admin_token);
        Ok(c)
    }

    /// Only what is saved on disk (what `save` would rewrite).
    pub fn load_file() -> Result<Config> {
        let path = path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("reading {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Writes the file readable only by the user, since it holds tokens.
    pub fn save(&self) -> Result<PathBuf> {
        let path = path()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string(self)?;
        write_private(&path, &text).with_context(|| format!("writing {}", path.display()))?;
        Ok(path)
    }

    pub fn server(&self) -> Result<&str> {
        self.server
            .as_deref()
            .ok_or_else(|| anyhow!("no server configured; run `arena login <server> <token>`"))
    }
}

/// The non-empty values in `./.env`; empty if there is no such file.
pub fn dotenv() -> Result<HashMap<String, String>> {
    match std::fs::read_to_string(".env") {
        Ok(text) => Ok(parse_dotenv(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(e) => Err(e).context("reading ./.env"),
    }
}

/// Parses `KEY=value` lines, with optional `export`, quotes and `#` comments.
pub fn parse_dotenv(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with('#') {
                return None;
            }
            let (key, value) = line
                .strip_prefix("export ")
                .unwrap_or(line)
                .split_once('=')?;
            let value = value.trim();
            let quoted = ['"', '\'']
                .into_iter()
                .find_map(|q| value.strip_prefix(q)?.split_once(q).map(|(inner, _)| inner));
            let value = quoted.unwrap_or_else(|| {
                value
                    .split_once(" #")
                    .map_or(value, |(before, _)| before)
                    .trim_end()
            });
            (!value.is_empty()).then(|| (key.trim().to_owned(), value.to_owned()))
        })
        .collect()
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(text.as_bytes())
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)
}
