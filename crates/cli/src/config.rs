//! `~/.config/arena/config.toml`: server URL and tokens. Environment variables
//! `ARENA_SERVER` overrides the server, and `ARENA_TOKEN` and `ARENA_ADMIN_TOKEN`
//! apply only together with it, so a token never reaches a server it wasn't
//! meant for. `arena login --admin` also reads `./.env` (see [`dotenv`]).

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::client::normalise_server;
use crate::errors::NotLoggedIn;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub server: Option<String>,
    pub token: Option<String>,
    pub admin_token: Option<String>,
    /// PEM-encoded CA trusted for the remembered server, used for LAN HTTPS.
    #[serde(default)]
    pub ca_certificate: Option<String>,
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
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Ok(Self::load_file()?.with_env(env))
    }

    /// Applies environment overrides. Saved tokens are dropped when the server
    /// differs, and environment tokens need `ARENA_SERVER` beside them.
    fn with_env(mut self, env: impl Fn(&str) -> Option<String>) -> Config {
        if let Some(server) = env("ARENA_SERVER") {
            self.set_server(&server);
            self.token = env("ARENA_TOKEN").or(self.token);
            self.admin_token = env("ARENA_ADMIN_TOKEN").or(self.admin_token);
        }
        self
    }

    /// Changes the server, forgetting tokens that belong to a different one.
    pub fn set_server(&mut self, server: &str) {
        let server = normalise_server(server);
        if self.server.as_deref().map(normalise_server).as_deref() != Some(server.as_str()) {
            self.token = None;
            self.admin_token = None;
            self.ca_certificate = None;
        }
        self.server = Some(server);
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
        self.server.as_deref().ok_or_else(|| {
            NotLoggedIn(
                "no server configured; run `arena login <server>`, \
                     or `arena signup <server>` for a new account"
                    .into(),
            )
            .into()
        })
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

/// Warns when `./.env`, which holds the admin token, is readable by other users.
pub fn warn_if_dotenv_shared() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(".env").is_ok_and(|m| m.permissions().mode() & 0o077 != 0) {
            eprintln!("warning: .env is readable by other users; run `chmod 600 .env`");
        }
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
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // `mode` only applies to new files; tighten an existing one before writing.
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(text.as_bytes())
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Config {
        Config {
            server: Some("http://a:8080".into()),
            token: Some("user".into()),
            admin_token: Some("admin".into()),
            ca_certificate: None,
        }
    }

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        |k| {
            pairs
                .iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn changing_server_forgets_tokens() {
        let mut c = saved();
        c.set_server("a:8080/");
        assert_eq!(c, saved());
        c.set_server("http://b:8080");
        assert_eq!(c.server.as_deref(), Some("http://b:8080"));
        assert_eq!((c.token, c.admin_token), (None, None));
    }

    #[test]
    fn saved_tokens_do_not_follow_arena_server() {
        let c = saved().with_env(vars(&[("ARENA_SERVER", "http://b:8080")]));
        assert_eq!((c.token, c.admin_token), (None, None));
    }

    #[test]
    fn environment_tokens_need_arena_server() {
        let c = saved().with_env(vars(&[("ARENA_ADMIN_TOKEN", "other")]));
        assert_eq!(c, saved());
        let c = saved().with_env(vars(&[
            ("ARENA_SERVER", "http://b:8080"),
            ("ARENA_ADMIN_TOKEN", "other"),
        ]));
        assert_eq!(c.admin_token.as_deref(), Some("other"));
    }

    #[cfg(unix)]
    #[test]
    fn saving_tightens_an_existing_file() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        std::fs::write(&p, "").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&p, "x").unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
