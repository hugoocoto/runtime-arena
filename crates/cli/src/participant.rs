//! Commands for participants: signup, login, logout, round, problem, submit, submissions, build.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use arena_protocol::{
    HostInfo, LoginRequest, ProblemStatement, RoundInfo, RoundState, ServerEvent, ServerInfo,
    SignupRequest, SubmissionInfo, SubmissionStatus, UserInfo, UserToken, Verdict,
};

use crate::ask;
use crate::client::{Client, version_mismatch};
use crate::config::Config;
use crate::errors::{NotLoggedIn, UsageError, status_of};
use crate::format;

/// Largest source archive the server accepts.
const MAX_SOURCE: usize = 16 * 1024 * 1024;
/// Directories never included in a source archive.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "__pycache__",
    ".venv",
    "build",
];

pub fn participant(config: &Config) -> Result<Client> {
    let token = config.token.clone().ok_or_else(|| {
        NotLoggedIn(
            "not logged in; run `arena login <server>`, \
             or `arena signup <server>` for a new account"
                .into(),
        )
    })?;
    Client::with_ca(
        config.server()?,
        Some(token),
        config.ca_certificate.as_deref(),
    )
}

fn trust_server(server: &str) -> Result<String> {
    let server = server.trim().trim_end_matches('/');
    if server.is_empty() {
        bail!("no server address given");
    }
    let server = if server.starts_with("https://") {
        server.to_owned()
    } else if server.starts_with("http://") {
        bail!("LAN trust needs the final HTTPS address, for example https://192.168.1.42");
    } else {
        format!("https://{server}")
    };
    Ok(crate::client::normalise_server(&server))
}

/// Downloads and saves the CA used by a LAN Caddy deployment.
pub async fn trust(server: Option<&str>, yes: bool) -> Result<()> {
    let server = match server {
        Some(server) => trust_server(server)?,
        None => {
            let server: String = ask::required("HTTPS server", "<SERVER>", None)?;
            trust_server(&server)?
        }
    };
    let certificate = Client::download_ca(&server).await?;
    let fingerprint = Client::certificate_fingerprint(&certificate);
    println!("LAN CA SHA-256 fingerprint: {fingerprint}");
    if !yes {
        let answer: String = ask::required("Trust this certificate? Type yes", "--yes", None)?;
        if answer != "yes" {
            bail!("certificate not trusted");
        }
    }
    let mut config = Config::load_file()?;
    config.set_server(&server);
    config.ca_certificate = Some(String::from_utf8(certificate)?);
    let path = config.save()?;
    println!(
        "Trusted LAN certificate for {server} in {}.",
        path.display()
    );
    Ok(())
}

/// The given server, else the remembered one.
fn target_server(server: Option<&str>, command: &str) -> Result<String> {
    match server {
        Some(s) => Ok(crate::client::normalise_server(s)),
        None => Config::load()?.server.ok_or_else(|| {
            UsageError(format!(
                "give the server address, e.g. `arena {command} https://arena.local`"
            ))
            .into()
        }),
    }
}

/// A password from standard input or a hidden prompt, typed twice if `confirm`.
fn read_password(from_stdin: bool, confirm: bool, min_length: Option<usize>) -> Result<String> {
    if from_stdin {
        if ask::is_interactive() {
            return Err(UsageError("--password-stdin needs the password piped in".into()).into());
        }
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        let password = line.trim_end_matches(['\r', '\n']).to_owned();
        if password.is_empty() {
            return Err(UsageError("no password on standard input".into()).into());
        }
        if let Some(min_length) = min_length
            && password.chars().count() < min_length
        {
            return Err(UsageError(format!(
                "the password needs at least {min_length} characters"
            ))
            .into());
        }
        return Ok(password);
    }
    if !ask::is_interactive() {
        return Err(UsageError("missing password; pipe it in with --password-stdin".into()).into());
    }
    loop {
        let password = rpassword::prompt_password("Password: ")?;
        if confirm && password != rpassword::prompt_password("Repeat password: ")? {
            eprintln!("error: the passwords don't match");
            continue;
        }
        if let Some(min_length) = min_length
            && password.chars().count() < min_length
        {
            eprintln!("error: the password needs at least {min_length} characters");
            continue;
        }
        return Ok(password);
    }
}

/// Exchanges name and password for a device token and saves it.
async fn log_in(server: &str, name: String, password: String) -> Result<()> {
    let mut config = Config::load_file()?;
    let t: UserToken = Client::with_ca(server, None, config.ca_certificate.as_deref())?
        .post("/api/login", &LoginRequest { name, password })
        .await?;
    let client = Client::with_ca(
        server,
        Some(t.token.clone()),
        config.ca_certificate.as_deref(),
    )?;
    let me: UserInfo = client.get("/api/me").await?;
    config.set_server(server);
    config.token = Some(t.token);
    config.save()?;
    println!("Logged in as {} ({}).", me.display_name, me.name);
    client.warn_if_version_differs().await;
    Ok(())
}

/// Logs in with the username and password of an existing account.
pub async fn login(
    server: Option<&str>,
    username: Option<String>,
    password_stdin: bool,
) -> Result<()> {
    let server = target_server(server, "login")?;
    let name: String = ask::required("Username", "--username", username)?;
    let password = read_password(password_stdin, false, None)?;
    log_in(&server, name, password).await
}

/// Creates an account with the event code, then logs in to it.
pub async fn signup(
    server: Option<&str>,
    username: Option<String>,
    display_name: Option<String>,
    join_code: Option<String>,
    password_stdin: bool,
) -> Result<()> {
    let server = target_server(server, "signup")?;
    let name: String = ask::required("Username", "--username", username)?;
    let join_code: String = ask::required("Event code", "--join-code", join_code)?;
    let display_name: String = ask::optional(
        "Name on leaderboard",
        display_name,
        name.clone(),
        ask::is_interactive(),
    )?;
    let password = read_password(password_stdin, true, Some(6))?;
    let saved = Config::load_file()?;
    let ca = (saved.server.as_deref().map(crate::client::normalise_server) == Some(server.clone()))
        .then_some(saved.ca_certificate)
        .flatten();
    let created: UserInfo = Client::with_ca(&server, None, ca.as_deref())?
        .post(
            "/api/signup",
            &SignupRequest {
                name,
                display_name: Some(display_name),
                password: password.clone(),
                join_code,
            },
        )
        .await?;
    log_in(&server, created.name, password).await
}

/// Forgets the saved token, ending its session on the server when possible.
/// The local token always goes, so logging out works offline too.
pub async fn logout(admin: bool) -> Result<()> {
    let mut config = Config::load_file()?;
    if admin {
        if config.admin_token.take().is_none() {
            println!("No admin token saved.");
            return Ok(());
        }
        config.save()?;
        println!("Admin token removed from this device.");
        return Ok(());
    }
    let Some(token) = config.token.take() else {
        println!("Not logged in.");
        return Ok(());
    };
    let ended = match config.server.as_deref() {
        Some(server) => Client::with_ca(server, Some(token), config.ca_certificate.as_deref())?
            .post_empty("/api/logout")
            .await
            .map_err(|e| (status_of(&e), e)),
        None => Ok(()),
    };
    config.save()?;
    match ended {
        Ok(()) => println!("Logged out."),
        // The session is already gone.
        Err((Some(reqwest::StatusCode::UNAUTHORIZED), _)) => println!("Logged out."),
        Err((_, e)) => {
            println!("Logged out of this device.");
            eprintln!("warning: the session may still be active on the server: {e:#}");
        }
    }
    Ok(())
}

/// Saves the admin token, taken only from the environment or `./.env`, so it
/// never appears in the command line or the shell history.
pub async fn admin_login() -> Result<()> {
    let mut config = Config::load_file()?;
    let file = crate::config::dotenv()?;
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let (server, token) = admin_target(env, |k| file.get(k).cloned())?;
    if env("ARENA_ADMIN_TOKEN").is_none() {
        crate::config::warn_if_dotenv_shared();
    }
    if server.starts_with("http://") && !is_loopback(&server) {
        eprintln!("warning: sending the admin token to {server} without encryption (http)");
    }
    let client = Client::with_ca(
        &server,
        Some(token.clone()),
        config.ca_certificate.as_deref(),
    )?;
    client
        .get::<Vec<UserInfo>>("/api/admin/users")
        .await
        .with_context(|| format!("could not verify the admin token on {server}"))?;
    config.set_server(&server);
    config.admin_token = Some(token);
    config.save()?;
    println!("Admin token verified on {server}.");
    crate::admin::notify_update(&client).await;
    Ok(())
}

/// Picks the server and token for `login --admin`. Both come from the same
/// place (environment, else `.env`), so a `.env` can't redirect an exported
/// token, nor the environment a `.env` token. With no server, uses localhost.
fn admin_target(
    env: impl Fn(&str) -> Option<String>,
    dotenv: impl Fn(&str) -> Option<String>,
) -> Result<(String, String)> {
    if let Some(token) = env("ARENA_ADMIN_TOKEN") {
        let server = match env("ARENA_SERVER") {
            Some(s) => checked_server(&s)?,
            None => local_server(&env, &dotenv)?,
        };
        return Ok((server, token));
    }
    let token = dotenv("ARENA_ADMIN_TOKEN").context(
        "no admin token found: set ARENA_ADMIN_TOKEN (and ARENA_SERVER for a remote server) \
         in the environment or in ./.env",
    )?;
    let server = match dotenv("ARENA_SERVER") {
        Some(s) => checked_server(&s)?,
        None => local_server(&env, &dotenv)?,
    };
    if let Some(s) = env("ARENA_SERVER")
        && checked_server(&s)? != server
    {
        bail!(
            "ARENA_SERVER in the environment differs from the server for the token in .env; \
             set ARENA_SERVER and ARENA_ADMIN_TOKEN in the same place"
        );
    }
    Ok((server, token))
}

/// A normalised http(s) URL. Errors never echo the value, which may be a secret.
fn checked_server(server: &str) -> Result<String> {
    // Checked first: normalising would turn `ftp://x` into `http://ftp://x`.
    if let Some((scheme, _)) = server.trim().split_once("://")
        && !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https")
    {
        bail!("ARENA_SERVER must be an http(s) URL");
    }
    let server = crate::client::normalise_server(server);
    let url = reqwest::Url::parse(&server)
        .map_err(|_| anyhow::anyhow!("ARENA_SERVER is not a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("ARENA_SERVER must be an http(s) URL");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("ARENA_SERVER must not contain credentials");
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("ARENA_SERVER must not have a query or fragment");
    }
    Ok(server)
}

fn local_server(
    env: &impl Fn(&str) -> Option<String>,
    dotenv: &impl Fn(&str) -> Option<String>,
) -> Result<String> {
    let port = env("ARENA_PORT")
        .or_else(|| dotenv("ARENA_PORT"))
        .unwrap_or_else(|| "8080".into());
    // Compose also accepts `127.0.0.1:8080` here.
    let number = port.rsplit(':').next().unwrap_or(&port);
    let number: u16 = number
        .parse()
        .with_context(|| format!("ARENA_PORT is not a port: {port}"))?;
    Ok(format!("http://localhost:{number}"))
}

fn is_loopback(server: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(server) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

pub async fn whoami(config: &Config, json: bool) -> Result<()> {
    let client = participant(config)?;
    let info: ServerInfo = client.get("/api/info").await?;
    let me: UserInfo = client.get("/api/me").await?;
    if let Some(warning) = version_mismatch(env!("CARGO_PKG_VERSION"), &info.version) {
        eprintln!("{warning}");
    }
    if json {
        return format::print_json(&serde_json::json!({
            "name": me.name,
            "display_name": me.display_name,
            "server": client.base(),
            "server_version": info.version,
            "source_url": info.source_url,
        }));
    }
    println!("{} ({}) on {}", me.display_name, me.name, client.base());
    println!("server {} — source: {}", info.version, info.source_url);
    Ok(())
}

pub async fn specs(config: &Config, json: bool) -> Result<()> {
    let client = participant(config)?;
    let host: HostInfo = client.get("/api/host").await?;
    if json {
        return format::print_json(&host);
    }
    print!("{}", render_specs(&host, client.base()));
    Ok(())
}

/// The `arena specs` text.
fn render_specs(h: &HostInfo, server: &str) -> String {
    let mut out = format!("Judge machine of {server}\n");
    let mut row = |label: &str, value: String| out.push_str(&format!("  {label:<8} {value}\n"));
    let mut cpu = h.cpu_model.clone().unwrap_or_else(|| "unknown".to_owned());
    match h.cpus {
        Some(n) => cpu.push_str(&format!(" ({n} logical CPUs, {})", h.arch)),
        None => cpu.push_str(&format!(" ({})", h.arch)),
    }
    row("CPU", cpu);
    if let Some(level) = h.x86_64_level {
        row(
            "Target",
            format!("x86-64-v{level} (safe to build with -march=x86-64-v{level})"),
        );
    }
    if !h.cpu_flags.is_empty() {
        row("Flags", h.cpu_flags.join(" "));
    }
    if !h.caches.is_empty() {
        let caches: Vec<String> = h
            .caches
            .iter()
            .map(|c| {
                let suffix = match c.kind.as_str() {
                    "data" => "d",
                    "instruction" => "i",
                    _ => "",
                };
                format!("L{}{suffix} {}", c.level, format::memory(c.size_kib))
            })
            .collect();
        row("Caches", caches.join(", "));
    }
    if let Some(kib) = h.memory_kib {
        row(
            "Memory",
            format!(
                "{:.1} GiB in total; each problem sets its own limit",
                kib as f64 / (1024.0 * 1024.0)
            ),
        );
    }
    if let Some(kernel) = &h.kernel {
        row("Kernel", format!("Linux {kernel}"));
    }
    let j = &h.judge;
    out.push_str("Judging\n");
    let mut line = |s: String| out.push_str(&format!("  {s}\n"));
    line(match j.workers {
        1 => "one submission at a time".to_owned(),
        n => format!("{n} submissions at a time, each in its own sandbox"),
    });
    line(match j.runs_per_test {
        1 => "each test runs once".to_owned(),
        n => format!(
            "each test runs {n} times and must pass every run; the best time and memory count"
        ),
    });
    line(format!(
        "time is CPU time of all processes and threads, measured in {}",
        if j.microsecond_timing {
            "microseconds"
        } else {
            "milliseconds"
        }
    ));
    line(format!("up to {} processes and threads", j.processes));
    if j.max_failures > 0 {
        line(format!(
            "judging stops after {} failed tests",
            j.max_failures
        ));
    }
    out
}

pub fn print_round(round: &RoundInfo) {
    let now = format::now();
    let when = match round.state {
        RoundState::Draft => "not started".to_owned(),
        RoundState::Running => format!(
            "running, {} left",
            format::duration(round.ends_at.unwrap_or(now) - now)
        ),
        RoundState::Finished => "finished".to_owned(),
    };
    println!("Round {}: {} ({when})", round.id, round.name);
    if round.freeze_minutes > 0 {
        println!(
            "  boards freeze {} min before the end",
            round.freeze_minutes
        );
    }
    println!("  {} s between submissions", round.cooldown_seconds);
    for p in &round.problems {
        println!(
            "  {:<20} {}  [{} ms, {} MiB]",
            p.slug, p.title, p.time_limit_ms, p.memory_limit_mb
        );
    }
}

pub async fn round(config: &Config, json: bool) -> Result<()> {
    let client = Client::with_ca(
        config.server()?,
        config.token.clone(),
        config.ca_certificate.as_deref(),
    )?;
    let round: RoundInfo = client.get("/api/rounds/current").await?;
    if json {
        return format::print_json(&round);
    }
    print_round(&round);
    Ok(())
}

pub async fn problem(
    config: &Config,
    slug: &str,
    samples_dir: Option<&Path>,
    json: bool,
) -> Result<()> {
    let client = participant(config)?;
    let p: ProblemStatement = client.get(&format!("/api/problems/{slug}")).await?;
    if json {
        format::print_json(&p)?;
    } else {
        println!("# {}\n", p.title);
        println!(
            "Limits: {} ms CPU time, {} MiB memory\n",
            p.time_limit_ms, p.memory_limit_mb
        );
        println!("{}", p.statement.trim_end());
        for s in &p.samples {
            println!(
                "\n## Sample {}\n\nInput:\n{}\nOutput:\n{}",
                s.name, s.input, s.output
            );
        }
    }
    if let Some(dir) = samples_dir {
        std::fs::create_dir_all(dir)?;
        for s in &p.samples {
            std::fs::write(dir.join(format!("{}.in", s.name)), &s.input)?;
            std::fs::write(dir.join(format!("{}.out", s.name)), &s.output)?;
        }
        // On stderr with --json, so stdout stays one JSON document.
        if json {
            eprintln!("Samples written to {}.", dir.display());
        } else {
            println!("\nSamples written to {}.", dir.display());
        }
    }
    Ok(())
}

/// `.tar.gz` of a file or directory (skipping build output and VCS data).
pub fn pack_source(path: &Path) -> Result<Vec<u8>> {
    let bytes = pack(path, SKIP_DIRS)?;
    if bytes.len() > MAX_SOURCE {
        bail!(
            "the source archive is {} MiB; the limit is 16 MiB. Point --src at your source files only",
            bytes.len() / (1024 * 1024)
        );
    }
    Ok(bytes)
}

/// `.tar.gz` of a file, or of a directory's regular files without the `skip` directories.
pub fn pack(path: &Path, skip: &[&str]) -> Result<Vec<u8>> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);
    tar.follow_symlinks(false);
    let meta =
        std::fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    if meta.is_file() {
        let name = path.file_name().context("source has no file name")?;
        tar.append_path_with_name(path, name)?;
    } else {
        let mut stack = VecDeque::from([PathBuf::new()]);
        while let Some(rel) = stack.pop_front() {
            let mut entries: Vec<_> =
                std::fs::read_dir(path.join(&rel))?.collect::<Result<_, _>>()?;
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let name = entry.file_name();
                let rel_child = rel.join(&name);
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    // Only top-level build directories are skipped (`src/build/` is kept).
                    let skipped = (rel.as_os_str().is_empty() || name == ".git")
                        && skip.iter().any(|s| name == *s);
                    if !skipped {
                        stack.push_back(rel_child);
                    }
                } else if kind.is_file() {
                    tar.append_path_with_name(entry.path(), &rel_child)?;
                }
            }
        }
    }
    Ok(tar.into_inner()?.finish()?)
}

pub async fn submit(
    config: &Config,
    problem: &str,
    executable: &Path,
    source: &Path,
    wait: bool,
) -> Result<bool> {
    let client = participant(config)?;
    let exe = std::fs::read(executable)
        .with_context(|| format!("cannot read {}", executable.display()))?;
    let src = pack_source(source)?;

    // Listen before submitting so no progress event is missed.
    let mut events = if wait {
        client.events().await.ok()
    } else {
        None
    };

    let form = reqwest::multipart::Form::new()
        .text("problem", problem.to_owned())
        .part(
            "executable",
            reqwest::multipart::Part::bytes(exe).file_name("solution"),
        )
        .part(
            "source",
            reqwest::multipart::Part::bytes(src).file_name("source.tar.gz"),
        );
    let sub: SubmissionInfo = client.post_multipart("/api/submissions", form).await?;
    println!("Submission {} queued for {problem}.", sub.id);
    if !wait {
        return Ok(true);
    }

    let done = match events.as_mut() {
        // A broken live connection is not fatal: fall back to polling.
        Some(stream) => follow(&client, stream, sub.id).await.unwrap_or(None),
        None => None,
    };
    let done = match done {
        Some(d) => d,
        // No live connection (or it dropped): poll instead.
        None => poll(&client, sub.id).await?,
    };
    print_result(&done);
    Ok(done.verdict == Some(Verdict::Accepted))
}

/// Prints live progress until the result arrives. Checks over HTTP when the
/// connection goes quiet, in case the final event was lost.
async fn follow(
    client: &Client,
    stream: &mut crate::client::EventStream,
    id: i64,
) -> Result<Option<SubmissionInfo>> {
    const QUIET: std::time::Duration = std::time::Duration::from_secs(5);
    loop {
        let event = match tokio::time::timeout(QUIET, stream.next()).await {
            Ok(event) => event?,
            Err(_) => {
                let s: SubmissionInfo = client.get(&format!("/api/submissions/{id}")).await?;
                if matches!(s.status, SubmissionStatus::Done | SubmissionStatus::Failed) {
                    return Ok(Some(s));
                }
                continue;
            }
        };
        let Some(event) = event else {
            return Ok(None);
        };
        match event {
            ServerEvent::Progress {
                submission_id,
                test_index,
                test_count,
                result,
            } if submission_id == id => {
                println!(
                    "  [{}/{}] test {:<8} {:<22} {:>9} {:>9}",
                    test_index + 1,
                    test_count,
                    result.name,
                    format::verdict(result.verdict),
                    format::time(result.time_us),
                    format::memory(result.memory_kib),
                );
                std::io::stdout().flush().ok();
            }
            ServerEvent::Submission { submission } if submission.id == id => {
                return Ok(Some(submission));
            }
            _ => {}
        }
    }
}

async fn poll(client: &Client, id: i64) -> Result<SubmissionInfo> {
    loop {
        let s: SubmissionInfo = client.get(&format!("/api/submissions/{id}")).await?;
        if matches!(s.status, SubmissionStatus::Done | SubmissionStatus::Failed) {
            return Ok(s);
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

pub fn print_result(s: &SubmissionInfo) {
    let verdict = s.verdict.map(format::verdict).unwrap_or("not judged");
    match (s.verdict, s.time_us, s.memory_kib) {
        (Some(Verdict::Accepted), Some(t), Some(m)) => println!(
            "Submission {}: accepted — {} CPU, {} memory (slowest test, peak memory).",
            s.id,
            format::time(t),
            format::memory(m)
        ),
        _ => println!("Submission {}: {verdict}.", s.id),
    }
    if let Some(report) = &s.report {
        for t in report
            .tests
            .iter()
            .filter(|t| t.verdict != Verdict::Accepted)
        {
            let msg = t.message.as_deref().unwrap_or("");
            println!("  test {}: {} {msg}", t.name, format::verdict(t.verdict));
        }
    }
}

pub async fn submissions(config: &Config, id: Option<i64>, json: bool) -> Result<()> {
    let client = participant(config)?;
    if let Some(id) = id {
        let s: SubmissionInfo = client.get(&format!("/api/submissions/{id}")).await?;
        if json {
            return format::print_json(&s);
        }
        print_result(&s);
        return Ok(());
    }
    let list: Vec<SubmissionInfo> = client.get("/api/me/submissions").await?;
    if json {
        return format::print_json(&list);
    }
    if list.is_empty() {
        println!("No submissions in the current round.");
    }
    for s in list {
        let verdict = match s.status {
            SubmissionStatus::Queued => "queued",
            SubmissionStatus::Judging => "judging",
            _ => s.verdict.map(format::verdict).unwrap_or("?"),
        };
        let numbers = match (s.time_us, s.memory_kib) {
            (Some(t), Some(m)) if s.verdict == Some(Verdict::Accepted) => {
                format!("{} {}", format::time(t), format::memory(m))
            }
            _ => String::new(),
        };
        println!("{:>5}  {:<16} {:<22} {numbers}", s.id, s.problem, verdict);
    }
    Ok(())
}

/// Runs a build command in the build image, which matches the judge's environment.
pub fn build(image: &str, command: &[String]) -> Result<bool> {
    if command.is_empty() {
        return Err(UsageError(
            "give the build command after `--`, e.g. `arena build -- gcc -O2 -o sol sol.c`".into(),
        )
        .into());
    }
    let cwd = std::env::current_dir()?;
    let mut docker = std::process::Command::new("docker");
    docker
        .args(["run", "--rm", "--platform", "linux/amd64"])
        .arg("-v")
        .arg(format!("{}:/work", cwd.display()))
        .args(["-w", "/work"]);
    #[cfg(unix)]
    {
        // Build as you, so the output isn't owned by root. Asking `id` avoids the
        // unsafe libc calls this workspace forbids.
        if let (Some(uid), Some(gid)) = (id_flag("-u"), id_flag("-g")) {
            docker.args(["--user", &format!("{uid}:{gid}")]);
        }
    }
    let status = docker
        .arg(image)
        .args(command)
        .status()
        .context(DOCKER_UNAVAILABLE)?;
    Ok(status.success())
}

/// Downloads the build image for the platform `build` runs it on.
pub fn pull(image: &str) -> Result<bool> {
    let status = std::process::Command::new("docker")
        .args(["pull", "--platform", "linux/amd64"])
        .arg(image)
        .status()
        .context(DOCKER_UNAVAILABLE)?;
    Ok(status.success())
}

const DOCKER_UNAVAILABLE: &str = "cannot run Docker; is it installed and running? \
    For a first submission, use a #!/usr/bin/env python3 script instead";

#[cfg(unix)]
fn id_flag(flag: &str) -> Option<String> {
    let out = std::process::Command::new("id").arg(flag).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arena_protocol::{CacheInfo, JudgeSettings};

    #[test]
    fn specs_show_hardware_and_judging_rules() {
        let host = HostInfo {
            arch: "x86_64".into(),
            cpu_model: Some("AMD Ryzen 7 5800X 8-Core Processor".into()),
            cpus: Some(16),
            x86_64_level: Some(3),
            cpu_flags: vec!["avx2".into(), "bmi2".into()],
            caches: vec![
                CacheInfo {
                    level: 1,
                    kind: "data".into(),
                    size_kib: 32,
                },
                CacheInfo {
                    level: 3,
                    kind: "unified".into(),
                    size_kib: 32 * 1024,
                },
            ],
            memory_kib: Some(32 * 1024 * 1024),
            kernel: Some("6.8.0".into()),
            judge: JudgeSettings {
                workers: 2,
                runs_per_test: 3,
                processes: 64,
                max_failures: 0,
                microsecond_timing: true,
            },
        };
        let text = render_specs(&host, "https://arena.local");
        for expected in [
            "AMD Ryzen 7 5800X 8-Core Processor (16 logical CPUs, x86_64)",
            "-march=x86-64-v3",
            "avx2 bmi2",
            "L1d 32 KiB, L3 32 MiB",
            "32.0 GiB",
            "Linux 6.8.0",
            "2 submissions at a time",
            "best time",
            "microseconds",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
        assert!(!text.contains("stops after"), "{text}");

        let bare = HostInfo {
            cpu_model: None,
            cpus: None,
            x86_64_level: None,
            cpu_flags: vec![],
            caches: vec![],
            memory_kib: None,
            kernel: None,
            ..host
        };
        let text = render_specs(&bare, "https://arena.local");
        assert!(text.contains("unknown (x86_64)"), "{text}");
        assert!(
            !text.contains("Caches") && !text.contains("Memory"),
            "{text}"
        );
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
    fn admin_login_on_the_host_uses_dotenv() {
        let file = [("ARENA_ADMIN_TOKEN", "secret"), ("ARENA_PORT", "9000")];
        let got = admin_target(vars(&[]), vars(&file)).unwrap();
        assert_eq!(got, ("http://localhost:9000".into(), "secret".into()));
    }

    #[test]
    fn admin_login_reads_a_remote_server_from_dotenv() {
        let file = [
            ("ARENA_ADMIN_TOKEN", "secret"),
            ("ARENA_SERVER", "arena.example:8080/"),
        ];
        let got = admin_target(vars(&[]), vars(&file)).unwrap();
        assert_eq!(got, ("http://arena.example:8080".into(), "secret".into()));
    }

    #[test]
    fn admin_login_reads_server_and_token_from_environment() {
        let env = [
            ("ARENA_ADMIN_TOKEN", "from-env"),
            ("ARENA_SERVER", "https://arena.example"),
        ];
        let file = [("ARENA_ADMIN_TOKEN", "from-file")];
        let got = admin_target(vars(&env), vars(&file)).unwrap();
        assert_eq!(got, ("https://arena.example".into(), "from-env".into()));
    }

    #[test]
    fn dotenv_cannot_redirect_an_exported_token() {
        let env = [("ARENA_ADMIN_TOKEN", "from-env")];
        let file = [("ARENA_SERVER", "http://evil.example:8080")];
        let got = admin_target(vars(&env), vars(&file)).unwrap();
        assert_eq!(got, ("http://localhost:8080".into(), "from-env".into()));
    }

    #[test]
    fn environment_cannot_redirect_a_dotenv_token() {
        let env = [("ARENA_SERVER", "http://evil.example:8080")];
        let file = [("ARENA_ADMIN_TOKEN", "secret")];
        assert!(admin_target(vars(&env), vars(&file)).is_err());
        let env = [("ARENA_SERVER", "http://localhost:8080")];
        assert!(admin_target(vars(&env), vars(&file)).is_ok());
    }

    #[test]
    fn server_urls_are_checked_without_echoing_them() {
        for bad in [
            "http://user:hunter2@arena.example",
            "http://arena.example?x=hunter2",
            "ftp://hunter2",
        ] {
            let err = format!("{:#}", checked_server(bad).map(|_| ()).unwrap_err());
            assert!(!err.contains("hunter2"), "{err}");
        }
        assert!(!is_loopback("http://localhost.example:8080"));
        assert!(is_loopback("http://[::1]:8080"));
    }

    #[test]
    fn admin_login_without_a_token_fails() {
        assert!(admin_target(vars(&[]), vars(&[])).is_err());
    }

    #[test]
    fn dotenv_parsing() {
        let parsed = crate::config::parse_dotenv(
            "# comment\nA=1\nexport B=\"two words\"\nC='x' # note\nD=y # note\nEMPTY=\nnot a pair\n",
        );
        assert_eq!(parsed["A"], "1");
        assert_eq!(parsed["B"], "two words");
        assert_eq!(parsed["C"], "x");
        assert_eq!(parsed["D"], "y");
        assert_eq!(parsed.len(), 4);
    }

    #[test]
    fn source_archive_skips_build_dirs() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::create_dir_all(d.path().join("target/release")).unwrap();
        std::fs::write(d.path().join("target/release/big"), vec![0u8; 1000]).unwrap();
        std::fs::create_dir_all(d.path().join("src/util")).unwrap();
        std::fs::write(d.path().join("src/util/mod.rs"), "").unwrap();
        std::fs::create_dir_all(d.path().join("src/build")).unwrap();
        std::fs::write(d.path().join("src/build/gen.rs"), "").unwrap();

        let bytes = pack_source(d.path()).unwrap();
        let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..]));
        let mut names: Vec<String> = ar
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().display().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["main.rs", "src/build/gen.rs", "src/util/mod.rs"]);
    }

    #[test]
    fn single_file_source() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("sol.c");
        std::fs::write(&f, "int main(){}").unwrap();
        let bytes = pack_source(&f).unwrap();
        let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(&bytes[..]));
        let names: Vec<String> = ar
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().display().to_string())
            .collect();
        assert_eq!(names, ["sol.c"]);
    }
}
