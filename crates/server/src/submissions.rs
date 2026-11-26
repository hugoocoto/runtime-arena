//! Submission intake, lookups and admin review.

use std::path::{Path as FsPath, PathBuf};

use arena_protocol::{
    JudgeReport, QueuePosition, RoundState, SubmissionInfo, SubmissionStatus, Verdict,
};
use axum::Json;
use axum::body::Body;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::Response;
use serde::Deserialize;

use crate::auth::{Admin, Caller, Participant};
use crate::error::{ApiError, ApiResult};
use crate::{AppState, now};

const MAX_EXECUTABLE: usize = 64 * 1024 * 1024;
const MAX_SOURCE: usize = 16 * 1024 * 1024;
/// Where shebang interpreters are looked up; the sandbox uses the same PATH.
const SANDBOX_PATH: &[&str] = &["/usr/local/bin", "/usr/bin", "/bin"];

pub(crate) struct SubmissionRow {
    pub id: i64,
    pub user: String,
    pub round_id: i64,
    pub problem: String,
    pub problem_version: i64,
    pub status: String,
    pub verdict: Option<String>,
    pub time_us: Option<i64>,
    pub memory_kib: Option<i64>,
    pub report: Option<String>,
    pub submitted_at: i64,
    pub judged_at: Option<i64>,
    pub judging_started_at: Option<i64>,
}

impl SubmissionRow {
    pub fn info(&self) -> SubmissionInfo {
        let status = match self.status.as_str() {
            "queued" => SubmissionStatus::Queued,
            "judging" => SubmissionStatus::Judging,
            "done" => SubmissionStatus::Done,
            _ => SubmissionStatus::Failed,
        };
        SubmissionInfo {
            id: self.id,
            user: self.user.clone(),
            round_id: self.round_id,
            problem: self.problem.clone(),
            problem_version: self.problem_version,
            status,
            verdict: self.verdict.as_deref().and_then(parse_verdict),
            time_us: self.time_us.map(|v| v as u64),
            memory_kib: self.memory_kib.map(|v| v as u64),
            submitted_at: self.submitted_at,
            judged_at: self.judged_at,
            judging_started_at: self.judging_started_at,
            queue_position: None,
            report: self
                .report
                .as_deref()
                .and_then(|r| serde_json::from_str::<JudgeReport>(r).ok()),
        }
    }
}

pub(crate) fn verdict_str(v: Verdict) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn parse_verdict(s: &str) -> Option<Verdict> {
    serde_json::from_value(serde_json::Value::String(s.to_owned())).ok()
}

pub(crate) async fn find(state: &AppState, id: i64) -> ApiResult<(i64, SubmissionRow)> {
    let row = sqlx::query!(
        "SELECT s.id, s.user_id, u.name AS user, s.round_id, p.slug AS problem, s.problem_version,
                s.status, s.verdict, s.time_us, s.memory_kib, s.report, s.submitted_at, s.judged_at, s.judging_started_at
         FROM submissions s JOIN users u ON u.id = s.user_id JOIN problems p ON p.id = s.problem_id
         WHERE s.id = ?",
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found(format!("no submission {id}")))?;
    Ok((
        row.user_id,
        SubmissionRow {
            id: row.id,
            user: row.user,
            round_id: row.round_id,
            problem: row.problem,
            problem_version: row.problem_version,
            status: row.status,
            verdict: row.verdict,
            time_us: row.time_us,
            memory_kib: row.memory_kib,
            report: row.report,
            submitted_at: row.submitted_at,
            judged_at: row.judged_at,
            judging_started_at: row.judging_started_at,
        },
    ))
}

async fn list(
    state: &AppState,
    round_id: i64,
    user_id: Option<i64>,
) -> ApiResult<Vec<SubmissionInfo>> {
    let rows = sqlx::query_as!(
        SubmissionRow,
        "SELECT s.id, u.name AS user, s.round_id, p.slug AS problem, s.problem_version, s.status,
                s.verdict, s.time_us, s.memory_kib, s.report, s.submitted_at, s.judged_at, s.judging_started_at
         FROM submissions s JOIN users u ON u.id = s.user_id JOIN problems p ON p.id = s.problem_id
         WHERE s.round_id = ? AND (? IS NULL OR s.user_id = ?)
         ORDER BY s.id",
        round_id,
        user_id,
        user_id
    )
    .fetch_all(&state.db)
    .await?;
    Ok(rows.iter().map(SubmissionRow::info).collect())
}

pub(crate) fn dir(state: &AppState, id: i64) -> PathBuf {
    state
        .config
        .data_dir
        .join("submissions")
        .join(id.to_string())
}

pub(crate) fn executable_path(state: &AppState, id: i64) -> PathBuf {
    dir(state, id).join("solution")
}

/// `POST /api/submissions`, multipart with `problem` (slug), `executable` and `source`.
pub async fn create(
    user: Participant,
    State(state): State<AppState>,
    mut form: Multipart,
) -> ApiResult<(StatusCode, Json<SubmissionInfo>)> {
    let (mut problem, mut executable, mut source) = (None, None, None);
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
    {
        let name = field.name().unwrap_or_default().to_owned();
        let bytes = field
            .bytes()
            .await
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
        match name.as_str() {
            "problem" => problem = Some(String::from_utf8_lossy(&bytes).trim().to_owned()),
            "executable" => executable = Some(bytes),
            "source" => source = Some(bytes),
            _ => return Err(ApiError::bad_request(format!("unexpected field {name:?}"))),
        }
    }
    let problem = problem.ok_or_else(|| ApiError::bad_request("missing field problem"))?;
    let executable = executable.ok_or_else(|| ApiError::bad_request("missing field executable"))?;
    let source = source.filter(|s| !s.is_empty()).ok_or_else(|| {
        ApiError::bad_request("missing field source (your source code is required)")
    })?;
    if executable.len() > MAX_EXECUTABLE {
        return Err(ApiError::bad_request("executable is larger than 64 MiB"));
    }
    if source.len() > MAX_SOURCE {
        return Err(ApiError::bad_request("source is larger than 16 MiB"));
    }
    check_executable(&executable).map_err(ApiError::bad_request)?;

    // One intake at a time: the round, cooldown and insert checks must not interleave.
    let _guard = state.submit_lock.lock().await;
    let t = now();
    let round = crate::rounds::running(&state)
        .await?
        .ok_or_else(|| ApiError::conflict("no round is running"))?;
    debug_assert_eq!(round.state(t), RoundState::Running);
    let target = crate::rounds::problems(&state, round.id)
        .await?
        .into_iter()
        .find(|p| p.slug == problem)
        .ok_or_else(|| ApiError::not_found(format!("problem {problem} is not in this round")))?;

    let last = sqlx::query_scalar!(
        "SELECT MAX(submitted_at) FROM submissions WHERE user_id = ? AND round_id = ?",
        user.id,
        round.id
    )
    .fetch_one(&state.db)
    .await?;
    if let Some(last) = last {
        let wait = last + round.cooldown_s - t;
        if wait > 0 {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                format!("wait {wait} s before submitting again"),
            ));
        }
    }

    let mut tx = state.db.begin().await?;
    let id = sqlx::query_scalar!(
        "INSERT INTO submissions
            (user_id, round_id, problem_id, problem_version, problem_dir, status, submitted_at)
         VALUES (?, ?, ?, ?, ?, 'queued', ?) RETURNING id AS \"id!\"",
        user.id,
        round.id,
        target.id,
        target.version,
        target.dir,
        t
    )
    .fetch_one(&mut *tx)
    .await?;
    let load = crate::queue::status(&state, &mut tx, round.id, 1).await?;
    let sub_dir = dir(&state, id);
    write_files(&sub_dir, &executable, &source)
        .await
        .map_err(ApiError::internal)?;
    tx.commit().await?;
    drop(_guard);

    state.queue.notify_one();
    tracing::info!(submission = id, user = %user.name, problem = %problem, "submission queued");
    let (_, row) = find(&state, id).await?;
    let mut info = row.info();
    info.queue_position = Some(QueuePosition {
        ahead: load.queued,
        estimated_wait_seconds: load.estimated_wait_seconds,
    });
    Ok((StatusCode::CREATED, Json(info)))
}

async fn write_files(dir: &FsPath, executable: &[u8], source: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::create_dir_all(dir).await?;
    let exe = dir.join("solution");
    tokio::fs::write(&exe, executable).await?;
    tokio::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).await?;
    tokio::fs::write(dir.join("source"), source).await
}

/// Accepts a Linux x86-64 ELF executable, or a `#!` script whose interpreter exists here
/// (the judge runs in this same environment).
fn check_executable(bytes: &[u8]) -> Result<(), String> {
    const EM_X86_64: u16 = 62;
    if bytes.starts_with(b"\x7fELF") {
        if bytes.len() < 20 {
            return Err("truncated ELF file".into());
        }
        let (class, endian) = (bytes[4], bytes[5]);
        let machine = u16::from_le_bytes([bytes[18], bytes[19]]);
        if class != 2 || endian != 1 || machine != EM_X86_64 {
            return Err(
                "executable is not for Linux x86-64; build it in the build-env image".into(),
            );
        }
        return Ok(());
    }
    if let Some(rest) = bytes.strip_prefix(b"#!") {
        let line = rest.split(|b| *b == b'\n').next().unwrap_or_default();
        let line = String::from_utf8_lossy(line);
        let mut words = line.split_whitespace();
        let interpreter = words.next().ok_or("empty #! line")?;
        if !FsPath::new(interpreter).is_file() {
            return Err(format!(
                "interpreter {interpreter} is not installed on the judge"
            ));
        }
        if interpreter.ends_with("/env") {
            let program = words
                .find(|w| !w.starts_with('-'))
                .ok_or("#!/usr/bin/env needs a program name")?;
            let found = SANDBOX_PATH
                .iter()
                .any(|d| FsPath::new(d).join(program).is_file());
            if !found {
                return Err(format!("{program} is not installed on the judge"));
            }
        }
        return Ok(());
    }
    Err("not a Linux x86-64 executable or a #! script".into())
}

/// `GET /api/submissions/{id}`: your own submission, or any for the admin.
pub async fn get(
    caller: Caller,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<SubmissionInfo>> {
    let (owner, row) = find(&state, id).await?;
    if let Caller::Participant(p) = &caller
        && p.id != owner
    {
        return Err(ApiError::not_found(format!("no submission {id}")));
    }
    Ok(Json(row.info()))
}

#[derive(Deserialize)]
pub struct RoundQuery {
    round: Option<i64>,
}

async fn round_or_current(state: &AppState, round: Option<i64>) -> ApiResult<i64> {
    match round {
        Some(r) => Ok(r),
        None => crate::rounds::latest(state)
            .await?
            .map(|r| r.id)
            .ok_or_else(|| ApiError::not_found("no round has started yet")),
    }
}

/// `GET /api/me/submissions?round=ID`: your submissions (default: current round).
pub async fn mine(
    user: Participant,
    State(state): State<AppState>,
    Query(q): Query<RoundQuery>,
) -> ApiResult<Json<Vec<SubmissionInfo>>> {
    let round = round_or_current(&state, q.round).await?;
    Ok(Json(list(&state, round, Some(user.id)).await?))
}

/// `GET /api/admin/submissions?round=ID`: everyone's submissions.
pub async fn admin_list(
    _: Admin,
    State(state): State<AppState>,
    Query(q): Query<RoundQuery>,
) -> ApiResult<Json<Vec<SubmissionInfo>>> {
    let round = round_or_current(&state, q.round).await?;
    Ok(Json(list(&state, round, None).await?))
}

/// `GET /api/admin/submissions/{id}/source`: the uploaded source archive.
pub async fn source(
    _: Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    find(&state, id).await?;
    let bytes = tokio::fs::read(dir(&state, id).join("source"))
        .await
        .map_err(ApiError::internal)?;
    Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"submission-{id}-source\""),
        )
        .body(Body::from(bytes))
        .map_err(ApiError::internal)
}

/// `POST /api/admin/submissions/{id}/rejudge`: queues it again against the problem's
/// current version.
pub async fn rejudge(
    _: Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<SubmissionInfo>> {
    let updated = sqlx::query!(
        "UPDATE submissions SET status = 'queued', verdict = NULL, time_us = NULL,
            memory_kib = NULL, report = NULL, error = NULL, judged_at = NULL, judging_started_at = NULL,
            problem_version = (SELECT version FROM problems WHERE id = submissions.problem_id),
            problem_dir = (SELECT dir FROM problems WHERE id = submissions.problem_id)
         WHERE id = ? AND status IN ('done', 'failed')",
        id
    )
    .execute(&state.db)
    .await?;
    if updated.rows_affected() == 0 {
        find(&state, id).await?;
        return Err(ApiError::conflict(format!(
            "submission {id} is still being judged"
        )));
    }
    state.queue.notify_one();
    Ok(Json(find(&state, id).await?.1.info()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elf(class: u8, machine: u16) -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[..4].copy_from_slice(b"\x7fELF");
        b[4] = class;
        b[5] = 1;
        b[18..20].copy_from_slice(&machine.to_le_bytes());
        b
    }

    #[test]
    fn accepts_x86_64_elf_only() {
        assert!(check_executable(&elf(2, 62)).is_ok());
        assert!(check_executable(&elf(2, 183)).is_err(), "aarch64");
        assert!(check_executable(&elf(1, 3)).is_err(), "32-bit x86");
        assert!(check_executable(b"\x7fELF").is_err());
    }

    #[test]
    fn checks_shebang_interpreters() {
        assert!(check_executable(b"#!/bin/sh\necho hi\n").is_ok());
        assert!(check_executable(b"#!/usr/bin/env sh\n").is_ok());
        assert!(check_executable(b"#!/no/such/interpreter\n").is_err());
        assert!(check_executable(b"#!/usr/bin/env no-such-language-xyz\n").is_err());
    }

    #[test]
    fn rejects_other_files() {
        assert!(check_executable(b"int main() {}").is_err());
        assert!(check_executable(b"").is_err());
    }

    #[test]
    fn verdict_round_trips_through_text() {
        for v in [
            Verdict::Accepted,
            Verdict::TimeLimitExceeded,
            Verdict::JudgeError,
        ] {
            assert_eq!(parse_verdict(&verdict_str(v)), Some(v));
        }
    }
}
