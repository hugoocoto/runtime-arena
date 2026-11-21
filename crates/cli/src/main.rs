use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use arena_cli::{admin, ask, board, client::Client, config::Config, participant};
use arena_protocol::CreateRound;
use clap::{Parser, Subcommand};

/// Runtime Arena: submit solutions and follow the contest from your terminal.
/// In a terminal, arguments you leave out are asked for.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Log in: asks for the username and password you created on the server's
    /// /join page, or takes a token from the organiser.
    Login {
        /// e.g. arena.local:8080 (remembered after the first login)
        server: Option<String>,
        /// A token from the organiser, instead of username and password.
        token: Option<String>,
        /// Log in as admin. Takes no arguments: reads ARENA_ADMIN_TOKEN and
        /// ARENA_SERVER (default localhost:ARENA_PORT) from the environment or ./.env.
        #[arg(long, conflicts_with_all = ["server", "token"])]
        admin: bool,
    },
    /// Show who you are logged in as.
    Whoami,
    /// Show the current round and its problems.
    Round,
    /// Show a problem's statement and samples.
    Problem {
        slug: Option<String>,
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
        problem: Option<String>,
        /// Linux x86-64 executable or #! script.
        executable: Option<PathBuf>,
        /// Your source code (file or directory); required, kept for review.
        #[arg(long)]
        src: Option<PathBuf>,
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
        id: Option<i64>,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Judge a submission again.
    Rejudge { id: Option<i64> },
    /// Check whether a newer server release exists.
    Update,
}

#[derive(Subcommand)]
enum UserCommand {
    /// Create a participant and print their token.
    Add {
        name: Option<String>,
        #[arg(long)]
        display_name: Option<String>,
    },
    List,
    /// Issue a new token (the old one stops working).
    Reset {
        name: Option<String>,
    },
}

#[derive(Subcommand)]
enum ProblemCommand {
    /// Upload a problem directory (problem.toml, statement.md, tests/).
    Add {
        dir: Option<PathBuf>,
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
        name: Option<String>,
        #[arg(long)]
        minutes: Option<u32>,
        /// Freeze public boards this many minutes before the end [default: 0].
        #[arg(long)]
        freeze: Option<u32>,
        /// Seconds between two submissions of one participant [default: 30].
        #[arg(long)]
        cooldown: Option<u32>,
        /// Problem slugs in display order.
        problems: Vec<String>,
    },
    List,
    Start {
        id: Option<i64>,
    },
    Stop {
        id: Option<i64>,
    },
    Extend {
        id: Option<i64>,
        minutes: Option<u32>,
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
        if *admin {
            participant::admin_login().await?;
        } else {
            participant::login(server.as_deref(), token.as_deref()).await?;
        }
        return Ok(true);
    }
    let config = Config::load()?;
    match cli.command {
        Command::Login { .. } => unreachable!(),
        Command::Whoami => participant::whoami(&config).await?,
        Command::Round => participant::round(&config).await?,
        Command::Problem { slug, samples } => {
            let slug: String = ask::required("Problem", "<SLUG>", slug)?;
            participant::problem(&config, &slug, samples.as_deref()).await?
        }
        Command::Build { image, command } => return participant::build(&image, &command),
        Command::Submit {
            problem,
            executable,
            src,
            no_wait,
        } => {
            let problem: String = ask::required("Problem", "<PROBLEM>", problem)?;
            let executable = ask::path("Executable", "<EXECUTABLE>", executable)?;
            let src = ask::path("Source file or directory", "--src", src)?;
            return participant::submit(&config, &problem, &executable, &src, !no_wait).await;
        }
        Command::Submissions { id } => participant::submissions(&config, id).await?,
        Command::Board => {
            let client = Client::new(config.server()?, config.token.clone())?;
            board::run(client, config.token.is_some()).await?
        }
        Command::Admin { command } => {
            let c = admin::admin(&config)?;
            match command {
                AdminCommand::User(UserCommand::Add { name, display_name }) => {
                    let name: String = ask::required("Username", "<NAME>", name)?;
                    admin::user_add(&c, &name, display_name).await?
                }
                AdminCommand::User(UserCommand::List) => admin::user_list(&c).await?,
                AdminCommand::User(UserCommand::Reset { name }) => {
                    if name.is_none() && ask::is_interactive() {
                        admin::user_list(&c).await?;
                    }
                    let name: String = ask::required("Username", "<NAME>", name)?;
                    admin::user_reset(&c, &name).await?
                }
                AdminCommand::Problem(ProblemCommand::Add { dir, slug, rejudge }) => {
                    let dir = ask::path("Problem directory", "<DIR>", dir)?;
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
                    // Asked only when the command is incomplete, so a full one stays as it was.
                    let prompt = name.is_none() || minutes.is_none() || problems.is_empty();
                    let name: String = ask::required("Round name", "--name", name)?;
                    let minutes: u32 = ask::required("Duration (minutes)", "--minutes", minutes)?;
                    if problems.is_empty() && ask::is_interactive() {
                        println!("Available problems:");
                        admin::problem_list(&c).await?;
                    }
                    let problems = ask::list(
                        "Problems, in order (separated by spaces or commas)",
                        "<PROBLEMS>",
                        problems,
                    )?;
                    let freeze =
                        ask::optional("Freeze boards before the end (minutes)", freeze, 0, prompt)?;
                    let cooldown =
                        ask::optional("Seconds between submissions", cooldown, 30, prompt)?;
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
                    let id = round_id(&c, id).await?;
                    admin::round_action(&c, id, "start", None).await?
                }
                AdminCommand::Round(RoundCommand::Stop { id }) => {
                    let id = round_id(&c, id).await?;
                    admin::round_action(&c, id, "stop", None).await?
                }
                AdminCommand::Round(RoundCommand::Extend { id, minutes }) => {
                    let id = round_id(&c, id).await?;
                    let minutes: u32 = ask::required("Minutes to add", "<MINUTES>", minutes)?;
                    admin::round_action(&c, id, "extend", Some(minutes)).await?
                }
                AdminCommand::Submissions { round } => admin::submissions(&c, round).await?,
                AdminCommand::Source { id, output } => {
                    let id = submission_id(&c, id).await?;
                    let out = output.unwrap_or_else(|| format!("submission-{id}.tar.gz").into());
                    admin::source(&c, id, &out).await?
                }
                AdminCommand::Rejudge { id } => {
                    let id = submission_id(&c, id).await?;
                    admin::rejudge(&c, id).await?
                }
                AdminCommand::Update => admin::update(&c).await?,
            }
        }
    }
    Ok(true)
}

/// The given round, or one picked from the list.
async fn round_id(c: &Client, id: Option<i64>) -> Result<i64> {
    if id.is_none() && ask::is_interactive() {
        admin::round_list(c).await?;
    }
    ask::required("Round ID", "<ID>", id)
}

/// The given submission, or one picked from the current round's list.
async fn submission_id(c: &Client, id: Option<i64>) -> Result<i64> {
    if id.is_none() && ask::is_interactive() {
        admin::submissions(c, None).await?;
    }
    ask::required("Submission ID", "<ID>", id)
}
