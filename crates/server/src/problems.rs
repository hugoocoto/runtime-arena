//! Problem upload (admin) and statements (participants).

use std::io::Read;
use std::path::{Path as FsPath, PathBuf};

use arena_judge::Problem;
use arena_protocol::{ProblemStatement, ProblemSummary, Sample};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};

use crate::auth::{Admin, Caller};
use crate::error::{ApiError, ApiResult};
use crate::{AppState, now, valid_slug};

/// Largest sample file shown in a statement.
const MAX_SAMPLE_BYTES: u64 = 64 * 1024;
/// Largest unpacked problem, so a small archive can't fill the disk.
const MAX_UNPACKED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub(crate) struct ProblemRow {
    pub id: i64,
    pub slug: String,
    pub title: String,
    pub version: i64,
    pub dir: String,
    pub time_limit_ms: i64,
    pub memory_limit_kib: i64,
    pub test_count: i64,
}

impl ProblemRow {
    pub fn summary(&self) -> ProblemSummary {
        ProblemSummary {
            slug: self.slug.clone(),
            title: self.title.clone(),
            version: self.version,
            time_limit_ms: self.time_limit_ms as u64,
            memory_limit_mb: self.memory_limit_kib as u64 / 1024,
            test_count: self.test_count as u64,
        }
    }

    /// Loads the judge's view of this problem version from disk.
    pub fn load(&self) -> Result<Problem, ApiError> {
        Problem::load(&self.dir).map_err(ApiError::internal)
    }
}

pub(crate) async fn find(state: &AppState, slug: &str) -> ApiResult<ProblemRow> {
    sqlx::query_as!(
        ProblemRow,
        "SELECT id, slug, title, version, dir, time_limit_ms, memory_limit_kib, test_count
         FROM problems WHERE slug = ?",
        slug
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found(format!("no problem {slug}")))
}

/// `GET /api/admin/problems`
pub async fn list(_: Admin, State(state): State<AppState>) -> ApiResult<Json<Vec<ProblemSummary>>> {
    let rows = sqlx::query_as!(
        ProblemRow,
        "SELECT id, slug, title, version, dir, time_limit_ms, memory_limit_kib, test_count
         FROM problems ORDER BY slug"
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(ProblemRow::summary).collect()))
}

/// `PUT /api/admin/problems/{slug}` with a `.tar.gz` of the problem directory as body.
///
/// Creates the problem or a new version of it. Refused once a started round uses it.
#[derive(serde::Deserialize)]
pub struct UploadQuery {
    /// Allow replacing a problem of a started round, and rejudge its submissions.
    #[serde(default)]
    rejudge: bool,
}

pub async fn upload(
    _: Admin,
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> ApiResult<Json<ProblemSummary>> {
    if !valid_slug(&slug) {
        return Err(ApiError::bad_request(
            "slug must be 1-40 lowercase letters, digits or '-'",
        ));
    }
    let _guard = state.upload_lock.lock().await;
    let locked = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM round_problems rp
         JOIN rounds r ON r.id = rp.round_id
         JOIN problems p ON p.id = rp.problem_id
         WHERE p.slug = ? AND r.started_at IS NOT NULL",
        slug
    )
    .fetch_one(&state.db)
    .await?;
    if locked > 0 && !q.rejudge {
        return Err(ApiError::conflict(format!(
            "problem {slug} is used by a round that has started; upload with \
             ?rejudge=true to replace it and rejudge its submissions"
        )));
    }

    let current: Option<i64> =
        sqlx::query_scalar!("SELECT version FROM problems WHERE slug = ?", slug)
            .fetch_optional(&state.db)
            .await?;
    let version = current.unwrap_or(0) + 1;
    let dir = state
        .config
        .data_dir
        .join("problems")
        .join(&slug)
        .join(format!("v{version}"));

    let problem = {
        let dir = dir.clone();
        tokio::task::spawn_blocking(move || unpack_problem(&body, &dir))
            .await
            .map_err(ApiError::internal)?
    };
    let problem = match problem {
        Ok(p) => p,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
    };

    let dir_str = problem.dir.to_string_lossy().into_owned();
    let time_limit_ms = problem.time_limit.as_millis() as i64;
    let memory_limit_kib = problem.memory_limit_kib as i64;
    let test_count = problem.tests.len() as i64;
    let updated_at = now();
    sqlx::query!(
        "INSERT INTO problems
            (slug, title, version, dir, time_limit_ms, memory_limit_kib, test_count, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (slug) DO UPDATE SET
            title = excluded.title, version = excluded.version, dir = excluded.dir,
            time_limit_ms = excluded.time_limit_ms, memory_limit_kib = excluded.memory_limit_kib,
            test_count = excluded.test_count, updated_at = excluded.updated_at",
        slug,
        problem.title,
        version,
        dir_str,
        time_limit_ms,
        memory_limit_kib,
        test_count,
        updated_at
    )
    .execute(&state.db)
    .await?;
    tracing::info!(%slug, version, tests = test_count, "problem uploaded");
    if q.rejudge {
        let requeued = sqlx::query!(
            "UPDATE submissions SET status = 'queued', verdict = NULL, time_us = NULL,
                memory_kib = NULL, report = NULL, error = NULL, judged_at = NULL,
                problem_version = ?, problem_dir = ?
             WHERE problem_id = (SELECT id FROM problems WHERE slug = ?)",
            version,
            dir_str,
            slug
        )
        .execute(&state.db)
        .await?
        .rows_affected();
        // Queued ones now point at the new version too; runs in progress are
        // dropped when they finish (see queue::record) and judged again.
        tracing::info!(%slug, requeued, "rejudging after problem change");
        for _ in 0..requeued.min(64) {
            state.queue.notify_one();
        }
        // The cleared verdicts take entries off the boards until rejudged.
        let rounds = sqlx::query!(
            "SELECT rp.round_id, rp.problem_id FROM round_problems rp
             JOIN problems p ON p.id = rp.problem_id WHERE p.slug = ?",
            slug
        )
        .fetch_all(&state.db)
        .await?;
        for r in rounds {
            crate::queue::publish_board(&state, r.round_id, r.problem_id).await;
        }
    }
    Ok(Json(find(&state, &slug).await?.summary()))
}

/// Unpacks a `.tar.gz` into `dir` and validates it as a problem.
///
/// Only regular files and directories are accepted (no links), and tar refuses
/// paths that leave `dir`. `problem.toml` may sit at the top or inside a single
/// top-level directory.
fn unpack_problem(archive: &[u8], dir: &FsPath) -> Result<Problem, ApiError> {
    std::fs::create_dir_all(dir).map_err(ApiError::internal)?;
    let bad = |msg: String| ApiError::bad_request(format!("invalid problem archive: {msg}"));

    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let entries = tar.entries().map_err(|e| bad(e.to_string()))?;
    let mut total = 0u64;
    for entry in entries {
        let mut entry = entry.map_err(|e| bad(e.to_string()))?;
        total = total.saturating_add(entry.header().size().unwrap_or(u64::MAX));
        if total > MAX_UNPACKED_BYTES {
            return Err(bad("larger than 2 GiB unpacked".into()));
        }
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            let path = entry
                .path()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            return Err(bad(format!(
                "{path}: only files and directories are allowed"
            )));
        }
        let unpacked = entry.unpack_in(dir).map_err(|e| bad(e.to_string()))?;
        if !unpacked {
            return Err(bad("entry outside the problem directory".into()));
        }
    }

    let root = find_root(dir).ok_or_else(|| bad("no problem.toml found".into()))?;
    Problem::load(&root).map_err(|e| ApiError::bad_request(e.to_string()))
}

fn find_root(dir: &FsPath) -> Option<PathBuf> {
    if dir.join("problem.toml").is_file() {
        return Some(dir.to_path_buf());
    }
    let mut subdirs = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir());
    let only = subdirs.next()?;
    if subdirs.next().is_some() {
        return None;
    }
    let root = only.path();
    root.join("problem.toml").is_file().then_some(root)
}

/// `GET /api/problems/{slug}`: statement and samples, for problems of a started round.
pub async fn statement(
    caller: Caller,
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<ProblemStatement>> {
    let row = find(&state, &slug).await?;
    if matches!(caller, Caller::Participant(_)) {
        let visible = sqlx::query_scalar!(
            "SELECT COUNT(*) FROM round_problems rp JOIN rounds r ON r.id = rp.round_id
             WHERE rp.problem_id = ? AND r.started_at IS NOT NULL",
            row.id
        )
        .fetch_one(&state.db)
        .await?;
        if visible == 0 {
            // Same answer as for an unknown problem, so participants can't probe slugs.
            return Err(ApiError::not_found(format!("no problem {slug}")));
        }
    }

    let problem = row.load()?;
    let statement = std::fs::read_to_string(problem.dir.join("statement.md")).unwrap_or_default();
    let mut samples = Vec::new();
    for name in &problem.samples {
        let test = problem
            .tests
            .iter()
            .find(|t| &t.name == name)
            .ok_or_else(|| ApiError::internal(format!("sample {name} missing in {slug}")))?;
        samples.push(Sample {
            name: name.clone(),
            input: read_capped(&test.input)?,
            output: read_capped(&test.answer)?,
        });
    }
    Ok(Json(ProblemStatement {
        slug: row.slug.clone(),
        title: row.title.clone(),
        time_limit_ms: row.time_limit_ms as u64,
        memory_limit_mb: row.memory_limit_kib as u64 / 1024,
        statement,
        samples,
    }))
}

fn read_capped(path: &FsPath) -> ApiResult<String> {
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_SAMPLE_BYTES).read_to_end(&mut buf))
        .map_err(ApiError::internal)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}
