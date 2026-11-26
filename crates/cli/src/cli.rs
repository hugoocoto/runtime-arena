//! The command line, in the library so completions and man pages come from it.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

const BUILD_IMAGE: &str = concat!(
    "ghcr.io/hugoocoto/runtime-arena-build-env:",
    env!("CARGO_PKG_VERSION")
);

/// Runtime Arena: submit solutions and follow the contest from your terminal.
/// In a terminal, arguments you leave out are asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum BuildPlatform {
    #[value(name = "linux/amd64")]
    Amd64,
    #[value(name = "linux/arm64")]
    Arm64,
}

impl BuildPlatform {
    pub fn docker_platform(self) -> &'static str {
        match self {
            Self::Amd64 => "linux/amd64",
            Self::Arm64 => "linux/arm64",
        }
    }
}

#[derive(Parser)]
#[command(name = "arena", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Trust the LAN CA served by a local HTTPS deployment.
    Trust {
        /// e.g. https://192.168.1.42 (remembered afterwards)
        server: Option<String>,
        /// SHA-256 fingerprint from the organiser, in hex with optional colons.
        #[arg(long, conflicts_with = "yes")]
        fingerprint: Option<String>,
        /// Read a single PEM certificate from this file instead of downloading it.
        #[arg(long)]
        ca_file: Option<PathBuf>,
        /// Trust the CA without checking its fingerprint.
        #[arg(short, long)]
        yes: bool,
    },
    /// Create an account with the organiser's event code and log in.
    Signup {
        /// e.g. https://arena.local (remembered afterwards)
        server: Option<String>,
        #[arg(short, long)]
        username: Option<String>,
        /// Name shown on the leaderboards [default: the username].
        #[arg(long)]
        display_name: Option<String>,
        /// The event code announced by the organiser.
        #[arg(long, env = "ARENA_JOIN_CODE", hide_env_values = true)]
        join_code: Option<String>,
        /// Read the password from standard input, for scripts.
        #[arg(long)]
        password_stdin: bool,
    },
    /// Log in with your username and password.
    Login {
        /// e.g. https://arena.local (remembered after the first login)
        server: Option<String>,
        #[arg(short, long, conflicts_with = "admin")]
        username: Option<String>,
        /// Read the password from standard input, for scripts.
        #[arg(long, conflicts_with = "admin")]
        password_stdin: bool,
        /// Log in as admin. Reads ARENA_ADMIN_TOKEN and ARENA_SERVER
        /// (default localhost:ARENA_PORT) from the environment or ./.env.
        #[arg(long, conflicts_with = "server")]
        admin: bool,
    },
    /// Log out of this device.
    Logout {
        /// Forget the saved admin token instead.
        #[arg(long)]
        admin: bool,
    },
    /// Show who you are logged in as.
    Whoami {
        #[arg(long)]
        json: bool,
    },
    /// Show the current round and its problems.
    Round {
        #[arg(long)]
        json: bool,
    },
    /// Show the judge machine (CPU, caches, memory) and how submissions are timed.
    Specs {
        #[arg(long)]
        json: bool,
    },
    /// Show a problem's statement and samples.
    Problem {
        slug: Option<String>,
        /// Also write the samples as NAME.in / NAME.out into this directory.
        #[arg(long, value_hint = clap::ValueHint::DirPath)]
        samples: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Build in the judge's environment (Docker): `arena build -- gcc -O2 -o sol sol.c`.
    Build {
        /// Build image to use.
        #[arg(long, default_value = BUILD_IMAGE)]
        image: String,
        /// Override judge detection, for example when building offline.
        #[arg(long)]
        platform: Option<BuildPlatform>,
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Download the build image now, so the first `arena build` doesn't have to.
    Pull {
        /// Build image to download.
        #[arg(long, default_value = BUILD_IMAGE)]
        image: String,
        /// Override judge detection, for example before signing in.
        #[arg(long)]
        platform: Option<BuildPlatform>,
    },
    /// Submit an executable and follow its judging.
    Submit {
        problem: Option<String>,
        /// Linux executable matching the judge architecture (see arena specs), or a #! script.
        #[arg(value_hint = clap::ValueHint::FilePath)]
        executable: Option<PathBuf>,
        /// Your source code (file or directory); required, kept for review.
        #[arg(long, value_hint = clap::ValueHint::AnyPath)]
        src: Option<PathBuf>,
        /// Return right after uploading.
        #[arg(long)]
        no_wait: bool,
    },
    /// List your submissions in the current round, or show one.
    Submissions {
        #[arg(value_parser = positive_id)]
        id: Option<i64>,
        #[arg(long)]
        json: bool,
        /// Show your refused submissions across rounds.
        #[arg(long, requires = "json", conflicts_with = "id")]
        refused: bool,
    },
    /// Live leaderboards and charts (or a snapshot with --once / --json).
    Board {
        /// Print the current boards once as text instead of opening the live view.
        #[arg(long)]
        once: bool,
        /// Print the current boards as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Organiser commands (need the admin token).
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
    /// Print a shell completion script.
    Completions { shell: Shell },
    /// Write completions and man pages into a directory (used for releases).
    #[command(hide = true)]
    Generate { dir: PathBuf },
}

#[derive(Subcommand)]
pub enum AdminCommand {
    /// Participants, who create their own accounts with `arena signup` or on /join.
    #[command(subcommand)]
    User(UserCommand),
    /// Contest problems.
    #[command(subcommand)]
    Problem(ProblemCommand),
    /// Contest rounds.
    #[command(subcommand)]
    Round(RoundCommand),
    /// List submissions of a round (default: current).
    Submissions {
        #[arg(long, value_parser = positive_id)]
        round: Option<i64>,
        #[arg(long)]
        json: bool,
    },
    /// Download a submission's source archive.
    Source {
        #[arg(value_parser = positive_id)]
        id: Option<i64>,
        #[arg(short, long, value_hint = clap::ValueHint::FilePath)]
        output: Option<PathBuf>,
    },
    /// Judge a submission again.
    Rejudge {
        #[arg(value_parser = positive_id)]
        id: Option<i64>,
    },
    /// Check whether a newer server release exists.
    Update,
}

#[derive(Subcommand)]
pub enum UserCommand {
    /// List participants.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Give a participant a new random password and log out all their devices.
    ResetPassword { name: Option<String> },
}

#[derive(Subcommand)]
pub enum ProblemCommand {
    /// Upload a problem directory (problem.toml, statement.md, tests/).
    Add {
        #[arg(value_hint = clap::ValueHint::DirPath)]
        dir: Option<PathBuf>,
        /// Defaults to the directory name.
        #[arg(long)]
        slug: Option<String>,
        /// Replace a problem of a started round and rejudge its submissions.
        #[arg(long)]
        rejudge: bool,
    },
    /// List problems.
    List {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum RoundCommand {
    /// Create a round (it starts with `round start`).
    Create {
        #[arg(long)]
        name: Option<String>,
        #[arg(long, value_parser = whole_number)]
        minutes: Option<u32>,
        /// Freeze public boards this many minutes before the end [default: 0].
        #[arg(long, value_parser = whole_number)]
        freeze: Option<u32>,
        /// Seconds between two submissions of one participant [default: 30].
        #[arg(long, value_parser = whole_number)]
        cooldown: Option<u32>,
        /// Problem slugs in display order.
        problems: Vec<String>,
    },
    /// List rounds.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Export the statements, limits, and public samples of a round.
    Export {
        #[arg(value_parser = positive_id)]
        id: i64,
        #[arg(short, long, value_hint = clap::ValueHint::FilePath)]
        output: Option<PathBuf>,
    },
    /// Copy a round's settings and problems to a new draft.
    Clone {
        #[arg(value_parser = positive_id)]
        id: Option<i64>,
    },
    /// Start a round.
    Start {
        #[arg(value_parser = positive_id)]
        id: Option<i64>,
    },
    /// End a running round now.
    Stop {
        #[arg(value_parser = positive_id)]
        id: Option<i64>,
    },
    /// Add minutes to a running round.
    Extend {
        #[arg(value_parser = positive_id)]
        id: Option<i64>,
        #[arg(value_parser = whole_number)]
        minutes: Option<u32>,
    },
    /// Remove a draft or finished round and its submissions.
    Delete {
        #[arg(value_parser = positive_id)]
        id: Option<i64>,
    },
}

fn positive_id(s: &str) -> Result<i64, String> {
    s.parse::<i64>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| "expected a positive whole number, e.g. 12".to_owned())
}

fn whole_number(s: &str) -> Result<u32, String> {
    s.parse()
        .map_err(|_| "expected a whole number, e.g. 30".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_line_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn numbers_get_readable_errors() {
        assert_eq!(positive_id("12"), Ok(12));
        assert!(positive_id("0").is_err());
        assert!(positive_id("abc").unwrap_err().contains("positive"));
        assert!(whole_number("-1").is_err());
    }

    #[test]
    fn pull_defaults_to_the_image_build_uses() {
        let Command::Pull { image: pulled, .. } = Cli::parse_from(["arena", "pull"]).command else {
            panic!("expected pull");
        };
        let Command::Build { image: built, .. } =
            Cli::parse_from(["arena", "build", "--", "true"]).command
        else {
            panic!("expected build");
        };
        assert_eq!(pulled, built);
    }

    #[test]
    fn login_no_longer_takes_a_token() {
        assert!(Cli::try_parse_from(["arena", "login", "host:8080", "arena_abc"]).is_err());
    }

    #[test]
    fn board_accepts_once_and_json() {
        assert!(matches!(
            Cli::parse_from(["arena", "board"]).command,
            Command::Board {
                once: false,
                json: false
            }
        ));
        assert!(matches!(
            Cli::parse_from(["arena", "board", "--once"]).command,
            Command::Board {
                once: true,
                json: false
            }
        ));
        assert!(matches!(
            Cli::parse_from(["arena", "board", "--json"]).command,
            Command::Board {
                once: false,
                json: true
            }
        ));
    }
}
