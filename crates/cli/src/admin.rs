//! `arena admin ...`: participants, problems, rounds and submission review.

use std::path::Path;

use anyhow::{Context, Result};
use arena_protocol::{
    CreateRound, ExtendRound, PasswordReset, ProblemSummary, RoundInfo, SubmissionInfo, UpdateInfo,
    UserInfo,
};
use serde_json::json;

use crate::client::Client;
use crate::config::Config;
use crate::errors::{NotLoggedIn, UsageError, status_of};
use crate::format;
use crate::participant::{pack, print_result, print_round};

pub fn admin(config: &Config) -> Result<Client> {
    let token = config.admin_token.clone().ok_or_else(|| {
        NotLoggedIn(
            "no admin token; run `arena login --admin` or set ARENA_SERVER and ARENA_ADMIN_TOKEN"
                .into(),
        )
    })?;
    Client::with_ca(
        config.server()?,
        Some(token),
        config.ca_certificate.as_deref(),
    )
}

const UPGRADE_HINT: &str = "Update the host with: docker compose pull && docker compose up -d\n\
     (if ARENA_VERSION in .env is pinned, change it first)";

/// `arena admin update`: whether a newer server release exists.
pub async fn update(c: &Client) -> Result<()> {
    let info: UpdateInfo = match c.get("/api/admin/update").await {
        Err(e) if status_of(&e) == Some(reqwest::StatusCode::NOT_FOUND) => {
            return Err(e.context("this server is too old to report updates"));
        }
        other => other?,
    };
    println!("Server version: {}", info.current);
    match &info.latest {
        Some(latest) => println!("Latest release: {latest}"),
        None => println!("Latest release: unknown (no successful check yet)"),
    }
    if info.update_available {
        print_update(&info);
    } else if info.latest.is_some() {
        println!("Up to date.");
    }
    Ok(())
}

/// After an admin login: mentions an update, and stays quiet otherwise.
pub async fn notify_update(c: &Client) {
    if let Ok(info) = c.get::<UpdateInfo>("/api/admin/update").await
        && info.update_available
    {
        print_update(&info);
    }
}

fn print_update(info: &UpdateInfo) {
    let latest = info.latest.as_deref().unwrap_or("?");
    println!("Update available: {} -> {latest}", info.current);
    if let Some(url) = &info.release_url {
        println!("{url}");
    }
    println!("{UPGRADE_HINT}");
}

pub async fn user_list(c: &Client, json: bool) -> Result<()> {
    let users: Vec<UserInfo> = c.get("/api/admin/users").await?;
    if json {
        return format::print_json(&users);
    }
    for u in users {
        println!("{:<20} {}", u.name, u.display_name);
    }
    Ok(())
}

/// Checked here because the name becomes part of the URL: `a?b` would reset `a`.
fn checked_user_name(name: &str) -> Result<String> {
    let name = name.trim().to_ascii_lowercase();
    let valid = (1..=40).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !valid {
        return Err(
            UsageError("a username is 1-40 lowercase letters, digits or '-'".into()).into(),
        );
    }
    Ok(name)
}

pub async fn reset_password(c: &Client, name: &str) -> Result<()> {
    let name = checked_user_name(name)?;
    let r: PasswordReset = c
        .post(&format!("/api/admin/users/{name}/password"), &json!({}))
        .await?;
    println!("New password for {} (shown only once):", r.name);
    println!("  {}", r.password);
    println!("All their devices were logged out. They log in with:");
    println!("  arena login {}", c.base());
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

pub async fn problem_list(c: &Client, json: bool) -> Result<()> {
    let problems: Vec<ProblemSummary> = c.get("/api/admin/problems").await?;
    if json {
        return format::print_json(&problems);
    }
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

pub async fn round_list(c: &Client, json: bool) -> Result<()> {
    let rounds: Vec<RoundInfo> = c.get("/api/admin/rounds").await?;
    if json {
        return format::print_json(&rounds);
    }
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

pub async fn round_delete(c: &Client, id: i64) -> Result<()> {
    c.delete_empty(&format!("/api/admin/rounds/{id}")).await?;
    println!("Round {id} removed.");
    Ok(())
}

pub async fn submissions(c: &Client, round: Option<i64>, json: bool) -> Result<()> {
    let path = match round {
        Some(r) => format!("/api/admin/submissions?round={r}"),
        None => "/api/admin/submissions".to_owned(),
    };
    let list: Vec<SubmissionInfo> = c.get(&path).await?;
    if json {
        return format::print_json(&list);
    }
    for s in list {
        let verdict = s.verdict.map(format::verdict).unwrap_or("pending");
        let numbers = match (s.time_us, s.memory_kib) {
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

#[cfg(test)]
mod tests {
    use super::checked_user_name;

    #[test]
    fn user_names_cannot_change_the_url() {
        assert_eq!(checked_user_name(" Alice-1 ").unwrap(), "alice-1");
        for bad in ["a?b", "a/b", "../x", "a#b", ""] {
            assert!(checked_user_name(bad).is_err(), "{bad}");
        }
    }
}
