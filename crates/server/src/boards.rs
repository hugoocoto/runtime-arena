//! Per-problem leaderboards, ranked by CPU time and by memory.

use std::collections::HashMap;

use arena_protocol::{AcceptedSubmission, Board, BoardEntry};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;

use crate::error::{ApiError, ApiResult};
use crate::problems::ProblemRow;
use crate::rounds::RoundRow;
use crate::{AppState, now};

/// When submissions stop counting on public boards (`None` if the round has no freeze).
fn freeze_at(round: &RoundRow) -> Option<i64> {
    round.freeze_at
}

/// True between the freeze and the end; boards show everything again once the round ends.
pub(crate) fn is_frozen(round: &RoundRow, t: i64) -> bool {
    match (freeze_at(round), round.ends_at) {
        (Some(freeze), Some(end)) => t >= freeze && t < end,
        _ => false,
    }
}

struct Accepted {
    submission_id: i64,
    user: String,
    display_name: String,
    time_us: i64,
    memory_kib: i64,
    submitted_at: i64,
}

async fn accepted(
    state: &AppState,
    round_id: i64,
    problem_id: i64,
    cutoff: i64,
) -> ApiResult<Vec<Accepted>> {
    Ok(sqlx::query_as!(
        Accepted,
        r#"SELECT s.id AS submission_id, u.name AS user, u.display_name,
                  s.time_us AS "time_us!", s.memory_kib AS "memory_kib!", s.submitted_at
           FROM submissions s JOIN users u ON u.id = s.user_id
           WHERE s.round_id = ? AND s.problem_id = ? AND s.status = 'done'
             AND s.verdict = 'accepted' AND s.submitted_at < ?
           ORDER BY s.submitted_at, s.id"#,
        round_id,
        problem_id,
        cutoff
    )
    .fetch_all(&state.db)
    .await?)
}

/// Submissions after the freeze are hidden from `frozen` views.
fn cutoff(round: &RoundRow, frozen: bool) -> i64 {
    if frozen {
        freeze_at(round).unwrap_or(i64::MAX)
    } else {
        i64::MAX
    }
}

/// Builds both boards. `live` ignores the freeze (admin view).
pub(crate) async fn board(
    state: &AppState,
    round: &RoundRow,
    problem: &ProblemRow,
    live: bool,
) -> ApiResult<Board> {
    let frozen = !live && is_frozen(round, now());
    let cutoff = cutoff(round, frozen);

    let accepted = accepted(state, round.id, problem.id, cutoff).await?;
    let attempts: HashMap<String, i64> = sqlx::query!(
        r#"SELECT u.name AS user, COUNT(*) AS "n!: i64"
           FROM submissions s JOIN users u ON u.id = s.user_id
           WHERE s.round_id = ? AND s.problem_id = ? AND s.submitted_at < ?
           GROUP BY u.name"#,
        round.id,
        problem.id,
        cutoff
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|r| (r.user, r.n))
    .collect();

    let by_time = rank(&accepted, &attempts, |a| {
        (a.time_us, a.memory_kib, a.submitted_at)
    });
    let by_memory = rank(&accepted, &attempts, |a| {
        (a.memory_kib, a.time_us, a.submitted_at)
    });
    Ok(Board {
        round_id: round.id,
        problem: problem.slug.clone(),
        frozen,
        by_time,
        by_memory,
    })
}

/// Each participant's best accepted submission under `key` (smaller is better), ranked.
fn rank(
    accepted: &[Accepted],
    attempts: &HashMap<String, i64>,
    key: impl Fn(&Accepted) -> (i64, i64, i64),
) -> Vec<BoardEntry> {
    let mut best: HashMap<&str, &Accepted> = HashMap::new();
    for a in accepted {
        let e = best.entry(&a.user).or_insert(a);
        if (key(a), a.submission_id) < (key(e), e.submission_id) {
            *e = a;
        }
    }
    let mut rows: Vec<&Accepted> = best.into_values().collect();
    rows.sort_by_key(|a| (key(a), a.submission_id));
    rows.into_iter()
        .enumerate()
        .map(|(i, a)| BoardEntry {
            rank: i as u32 + 1,
            user: a.user.clone(),
            display_name: a.display_name.clone(),
            time_us: a.time_us as u64,
            memory_kib: a.memory_kib as u64,
            submission_id: a.submission_id,
            submitted_at: a.submitted_at,
            attempts: attempts.get(&a.user).copied().unwrap_or(0) as u32,
        })
        .collect()
}

async fn boards_for_round(state: &AppState, round: &RoundRow, live: bool) -> ApiResult<Vec<Board>> {
    let mut boards = Vec::new();
    for problem in crate::rounds::problems(state, round.id).await? {
        boards.push(board(state, round, &problem, live).await?);
    }
    Ok(boards)
}

/// `GET /api/rounds/{id}/boards`: public (frozen near the end); live for the admin.
pub async fn round_boards(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Board>>> {
    let live = crate::auth::is_admin(&headers, &state);
    let round = crate::rounds::find_row(&state, id).await?;
    if round.started_at.is_none() && !live {
        return Err(ApiError::not_found(format!("round {id} has not started")));
    }
    Ok(Json(boards_for_round(&state, &round, live).await?))
}

/// `GET /api/rounds/current/boards`: public (frozen near the end); live for the admin.
pub async fn current_boards(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<Board>>> {
    let live = crate::auth::is_admin(&headers, &state);
    let round = crate::rounds::latest(&state)
        .await?
        .ok_or_else(|| ApiError::not_found("no round has started yet"))?;
    Ok(Json(boards_for_round(&state, &round, live).await?))
}

/// `GET /api/rounds/{id}/problems/{slug}/accepted`: every accepted submission in
/// order (frozen like the boards), so clients can chart ranks over time.
pub async fn accepted_history(
    State(state): State<AppState>,
    Path((id, slug)): Path<(i64, String)>,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<AcceptedSubmission>>> {
    let live = crate::auth::is_admin(&headers, &state);
    let round = crate::rounds::find_row(&state, id).await?;
    if round.started_at.is_none() && !live {
        return Err(ApiError::not_found(format!("round {id} has not started")));
    }
    let problem = crate::rounds::problems(&state, id)
        .await?
        .into_iter()
        .find(|p| p.slug == slug)
        .ok_or_else(|| ApiError::not_found(format!("problem {slug} is not in round {id}")))?;
    let frozen = !live && is_frozen(&round, now());
    let rows = accepted(&state, id, problem.id, cutoff(&round, frozen)).await?;
    Ok(Json(
        rows.into_iter()
            .map(|a| AcceptedSubmission {
                submission_id: a.submission_id,
                user: a.user,
                display_name: a.display_name,
                time_us: a.time_us as u64,
                memory_kib: a.memory_kib as u64,
                submitted_at: a.submitted_at,
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(id: i64, user: &str, time_us: i64, memory_kib: i64, at: i64) -> Accepted {
        Accepted {
            submission_id: id,
            user: user.into(),
            display_name: user.into(),
            time_us,
            memory_kib,
            submitted_at: at,
        }
    }

    #[test]
    fn best_per_user_and_both_orders() {
        let subs = vec![
            acc(1, "ann", 50, 900, 10),
            acc(2, "ann", 30, 1200, 20), // ann's fastest
            acc(3, "bob", 40, 500, 15),
            acc(4, "cid", 30, 800, 25), // ties ann on time, less memory
        ];
        let attempts = HashMap::from([("ann".to_owned(), 3), ("bob".to_owned(), 1)]);

        let t = rank(&subs, &attempts, |a| {
            (a.time_us, a.memory_kib, a.submitted_at)
        });
        let order: Vec<_> = t
            .iter()
            .map(|e| (e.rank, e.user.as_str(), e.submission_id))
            .collect();
        assert_eq!(order, [(1, "cid", 4), (2, "ann", 2), (3, "bob", 3)]);
        assert_eq!(t[1].attempts, 3);

        let m = rank(&subs, &attempts, |a| {
            (a.memory_kib, a.time_us, a.submitted_at)
        });
        let order: Vec<_> = m
            .iter()
            .map(|e| (e.user.as_str(), e.submission_id))
            .collect();
        assert_eq!(order, [("bob", 3), ("cid", 4), ("ann", 1)]);
    }

    fn round(freeze_s: i64, ends_at: Option<i64>) -> RoundRow {
        RoundRow {
            id: 1,
            name: "r".into(),
            duration_s: 3600,
            freeze_s,
            cooldown_s: 0,
            started_at: ends_at.map(|e| e - 3600),
            ends_at,
            freeze_at: ends_at.filter(|_| freeze_s > 0).map(|e| e - freeze_s),
        }
    }

    #[test]
    fn freeze_window() {
        let r = round(600, Some(10_000));
        assert!(!is_frozen(&r, 9_399));
        assert!(is_frozen(&r, 9_400));
        assert!(!is_frozen(&r, 10_000), "unfrozen once the round ends");
        assert!(!is_frozen(&round(0, Some(10_000)), 9_999));
        assert!(!is_frozen(&round(600, None), 0));
    }
}
