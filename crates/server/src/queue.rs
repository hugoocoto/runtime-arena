//! Judge workers: each owns one isolate box and takes queued submissions in order.

use std::time::Duration;

use arena_judge::Problem;
use arena_protocol::{JudgeReport, ServerEvent, Verdict};

use crate::problems::resolve_dir;
use crate::submissions::{executable_path, verdict_str};
use crate::{AppState, Event, now};

/// Fallback wake-up in case a notification is missed.
const IDLE_POLL: Duration = Duration::from_secs(5);

struct Job {
    id: i64,
    user_id: i64,
    round_id: i64,
    problem_id: i64,
    problem_version: i64,
    problem_dir: String,
}

/// Submissions left `judging` by a crash or restart go back to the queue.
pub(crate) async fn requeue_interrupted(state: &AppState) -> Result<(), sqlx::Error> {
    let n = sqlx::query!("UPDATE submissions SET status = 'queued' WHERE status = 'judging'")
        .execute(&state.db)
        .await?
        .rows_affected();
    if n > 0 {
        tracing::warn!(count = n, "requeued submissions interrupted by a restart");
    }
    Ok(())
}

/// Atomically takes the oldest queued submission.
async fn claim(state: &AppState) -> Result<Option<Job>, sqlx::Error> {
    sqlx::query_as!(
        Job,
        "UPDATE submissions SET status = 'judging'
         WHERE id = (SELECT id FROM submissions WHERE status = 'queued' ORDER BY id LIMIT 1)
         RETURNING id, user_id, round_id, problem_id, problem_version, problem_dir",
    )
    .fetch_optional(&state.db)
    .await
}

pub(crate) async fn worker(state: AppState, box_id: u32) {
    tracing::info!(box_id, "judge worker started");
    loop {
        match claim(&state).await {
            Ok(Some(job)) => {
                if let Err(e) = run(&state, box_id, &job).await {
                    tracing::error!(submission = job.id, error = %e, "could not record result");
                }
            }
            Ok(None) => {
                let _ = tokio::time::timeout(IDLE_POLL, state.queue.notified()).await;
            }
            Err(e) => {
                tracing::error!(error = %e, "cannot read the queue");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

async fn run(state: &AppState, box_id: u32, job: &Job) -> Result<(), sqlx::Error> {
    tracing::info!(submission = job.id, box_id, "judging");
    let dir = resolve_dir(&state.config.data_dir, &job.problem_dir);
    let outcome = match Problem::load(dir) {
        Ok(problem) => {
            let exe = executable_path(state, job.id);
            let total = problem.tests.len() as u32;
            let (id, user_id) = (job.id, job.user_id);
            let events = state.events.clone();
            let on_test = Box::new(move |index: usize, result: &arena_protocol::TestResult| {
                let _ = events.send(Event::ForUser(
                    user_id,
                    ServerEvent::Progress {
                        submission_id: id,
                        test_index: index as u32,
                        test_count: total,
                        result: result.clone(),
                    },
                ));
            });
            state.judge.judge(box_id, &problem, &exe, on_test).await
        }
        Err(e) => Err(format!("cannot load problem: {e}")),
    };
    // A database hiccup must not leave the submission stuck in `judging`.
    let mut attempt = 0;
    let recorded = loop {
        match record(state, job, &outcome).await {
            Ok(r) => break r,
            Err(e) if attempt < 5 => {
                attempt += 1;
                tracing::warn!(submission = job.id, error = %e, attempt, "retrying result write");
                tokio::time::sleep(Duration::from_millis(200 * attempt)).await;
            }
            Err(e) => {
                // Nothing else would ever move it on, so keep trying to mark it
                // failed; the admin can rejudge it once the database recovers.
                let error = format!("the result could not be stored: {e}");
                if give_up(state, job, &error).await {
                    publish_result(state, job).await;
                }
                return Err(e);
            }
        }
    };
    if !recorded {
        // Rejudged while this run was going; the new run will report.
        tracing::info!(
            submission = job.id,
            "stale result dropped, submission was requeued"
        );
        return Ok(());
    }

    let (_, row) = crate::submissions::find(state, job.id)
        .await
        .map_err(|e| sqlx::Error::Protocol(e.message))?;
    state.publish(Event::ForUser(
        job.user_id,
        ServerEvent::Submission {
            submission: row.info(),
        },
    ));
    // Even a rejected result can change a board (a rejudge may remove an entry).
    publish_board(state, job.round_id, job.problem_id).await;
    Ok(())
}

/// Tells the participant and the boards about a result stored outside `run`'s
/// normal path.
async fn publish_result(state: &AppState, job: &Job) {
    match crate::submissions::find(state, job.id).await {
        Ok((_, row)) => state.publish(Event::ForUser(
            job.user_id,
            ServerEvent::Submission {
                submission: row.info(),
            },
        )),
        Err(e) => tracing::error!(submission = job.id, error = %e.message, "cannot publish result"),
    }
    publish_board(state, job.round_id, job.problem_id).await;
}

/// Stores the outcome. Returns false when the submission was requeued (rejudged)
/// while this run was going: then this result is stale and not stored.
async fn record(
    state: &AppState,
    job: &Job,
    outcome: &Result<JudgeReport, String>,
) -> Result<bool, sqlx::Error> {
    let t = now();
    let updated = match outcome {
        Ok(report) => {
            let verdict = verdict_str(report.verdict);
            let time_us = report.time_us as i64;
            let memory_kib = report.memory_kib as i64;
            let json = serde_json::to_string(report).unwrap_or_default();
            tracing::info!(submission = job.id, verdict = %verdict, time_us, memory_kib, "judged");
            sqlx::query!(
                "UPDATE submissions SET status = 'done', verdict = ?, time_us = ?, memory_kib = ?,
                    report = ?, error = NULL, judged_at = ?
                 WHERE id = ? AND status = 'judging' AND problem_version = ?",
                verdict,
                time_us,
                memory_kib,
                json,
                t,
                job.id,
                job.problem_version
            )
            .execute(&state.db)
            .await?
        }
        Err(error) => {
            tracing::error!(submission = job.id, %error, "judging failed");
            let verdict = verdict_str(Verdict::JudgeError);
            sqlx::query!(
                "UPDATE submissions SET status = 'failed', verdict = ?, error = ?, judged_at = ?
                 WHERE id = ? AND status = 'judging' AND problem_version = ?",
                verdict,
                error,
                t,
                job.id,
                job.problem_version
            )
            .execute(&state.db)
            .await?
        }
    };
    Ok(updated.rows_affected() == 1)
}

/// Marks the submission failed, retrying until the database takes it. Returns
/// false when it was requeued meanwhile, so it is no longer this run's. Holds
/// this worker until then: while the database refuses writes it couldn't record
/// other results either.
async fn give_up(state: &AppState, job: &Job, error: &str) -> bool {
    let verdict = verdict_str(Verdict::JudgeError);
    let mut wait = Duration::from_secs(1);
    loop {
        let t = now();
        let result = sqlx::query!(
            "UPDATE submissions SET status = 'failed', verdict = ?, error = ?, judged_at = ?
             WHERE id = ? AND status = 'judging' AND problem_version = ?",
            verdict,
            error,
            t,
            job.id,
            job.problem_version
        )
        .execute(&state.db)
        .await;
        match result {
            Ok(done) => {
                let marked = done.rows_affected() == 1;
                if marked {
                    tracing::error!(submission = job.id, %error, "submission marked failed");
                }
                return marked;
            }
            Err(e) => {
                tracing::error!(submission = job.id, error = %e, "cannot mark submission failed; retrying");
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(Duration::from_secs(30));
            }
        }
    }
}

pub(crate) async fn publish_board(state: &AppState, round_id: i64, problem_id: i64) {
    let result = async {
        let round = crate::rounds::find_row(state, round_id).await?;
        let problem = crate::rounds::problems(state, round_id)
            .await?
            .into_iter()
            .find(|p| p.id == problem_id);
        if let Some(problem) = problem {
            let board = crate::boards::board(state, &round, &problem, false).await?;
            state.publish(Event::Public(ServerEvent::Board { board }));
        }
        Ok::<_, crate::error::ApiError>(())
    }
    .await;
    if let Err(e) = result {
        tracing::error!(error = %e.message, "cannot publish board");
    }
}
