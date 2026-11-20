//! `arena admin ...`: users, problems, rounds and submission review.

use std::path::Path;

use anyhow::{Context, Result};
use arena_protocol::{
    CreateRound, CreateUser, ExtendRound, ProblemSummary, RoundInfo, SubmissionInfo, UserInfo,
    UserToken,
};
use serde_json::json;

use crate::client::Client;
use crate::config::Config;
use crate::format;
use crate::participant::{pack, print_result, print_round};

pub fn admin(config: &Config) -> Result<Client> {
    let token = config.admin_token.clone().context(
        "no admin token; run `arena login --admin <server> <token>` or set ARENA_ADMIN_TOKEN",
    )?;
    Client::new(config.server()?, Some(token))
}

pub async fn user_add(c: &Client, name: &str, display_name: Option<String>) -> Result<()> {
    let t: UserToken = c
        .post(
            "/api/admin/users",
            &CreateUser {
                name: name.to_owned(),
                display_name,
            },
        )
        .await?;
    print_token(c, &t);
    Ok(())
}

pub async fn user_reset(c: &Client, name: &str) -> Result<()> {
    let t: UserToken = c
        .post(&format!("/api/admin/users/{name}/token"), &json!({}))
        .await?;
    print_token(c, &t);
    Ok(())
}

fn print_token(c: &Client, t: &UserToken) {
    println!("Token for {} (shown only once):", t.name);
    println!("  {}", t.token);
    println!("They log in with:\n  arena login {} {}", c.base(), t.token);
}

pub async fn user_list(c: &Client) -> Result<()> {
    let users: Vec<UserInfo> = c.get("/api/admin/users").await?;
    for u in users {
        println!("{:<20} {}", u.name, u.display_name);
    }
    Ok(())
}

pub async fn problem_add(c: &Client, dir: &Path, slug: Option<&str>, rejudge: bool) -> Result<()> {
    let slug = match slug {
        Some(s) => s.to_owned(),
        None => dir
            .canonicalize()?
            .file_name()
            .context("cannot take the slug from the directory name; pass --slug")?
            .to_string_lossy()
            .into_owned(),
    };
    let archive = pack(dir, &[".git"])?;
    let query = if rejudge { "?rejudge=true" } else { "" };
    let p: ProblemSummary = c
        .put_bytes(&format!("/api/admin/problems/{slug}{query}"), archive)
        .await?;
    println!(
        "Problem {} \"{}\" version {}: {} tests, {} ms, {} MiB.",
        p.slug, p.title, p.version, p.test_count, p.time_limit_ms, p.memory_limit_mb
    );
    Ok(())
}

pub async fn problem_list(c: &Client) -> Result<()> {
    let problems: Vec<ProblemSummary> = c.get("/api/admin/problems").await?;
    for p in problems {
        println!(
            "{:<20} v{:<3} {:>3} tests  {:>5} ms {:>5} MiB  {}",
            p.slug, p.version, p.test_count, p.time_limit_ms, p.memory_limit_mb, p.title
        );
    }
    Ok(())
}

pub async fn round_create(c: &Client, req: CreateRound) -> Result<()> {
    let r: RoundInfo = c.post("/api/admin/rounds", &req).await?;
    print_round(&r);
    println!("Start it with: arena admin round start {}", r.id);
    Ok(())
}

pub async fn round_list(c: &Client) -> Result<()> {
    let rounds: Vec<RoundInfo> = c.get("/api/admin/rounds").await?;
    for r in rounds {
        print_round(&r);
    }
    Ok(())
}

pub async fn round_action(c: &Client, id: i64, action: &str, minutes: Option<u32>) -> Result<()> {
    let path = format!("/api/admin/rounds/{id}/{action}");
    let r: RoundInfo = match minutes {
        Some(m) => c.post(&path, &ExtendRound { minutes: m }).await?,
        None => c.post(&path, &json!({})).await?,
    };
    print_round(&r);
    Ok(())
}

pub async fn submissions(c: &Client, round: Option<i64>) -> Result<()> {
    let path = match round {
        Some(r) => format!("/api/admin/submissions?round={r}"),
        None => "/api/admin/submissions".to_owned(),
    };
    let list: Vec<SubmissionInfo> = c.get(&path).await?;
    for s in list {
        let verdict = s.verdict.map(format::verdict).unwrap_or("pending");
        let numbers = match (s.time_ms, s.memory_kib) {
            (Some(t), Some(m)) => format!("{} {}", format::time(t), format::memory(m)),
            _ => String::new(),
        };
        println!(
            "{:>5}  {:<16} {:<16} {:<22} {numbers}",
            s.id, s.user, s.problem, verdict
        );
    }
    Ok(())
}

pub async fn source(c: &Client, id: i64, out: &Path) -> Result<()> {
    let bytes = c
        .get_bytes(&format!("/api/admin/submissions/{id}/source"))
        .await?;
    std::fs::write(out, bytes)?;
    println!("Source of submission {id} written to {}.", out.display());
    Ok(())
}

pub async fn rejudge(c: &Client, id: i64) -> Result<()> {
    let s: SubmissionInfo = c
        .post(&format!("/api/admin/submissions/{id}/rejudge"), &json!({}))
        .await?;
    print_result(&s);
    Ok(())
}
