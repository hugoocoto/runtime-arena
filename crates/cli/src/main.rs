use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use arena_cli::{admin, board, client::Client, config::Config, participant};
use arena_protocol::CreateRound;
use clap::{Parser, Subcommand};

/// Runtime Arena: submit solutions and follow the contest from your terminal.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Save the server and your token (from the organiser).
    /// Log in: asks for the username and password you created on the server's
    /// /join page. Organisers can pass a token instead.
    Login {
        /// e.g. arena.local:8080 (remembered after the first login)
        server: Option<String>,
        /// A token from the organiser, instead of username and password.
        token: Option<String>,
        /// Save the token as the admin token.
        #[arg(long)]
        admin: bool,
    },
    /// Show who you are logged in as.
    Whoami,
    /// Show the current round and its problems.
    Round,
    /// Show a problem's statement and samples.
    Problem {
        slug: String,
        /// Also write the samples as NAME.in / NAME.out into this directory.
        #[arg(long)]
        samples: Option<PathBuf>,
    },
    /// Build in the judge's environment (Docker): `arena build -- gcc -O2 -o sol sol.c`.
    Build {
        /// Build image to use.
        #[arg(long, default_value = concat!("ghcr.io/hugoocoto/runtime-arena-build-env:", env!("CARGO_PKG_VERSION")))]
        image: String,
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Submit an executable and follow its judging.
    Submit {
        problem: String,
        /// Linux x86-64 executable or #! script.
        executable: PathBuf,
        /// Your source code (file or directory); required, kept for review.
        #[arg(long)]
        src: PathBuf,
        /// Return right after uploading.
        #[arg(long)]
        no_wait: bool,
    },
    /// List your submissions in the current round, or show one.
    Submissions { id: Option<i64> },
    /// Live leaderboards and charts.
    Board,
    /// Organiser commands (need the admin token).
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
}

#[derive(Subcommand)]
enum AdminCommand {
    #[command(subcommand)]
    User(UserCommand),
    #[command(subcommand)]
    Problem(ProblemCommand),
    #[command(subcommand)]
    Round(RoundCommand),
    /// List submissions of a round (default: current).
    Submissions {
        #[arg(long)]
        round: Option<i64>,
    },
    /// Download a submission's source archive.
    Source {
        id: i64,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Judge a submission again.
    Rejudge { id: i64 },
}

#[derive(Subcommand)]
enum UserCommand {
    /// Create a participant and print their token.
    Add {
        name: String,
        #[arg(long)]
        display_name: Option<String>,
    },
    List,
    /// Issue a new token (the old one stops working).
    Reset {
        name: String,
    },
}

#[derive(Subcommand)]
enum ProblemCommand {
    /// Upload a problem directory (problem.toml, statement.md, tests/).
    Add {
        dir: PathBuf,
        /// Defaults to the directory name.
        #[arg(long)]
        slug: Option<String>,
        /// Replace a problem of a started round and rejudge its submissions.
        #[arg(long)]
        rejudge: bool,
    },
    List,
}

#[derive(Subcommand)]
enum RoundCommand {
    /// Create a round (it starts with `round start`).
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        minutes: u32,
        /// Freeze public boards this many minutes before the end.
        #[arg(long, default_value_t = 0)]
        freeze: u32,
        /// Seconds between two submissions of one participant.
        #[arg(long, default_value_t = 30)]
        cooldown: u32,
        /// Problem slugs in display order.
        #[arg(required = true)]
        problems: Vec<String>,
    },
    List,
    Start {
        id: i64,
    },
    Stop {
        id: i64,
    },
    Extend {
        id: i64,
        minutes: u32,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

/// `Ok(false)` means the command ran but the result was negative (e.g. not accepted).
async fn run(cli: Cli) -> Result<bool> {
    if let Command::Login {
        server,
        token,
        admin,
    } = &cli.command
    {
        participant::login(server.as_deref(), token.as_deref(), *admin).await?;
        return Ok(true);
    }
    let config = Config::load()?;
    match cli.command {
        Command::Login { .. } => unreachable!(),
        Command::Whoami => participant::whoami(&config).await?,
        Command::Round => participant::round(&config).await?,
        Command::Problem { slug, samples } => {
            participant::problem(&config, &slug, samples.as_deref()).await?
        }
        Command::Build { image, command } => return participant::build(&image, &command),
        Command::Submit {
            problem,
            executable,
            src,
            no_wait,
        } => return participant::submit(&config, &problem, &executable, &src, !no_wait).await,
        Command::Submissions { id } => participant::submissions(&config, id).await?,
        Command::Board => {
            let client = Client::new(config.server()?, config.token.clone())?;
            board::run(client, config.token.is_some()).await?
        }
        Command::Admin { command } => {
            let c = admin::admin(&config)?;
            match command {
                AdminCommand::User(UserCommand::Add { name, display_name }) => {
                    admin::user_add(&c, &name, display_name).await?
                }
                AdminCommand::User(UserCommand::List) => admin::user_list(&c).await?,
                AdminCommand::User(UserCommand::Reset { name }) => {
                    admin::user_reset(&c, &name).await?
                }
                AdminCommand::Problem(ProblemCommand::Add { dir, slug, rejudge }) => {
                    admin::problem_add(&c, &dir, slug.as_deref(), rejudge).await?
                }
                AdminCommand::Problem(ProblemCommand::List) => admin::problem_list(&c).await?,
                AdminCommand::Round(RoundCommand::Create {
                    name,
                    minutes,
                    freeze,
                    cooldown,
                    problems,
                }) => {
                    let req = CreateRound {
                        name,
                        duration_minutes: minutes,
                        freeze_minutes: freeze,
                        cooldown_seconds: cooldown,
                        problems,
                    };
                    admin::round_create(&c, req).await?
                }
                AdminCommand::Round(RoundCommand::List) => admin::round_list(&c).await?,
                AdminCommand::Round(RoundCommand::Start { id }) => {
                    admin::round_action(&c, id, "start", None).await?
                }
                AdminCommand::Round(RoundCommand::Stop { id }) => {
                    admin::round_action(&c, id, "stop", None).await?
                }
                AdminCommand::Round(RoundCommand::Extend { id, minutes }) => {
                    admin::round_action(&c, id, "extend", Some(minutes)).await?
                }
                AdminCommand::Submissions { round } => admin::submissions(&c, round).await?,
                AdminCommand::Source { id, output } => {
                    let out = output.unwrap_or_else(|| format!("submission-{id}.tar.gz").into());
                    admin::source(&c, id, &out).await?
                }
                AdminCommand::Rejudge { id } => admin::rejudge(&c, id).await?,
            }
        }
    }
    Ok(true)
}
