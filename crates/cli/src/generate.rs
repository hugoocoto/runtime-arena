//! Shell completions and man pages, generated from the command line definition.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use clap::CommandFactory;
use clap_complete::{Shell, generate, generate_to};

use crate::cli::Cli;

/// Added to `arena(1)`, for what clap doesn't know about.
const MAN_EXTRA: &str = r#".SH "EXIT STATUS"
.TP
.B 0
Success.
.TP
.B 1
The command ran but the result was negative, e.g. a submission was not accepted.
.TP
.B 2
Invalid or missing arguments.
.TP
.B 3
The server could not be reached.
.TP
.B 4
Not logged in, or the server refused the credentials (HTTP 401 or 403).
.TP
.B 5
The server rejected the request (any other HTTP 4XX).
.TP
.B 6
The server failed (HTTP 5XX) or sent an unreadable response.
.TP
.B 7
Any other error, such as an unreadable local file.
.SH ENVIRONMENT
.TP
.B ARENA_SERVER
Server to use instead of the saved one. Saved tokens are not sent to a different server.
.TP
.B ARENA_TOKEN
Session token to use; applies only together with
.BR ARENA_SERVER .
.TP
.B ARENA_ADMIN_TOKEN
Admin token; applies only together with
.BR ARENA_SERVER ,
or read by
.B arena login --admin
.TP
.B ARENA_JOIN_CODE
Event code for
.BR "arena signup" .
.TP
.B ARENA_PORT
Port of the local server for
.BR "arena login --admin" .
.TP
.B ARENA_CONFIG
Path of the configuration file.
.SH FILES
.TP
.I ~/.config/arena/config.toml
Server address and session tokens, readable only by you. Passwords are never saved.
.TP
.I ./.env
Read by
.B arena login --admin
for the admin token and server.
"#;

/// Prints a completion script.
pub fn completions(shell: Shell, out: &mut impl Write) {
    generate(shell, &mut Cli::command(), "arena", out);
}

/// Writes `completions/` and `man/` into `dir`.
pub fn write_all(dir: &Path) -> Result<()> {
    let completions = dir.join("completions");
    let man = dir.join("man");
    std::fs::create_dir_all(&completions).context("creating the completions directory")?;
    std::fs::create_dir_all(&man).context("creating the man directory")?;
    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
        generate_to(shell, &mut Cli::command(), "arena", &completions)?;
    }
    let mut cmd = Cli::command();
    cmd.build();
    clap_mangen::generate_to(cmd.clone(), &man)?;
    let mut page = Vec::new();
    clap_mangen::Man::new(cmd).render(&mut page)?;
    page.extend_from_slice(MAN_EXTRA.as_bytes());
    std::fs::write(man.join("arena.1"), page)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_completions_and_man_pages() {
        let d = tempfile::tempdir().unwrap();
        write_all(d.path()).unwrap();
        for file in [
            "completions/arena.bash",
            "completions/_arena",
            "completions/arena.fish",
            "man/arena-login.1",
            "man/arena-admin-round-start.1",
        ] {
            assert!(d.path().join(file).is_file(), "{file}");
        }
        let page = std::fs::read_to_string(d.path().join("man/arena.1")).unwrap();
        assert!(page.contains("EXIT STATUS"));
    }
}
