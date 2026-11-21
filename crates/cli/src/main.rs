use std::process::ExitCode;

use anyhow::Result;
use arena_cli::cli::{AdminCommand, Cli, Command, ProblemCommand, RoundCommand, UserCommand};
use arena_cli::{admin, ask, board, client::Client, config::Config, errors, generate, participant};
use arena_protocol::CreateRound;
use clap::Parser;

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => ExitCode::from(errors::report(&e).await),
    }
}

/// `Ok(false)` means the command ran but the result was negative (e.g. not accepted).
async fn run(cli: Cli) -> Result<bool> {
    // These work without a saved configuration.
    let command = match cli.command {
        Command::Trust { server, yes } => {
            participant::trust(server.as_deref(), yes).await?;
            return Ok(true);
        }
        Command::Signup {
            server,
            username,
            display_name,
            join_code,
            password_stdin,
        } => {
            participant::signup(
                server.as_deref(),
                username,
                display_name,
                join_code,
                password_stdin,
            )
            .await?;
            return Ok(true);
        }
        Command::Login { admin: true, .. } => {
            participant::admin_login().await?;
            return Ok(true);
        }
        Command::Login {
            server,
            username,
            password_stdin,
            admin: false,
        } => {
            participant::login(server.as_deref(), username, password_stdin).await?;
            return Ok(true);
        }
        Command::Logout { admin } => {
            participant::logout(admin).await?;
            return Ok(true);
        }
        Command::Completions { shell } => {
            generate::completions(shell, &mut std::io::stdout());
            return Ok(true);
        }
        Command::Generate { dir } => {
            generate::write_all(&dir)?;
            return Ok(true);
        }
        command => command,
    };
    let config = Config::load()?;
    match command {
        Command::Signup { .. }
        | Command::Trust { .. }
        | Command::Login { .. }
        | Command::Logout { .. }
        | Command::Completions { .. }
        | Command::Generate { .. } => unreachable!("handled above"),
        Command::Whoami { json } => participant::whoami(&config, json).await?,
        Command::Round { json } => participant::round(&config, json).await?,
        Command::Problem {
            slug,
            samples,
            json,
        } => {
            let slug: String = ask::required("Problem", "<SLUG>", slug)?;
            participant::problem(&config, &slug, samples.as_deref(), json).await?
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
        Command::Submissions { id, json } => participant::submissions(&config, id, json).await?,
        Command::Board => {
            let client = Client::with_ca(
                config.server()?,
                config.token.clone(),
                config.ca_certificate.as_deref(),
            )?;
            board::run(client, config.token.is_some()).await?
        }
        Command::Admin { command } => {
            let c = admin::admin(&config)?;
            match command {
                AdminCommand::User(UserCommand::List { json }) => {
                    admin::user_list(&c, json).await?
                }
                AdminCommand::User(UserCommand::ResetPassword { name }) => {
                    if name.is_none() && ask::is_interactive() {
                        admin::user_list(&c, false).await?;
                    }
                    let name: String = ask::required("Username", "<NAME>", name)?;
                    admin::reset_password(&c, &name).await?
                }
                AdminCommand::Problem(ProblemCommand::Add { dir, slug, rejudge }) => {
                    let dir = ask::path("Problem directory", "<DIR>", dir)?;
                    admin::problem_add(&c, &dir, slug.as_deref(), rejudge).await?
                }
                AdminCommand::Problem(ProblemCommand::List { json }) => {
                    admin::problem_list(&c, json).await?
                }
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
                        admin::problem_list(&c, false).await?;
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
                AdminCommand::Round(RoundCommand::List { json }) => {
                    admin::round_list(&c, json).await?
                }
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
                AdminCommand::Submissions { round, json } => {
                    admin::submissions(&c, round, json).await?
                }
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
        admin::round_list(c, false).await?;
    }
    ask::required("Round ID", "<ID>", id)
}

/// The given submission, or one picked from the current round's list.
async fn submission_id(c: &Client, id: Option<i64>) -> Result<i64> {
    if id.is_none() && ask::is_interactive() {
        admin::submissions(c, None, false).await?;
    }
    ask::required("Submission ID", "<ID>", id)
}
