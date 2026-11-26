//! Rounds: a set of problems with a clock. One round runs at a time.

use arena_protocol::{CreateRound, ExtendRound, RoundInfo, RoundState};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;

use crate::auth::Admin;
use crate::error::{ApiError, ApiResult};
use crate::problems::ProblemRow;
use arena_protocol::ServerEvent;

use crate::{AppState, Event, now};

pub(crate) struct RoundRow {
    pub id: i64,
    pub name: String,
    pub duration_s: i64,
    pub freeze_s: i64,
    pub cooldown_s: i64,
    pub started_at: Option<i64>,
    pub ends_at: Option<i64>,
    pub freeze_at: Option<i64>,
}

impl RoundRow {
    pub fn state(&self, now: i64) -> RoundState {
        match (self.started_at, self.ends_at) {
            (None, _) => RoundState::Draft,
            (Some(_), Some(end)) if now < end => RoundState::Running,
            _ => RoundState::Finished,
        }
    }
}

pub(crate) async fn find_row(state: &AppState, id: i64) -> ApiResult<RoundRow> {
    sqlx::query_as!(
        RoundRow,
        "SELECT id, name, duration_s, freeze_s, cooldown_s, started_at, ends_at, freeze_at
         FROM rounds WHERE id = ?",
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found(format!("no round {id}")))
}

pub(crate) async fn problems(state: &AppState, round_id: i64) -> ApiResult<Vec<ProblemRow>> {
    Ok(sqlx::query_as!(
        ProblemRow,
        "SELECT p.id, p.slug, p.title, p.version, p.dir, p.time_limit_ms, p.memory_limit_kib,
                p.test_count
         FROM round_problems rp JOIN problems p ON p.id = rp.problem_id
         WHERE rp.round_id = ? ORDER BY rp.position",
        round_id
    )
    .fetch_all(&state.db)
    .await?)
}

async fn info(state: &AppState, round: RoundRow, show_problems: bool) -> ApiResult<RoundInfo> {
    let state_now = round.state(now());
    let problems = if show_problems || state_now != RoundState::Draft {
        problems(state, round.id)
            .await?
            .iter()
            .map(ProblemRow::summary)
            .collect()
    } else {
        Vec::new()
    };
    Ok(RoundInfo {
        id: round.id,
        name: round.name,
        state: state_now,
        duration_minutes: (round.duration_s / 60) as u32,
        freeze_minutes: (round.freeze_s / 60) as u32,
        cooldown_seconds: round.cooldown_s as u32,
        started_at: round.started_at,
        ends_at: round.ends_at,
        problems,
        server_time: now(),
    })
}

/// The most recently started round, running or not.
pub(crate) async fn latest(state: &AppState) -> ApiResult<Option<RoundRow>> {
    Ok(sqlx::query_as!(
        RoundRow,
        "SELECT id, name, duration_s, freeze_s, cooldown_s, started_at, ends_at, freeze_at
         FROM rounds WHERE started_at IS NOT NULL ORDER BY started_at DESC, id DESC LIMIT 1"
    )
    .fetch_optional(&state.db)
    .await?)
}

/// The round accepting submissions right now, if any.
pub(crate) async fn running(state: &AppState, t: i64) -> ApiResult<Option<RoundRow>> {
    Ok(latest(state)
        .await?
        .filter(|r| r.state(t) == RoundState::Running))
}

/// `GET /api/rounds/current`: the running round, else the most recently started one.
/// Public, so the projector page can show it.
pub async fn current(State(state): State<AppState>) -> ApiResult<Json<RoundInfo>> {
    let round = latest(&state)
        .await?
        .ok_or_else(|| ApiError::not_found("no round has started yet"))?;
    Ok(Json(info(&state, round, false).await?))
}

pub(crate) async fn current_info(state: &AppState) -> ApiResult<Option<RoundInfo>> {
    match latest(state).await? {
        Some(round) => Ok(Some(info(state, round, false).await?)),
        None => Ok(None),
    }
}

/// Tells everyone about the round and all its boards (after start, stop, extend,
/// the freeze and the end).
async fn announce(state: &AppState, id: i64) -> ApiResult<()> {
    let round = find_row(state, id).await?;
    let info = info(state, find_row(state, id).await?, false).await?;
    state.publish(Event::Public(ServerEvent::Round { round: info }));
    for problem in problems(state, id).await? {
        let board = crate::boards::board(state, &round, &problem, false).await?;
        state.publish(Event::Public(ServerEvent::Board { board }));
    }
    Ok(())
}

/// Announces the round when it freezes or ends on its own, which no request triggers.
pub(crate) async fn clock(state: AppState) {
    let mut last: Option<(i64, RoundState, bool)> = None;
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        tick.tick().await;
        let round = match latest(&state).await {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(_) => continue,
        };
        let t = now();
        let current = (
            round.id,
            round.state(t),
            crate::boards::is_frozen(&round, t),
        );
        if let Some(prev) = last
            && prev != current
            && prev.0 == current.0
            && announce(&state, round.id).await.is_err()
        {
            continue;
        }
        last = Some(current);
    }
}

/// `GET /api/admin/rounds`
pub async fn list(_: Admin, State(state): State<AppState>) -> ApiResult<Json<Vec<RoundInfo>>> {
    let rows = sqlx::query_as!(
        RoundRow,
        "SELECT id, name, duration_s, freeze_s, cooldown_s, started_at, ends_at, freeze_at
         FROM rounds ORDER BY id"
    )
    .fetch_all(&state.db)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(info(&state, row, true).await?);
    }
    Ok(Json(out))
}

/// `POST /api/admin/rounds`
pub async fn create(
    _: Admin,
    State(state): State<AppState>,
    Json(req): Json<CreateRound>,
) -> ApiResult<Json<RoundInfo>> {
    if req.name.trim().is_empty() {
        return Err(ApiError::bad_request("name is empty"));
    }
    if req.duration_minutes == 0 {
        return Err(ApiError::bad_request("duration_minutes must be positive"));
    }
    if req.freeze_minutes > req.duration_minutes {
        return Err(ApiError::bad_request(
            "freeze_minutes exceeds duration_minutes",
        ));
    }
    if req.problems.is_empty() {
        return Err(ApiError::bad_request("a round needs at least one problem"));
    }
    let mut problem_ids = Vec::new();
    for slug in &req.problems {
        let id = crate::problems::find(&state, slug).await?.id;
        if problem_ids.contains(&id) {
            return Err(ApiError::bad_request(format!(
                "problem {slug} listed twice"
            )));
        }
        problem_ids.push(id);
    }

    let name = req.name.trim();
    let duration_s = i64::from(req.duration_minutes) * 60;
    let freeze_s = i64::from(req.freeze_minutes) * 60;
    let cooldown_s = i64::from(req.cooldown_seconds);
    let created_at = now();
    let mut tx = state.db.begin().await?;
    let id = sqlx::query_scalar!(
        "INSERT INTO rounds (name, duration_s, freeze_s, cooldown_s, created_at)
         VALUES (?, ?, ?, ?, ?) RETURNING id",
        name,
        duration_s,
        freeze_s,
        cooldown_s,
        created_at
    )
    .fetch_one(&mut *tx)
    .await?;
    for (position, problem_id) in problem_ids.iter().enumerate() {
        let position = position as i64;
        sqlx::query!(
            "INSERT INTO round_problems (round_id, problem_id, position) VALUES (?, ?, ?)",
            id,
            problem_id,
            position
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(info(&state, find_row(&state, id).await?, true).await?))
}

/// `POST /api/admin/rounds/{id}/clone`: copy the settings and problems to a draft.
pub async fn clone_round(
    _: Admin,
    State(state): State<AppState>,
    Path(source_id): Path<i64>,
) -> ApiResult<Json<RoundInfo>> {
    let mut tx = state.db.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO rounds (name, duration_s, freeze_s, cooldown_s, created_at)
         SELECT name, duration_s, freeze_s, cooldown_s, ? FROM rounds WHERE id = ?
         RETURNING id",
    )
    .bind(now())
    .bind(source_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found(format!("no round {source_id}")))?;
    sqlx::query(
        "INSERT INTO round_problems (round_id, problem_id, position)
         SELECT ?, problem_id, position FROM round_problems WHERE round_id = ?",
    )
    .bind(id)
    .bind(source_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    tracing::info!(round = id, source_round = source_id, "round cloned");
    Ok(Json(info(&state, find_row(&state, id).await?, true).await?))
}

/// `DELETE /api/admin/rounds/{id}`: removes a draft or finished round.
pub async fn remove(
    _: Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    let round = find_row(&state, id).await?;
    if round.state(now()) == RoundState::Running {
        return Err(ApiError::conflict(format!("round {id} is still running")));
    }

    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM submissions
         WHERE round_id = ? AND status IN ('queued', 'judging')",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;
    if active > 0 {
        return Err(ApiError::conflict(
            "round still has submissions waiting to be judged",
        ));
    }

    let mut tx = state.db.begin().await?;
    let submission_ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM submissions WHERE round_id = ?")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
    sqlx::query("DELETE FROM submissions WHERE round_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM round_problems WHERE round_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM rounds WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // SQLite hands the ids of deleted rows out again once they are the highest,
    // so a new submission may get one of these ids as soon as this commits. Move
    // the files aside first: afterwards, `submissions/{id}` may be someone else's.
    let moved = move_aside(&state, &submission_ids).await;
    if let Err(e) = tx.commit().await {
        for (from, to) in &moved {
            if let Err(error) = tokio::fs::rename(to, from).await {
                tracing::error!(dir = %from.display(), %error, "could not restore submission files");
            }
        }
        return Err(e.into());
    }

    for (_, trash) in moved {
        if let Err(error) = tokio::fs::remove_dir_all(&trash).await {
            tracing::warn!(dir = %trash.display(), %error, "could not remove submission files");
        }
    }
    tracing::info!(round = id, "round removed");
    Ok(StatusCode::NO_CONTENT)
}

/// Renames each submission's directory to a name no submission uses, returning
/// (original, new) pairs. Missing directories are skipped.
async fn move_aside(
    state: &AppState,
    submission_ids: &[i64],
) -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
    let mut moved = Vec::new();
    for &submission_id in submission_ids {
        let from = crate::submissions::dir(state, submission_id);
        let to = from.with_file_name(format!(
            ".removed-{submission_id}-{}",
            crate::auth::new_token()
        ));
        match tokio::fs::rename(&from, &to).await {
            Ok(()) => moved.push((from, to)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(submission = submission_id, %error, "could not move submission files aside");
            }
        }
    }
    moved
}

/// `POST /api/admin/rounds/{id}/start`: starts the clock. Only one round may run at a time.
pub async fn start(
    _: Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<RoundInfo>> {
    // Keep the checked version stable until the round is marked as started.
    let uploads = state.upload_lock.lock().await;
    let round = find_row(&state, id).await?;
    if round.started_at.is_some() {
        return Err(ApiError::conflict(format!(
            "round {id} has already started"
        )));
    }
    let entries = problems(&state, id).await?;
    let preflight_state = state.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        for problem in entries {
            problem.validate_public_material(&preflight_state)?;
        }
        Ok(())
    })
    .await
    .map_err(ApiError::internal)??;

    let t = now();
    let mut tx = state.db.begin().await?;
    let running = sqlx::query_scalar!(
        "SELECT id FROM rounds WHERE started_at IS NOT NULL AND ends_at > ? AND id != ?",
        t,
        id
    )
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(other) = running {
        return Err(ApiError::conflict(format!(
            "round {other} is still running"
        )));
    }
    let updated = sqlx::query!(
        "UPDATE rounds SET started_at = ?, ends_at = ? + duration_s,
            freeze_at = CASE WHEN freeze_s > 0 THEN ? + duration_s - freeze_s END
         WHERE id = ? AND started_at IS NULL",
        t,
        t,
        t,
        id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    drop(uploads);
    if updated.rows_affected() == 0 {
        let round = find_row(&state, id).await?;
        return Err(ApiError::conflict(format!(
            "round {} has already started",
            round.id
        )));
    }
    tracing::info!(round = id, "round started");
    announce(&state, id).await?;
    Ok(Json(info(&state, find_row(&state, id).await?, true).await?))
}

/// `POST /api/admin/rounds/{id}/stop`: ends a running round now.
pub async fn stop(
    _: Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> ApiResult<Json<RoundInfo>> {
    let t = now();
    let updated = sqlx::query!(
        "UPDATE rounds SET ends_at = ? WHERE id = ? AND started_at IS NOT NULL AND ends_at > ?",
        t,
        id,
        t
    )
    .execute(&state.db)
    .await?;
    if updated.rows_affected() == 0 {
        find_row(&state, id).await?;
        return Err(ApiError::conflict(format!("round {id} is not running")));
    }
    tracing::info!(round = id, "round stopped");
    announce(&state, id).await?;
    Ok(Json(info(&state, find_row(&state, id).await?, true).await?))
}

/// `POST /api/admin/rounds/{id}/extend`: moves the end of a running round.
pub async fn extend(
    _: Admin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<ExtendRound>,
) -> ApiResult<Json<RoundInfo>> {
    if req.minutes == 0 {
        return Err(ApiError::bad_request("minutes must be positive"));
    }
    let t = now();
    let extra = i64::from(req.minutes) * 60;
    let updated = sqlx::query!(
        "UPDATE rounds SET ends_at = ends_at + ?, duration_s = duration_s + ?,
            -- Not frozen yet: the freeze moves with the end. Already frozen: it stays.
            freeze_at = CASE WHEN freeze_at > ? THEN freeze_at + ? ELSE freeze_at END
         WHERE id = ? AND started_at IS NOT NULL AND ends_at > ?",
        extra,
        extra,
        t,
        extra,
        id,
        t
    )
    .execute(&state.db)
    .await?;
    if updated.rows_affected() == 0 {
        find_row(&state, id).await?;
        return Err(ApiError::conflict(format!("round {id} is not running")));
    }
    tracing::info!(round = id, minutes = req.minutes, "round extended");
    announce(&state, id).await?;
    Ok(Json(info(&state, find_row(&state, id).await?, true).await?))
}
