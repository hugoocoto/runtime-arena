//! Commands for participants: login, round, problem, submit, submissions, build.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use arena_protocol::{
    ProblemStatement, RoundInfo, RoundState, ServerEvent, ServerInfo, SubmissionInfo,
    SubmissionStatus, UserInfo, Verdict,
};

use crate::client::Client;
use crate::config::Config;
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
    let token = config
        .token
        .clone()
        .context("not logged in; run `arena login <server> <token>`")?;
    Client::new(config.server()?, Some(token))
}

pub async fn login(server: &str, token: &str, admin: bool) -> Result<()> {
    let mut config = Config::load_file()?;
    if admin {
        let client = Client::new(server, Some(token.to_owned()))?;
        client
            .get::<Vec<UserInfo>>("/api/admin/users")
            .await
            .context("the admin token was refused")?;
        config.admin_token = Some(token.to_owned());
        println!("Admin token verified.");
    } else {
        let client = Client::new(server, Some(token.to_owned()))?;
        let me: UserInfo = client
            .get("/api/me")
            .await
            .context("the token was refused")?;
        config.token = Some(token.to_owned());
        println!("Logged in as {} ({}).", me.display_name, me.name);
    }
    config.server = Some(server.trim_end_matches('/').to_owned());
    let path = config.save()?;
    println!("Saved to {}.", path.display());
    Ok(())
}

pub async fn whoami(config: &Config) -> Result<()> {
    let client = participant(config)?;
    let info: ServerInfo = client.get("/api/info").await?;
    let me: UserInfo = client.get("/api/me").await?;
    println!("{} ({}) on {}", me.display_name, me.name, client.base());
    println!("server {} — source: {}", info.version, info.source_url);
    Ok(())
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

pub async fn round(config: &Config) -> Result<()> {
    let client = Client::new(config.server()?, config.token.clone())?;
    let round: RoundInfo = client.get("/api/rounds/current").await?;
    print_round(&round);
    Ok(())
}

pub async fn problem(config: &Config, slug: &str, samples_dir: Option<&Path>) -> Result<()> {
    let client = participant(config)?;
    let p: ProblemStatement = client.get(&format!("/api/problems/{slug}")).await?;
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
    if let Some(dir) = samples_dir {
        std::fs::create_dir_all(dir)?;
        for s in &p.samples {
            std::fs::write(dir.join(format!("{}.in", s.name)), &s.input)?;
            std::fs::write(dir.join(format!("{}.out", s.name)), &s.output)?;
        }
        println!("\nSamples written to {}.", dir.display());
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
                    format::time(result.time_ms),
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
    match (s.verdict, s.time_ms, s.memory_kib) {
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

pub async fn submissions(config: &Config, id: Option<i64>) -> Result<()> {
    let client = participant(config)?;
    if let Some(id) = id {
        let s: SubmissionInfo = client.get(&format!("/api/submissions/{id}")).await?;
        print_result(&s);
        return Ok(());
    }
    let list: Vec<SubmissionInfo> = client.get("/api/me/submissions").await?;
    if list.is_empty() {
        println!("No submissions in the current round.");
    }
    for s in list {
        let verdict = match s.status {
            SubmissionStatus::Queued => "queued",
            SubmissionStatus::Judging => "judging",
            _ => s.verdict.map(format::verdict).unwrap_or("?"),
        };
        let numbers = match (s.time_ms, s.memory_kib) {
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
        bail!("give the build command after `--`, e.g. `arena build -- gcc -O2 -o sol sol.c`");
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
        .context("cannot run docker; is it installed and running?")?;
    Ok(status.success())
}

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
