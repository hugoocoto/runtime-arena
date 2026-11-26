//! Submission intake, the judge queue and boards, with a fake judge.

mod common;

use std::time::Duration;

use arena_protocol::{Board, SubmissionInfo, SubmissionStatus, Verdict};
use axum::http::StatusCode;
use common::{ADMIN, TestServer, fake_solution};
use serde_json::json;

async fn server() -> TestServer {
    let s = TestServer::new().await;
    arena_server::start_background(&s.state).await.unwrap();
    let (status, body) = s
        .upload_problem("sum", common::problem_archive("sum"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    s
}

async fn user(s: &TestServer, name: &str) -> String {
    s.participant(name).await
}

async fn start_round(s: &TestServer, extra: serde_json::Value) -> i64 {
    let mut req =
        json!({"name": "R", "duration_minutes": 60, "cooldown_seconds": 0, "problems": ["sum"]});
    req.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let (status, body) = s.post("/api/admin/rounds", Some(ADMIN), req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = body["id"].as_i64().unwrap();
    let (status, _) = s
        .post(
            &format!("/api/admin/rounds/{id}/start"),
            Some(ADMIN),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    id
}

async fn submit(s: &TestServer, token: &str, spec: &str) -> SubmissionInfo {
    let (status, body) = s.submit(token, "sum", fake_solution(spec)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    serde_json::from_value(body).unwrap()
}

/// Polls until the submission leaves the queue.
async fn wait_judged(s: &TestServer, token: &str, id: i64) -> SubmissionInfo {
    for _ in 0..200 {
        let (_, body) = s.get(&format!("/api/submissions/{id}"), Some(token)).await;
        let info: SubmissionInfo = serde_json::from_value(body).unwrap();
        if matches!(
            info.status,
            SubmissionStatus::Done | SubmissionStatus::Failed
        ) {
            return info;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("submission {id} was not judged");
}

async fn boards(s: &TestServer, round: i64, token: Option<&str>) -> Vec<Board> {
    let (status, body) = s.get(&format!("/api/rounds/{round}/boards"), token).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).unwrap()
}

#[tokio::test]
async fn submissions_need_a_running_round() {
    let s = server().await;
    let alice = user(&s, "alice").await;
    let (status, _) = s.submit(&alice, "sum", fake_solution("")).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn accepted_submission_reaches_both_boards() {
    let s = server().await;
    let alice = user(&s, "alice").await;
    let round = start_round(&s, json!({})).await;

    let sub = submit(&s, &alice, "time=30 memory=900").await;
    let done = wait_judged(&s, &alice, sub.id).await;
    assert_eq!(done.verdict, Some(Verdict::Accepted));
    assert_eq!((done.time_us, done.memory_kib), (Some(30), Some(900)));
    assert_eq!(done.report.unwrap().tests.len(), 2);

    let b = &boards(&s, round, None).await[0];
    assert_eq!(b.problem, "sum");
    assert!(!b.frozen);
    assert_eq!(b.by_time.len(), 1);
    assert_eq!(
        (b.by_time[0].user.as_str(), b.by_time[0].rank),
        ("alice", 1)
    );
    assert_eq!(b.by_memory[0].memory_kib, 900);
}

#[tokio::test]
async fn boards_rank_time_and_memory_separately() {
    let s = server().await;
    let (ann, bob) = (user(&s, "ann").await, user(&s, "bob").await);
    let round = start_round(&s, json!({})).await;
    for (token, spec) in [
        (&ann, "time=20 memory=5000"),
        (&bob, "time=40 memory=800"),
        (&ann, "time=50 memory=700"), // ann's leanest, but slower
        (&bob, "verdict=wrong_answer time=1 memory=1"),
    ] {
        let sub = submit(&s, token, spec).await;
        wait_judged(&s, token, sub.id).await;
    }
    let b = &boards(&s, round, None).await[0];
    let by_time: Vec<_> = b
        .by_time
        .iter()
        .map(|e| (e.user.as_str(), e.time_us))
        .collect();
    assert_eq!(by_time, [("ann", 20), ("bob", 40)]);
    let by_memory: Vec<_> = b
        .by_memory
        .iter()
        .map(|e| (e.user.as_str(), e.memory_kib))
        .collect();
    assert_eq!(by_memory, [("ann", 700), ("bob", 800)]);
    // Wrong answers count as attempts but never rank.
    assert_eq!(b.by_time[1].attempts, 2);

    let (_, body) = s
        .get(&format!("/api/rounds/{round}/problems/sum/accepted"), None)
        .await;
    let history: Vec<arena_protocol::AcceptedSubmission> = serde_json::from_value(body).unwrap();
    let users: Vec<_> = history.iter().map(|a| a.user.as_str()).collect();
    assert_eq!(users, ["ann", "bob", "ann"]);
}

#[tokio::test]
async fn cooldown_is_enforced() {
    let s = server().await;
    let alice = user(&s, "alice").await;
    start_round(&s, json!({"cooldown_seconds": 60})).await;
    submit(&s, &alice, "").await;
    let (status, body) = s.submit(&alice, "sum", fake_solution("")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert!(body["error"].as_str().unwrap().starts_with("wait "));
}

#[tokio::test]
async fn uploads_are_validated() {
    let s = server().await;
    let alice = user(&s, "alice").await;
    start_round(&s, json!({})).await;

    let mut arm64 = vec![0u8; 64];
    arm64[..4].copy_from_slice(b"\x7fELF");
    arm64[4] = 2;
    arm64[5] = 1;
    arm64[18] = 183;
    for exe in [
        arm64,
        b"print('hi')".to_vec(),
        b"#!/no/such/python\n".to_vec(),
    ] {
        let (status, body) = s.submit(&alice, "sum", exe).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    let (status, _) = s.submit(&alice, "nope", fake_solution("")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn submissions_are_private() {
    let s = server().await;
    let (alice, bob) = (user(&s, "alice").await, user(&s, "bob").await);
    start_round(&s, json!({})).await;
    let sub = submit(&s, &alice, "").await;
    let path = format!("/api/submissions/{}", sub.id);
    assert_eq!(s.get(&path, Some(&bob)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(s.get(&path, Some(ADMIN)).await.0, StatusCode::OK);

    let (_, body) = s.get("/api/me/submissions", Some(&bob)).await;
    assert_eq!(body.as_array().unwrap().len(), 0);
    let (_, body) = s.get("/api/admin/submissions", Some(ADMIN)).await;
    assert_eq!(body.as_array().unwrap().len(), 1);

    let (status, body) = s
        .get(
            &format!("/api/admin/submissions/{}/source", sub.id),
            Some(ADMIN),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "int main() {}");
}

#[tokio::test]
async fn judge_failures_are_reported_and_can_be_rejudged() {
    let s = server().await;
    let alice = user(&s, "alice").await;
    start_round(&s, json!({})).await;
    let sub = submit(&s, &alice, "verdict=crash").await;
    let done = wait_judged(&s, &alice, sub.id).await;
    assert_eq!(done.status, SubmissionStatus::Failed);
    assert_eq!(done.verdict, Some(Verdict::JudgeError));

    let (status, body) = s
        .post(
            &format!("/api/admin/submissions/{}/rejudge", sub.id),
            Some(ADMIN),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let again = wait_judged(&s, &alice, sub.id).await;
    assert_eq!(again.status, SubmissionStatus::Failed);
}

#[tokio::test]
async fn freeze_hides_new_results_from_the_public_board_only() {
    let s = server().await;
    let alice = user(&s, "alice").await;
    // Freeze as long as the round: frozen from the start.
    let round = start_round(&s, json!({"duration_minutes": 5, "freeze_minutes": 5})).await;
    let sub = submit(&s, &alice, "").await;
    let done = wait_judged(&s, &alice, sub.id).await;
    assert_eq!(
        done.verdict,
        Some(Verdict::Accepted),
        "own verdict stays visible"
    );

    let public = &boards(&s, round, None).await[0];
    assert!(public.frozen);
    assert!(public.by_time.is_empty());
    let live = &boards(&s, round, Some(ADMIN)).await[0];
    assert!(!live.frozen);
    assert_eq!(live.by_time.len(), 1);
}

#[tokio::test]
async fn locked_problem_can_be_replaced_with_rejudge() {
    let s = server().await;
    let alice = user(&s, "alice").await;
    start_round(&s, json!({})).await;
    let sub = submit(&s, &alice, "").await;
    assert_eq!(wait_judged(&s, &alice, sub.id).await.problem_version, 1);

    let (status, _) = s
        .upload_problem("sum", common::problem_archive("sum"))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, body) = s
        .upload_problem("sum?rejudge=true", common::problem_archive("sum"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let again = wait_judged(&s, &alice, sub.id).await;
    assert_eq!(again.problem_version, 2);
}

#[tokio::test]
async fn extending_a_frozen_round_keeps_it_frozen() {
    let s = server().await;
    let alice = user(&s, "alice").await;
    let round = start_round(&s, json!({"duration_minutes": 5, "freeze_minutes": 5})).await;
    let sub = submit(&s, &alice, "").await;
    wait_judged(&s, &alice, sub.id).await;

    // Moving the end by more than the freeze length must not reveal the results.
    let (status, body) = s
        .post(
            &format!("/api/admin/rounds/{round}/extend"),
            Some(ADMIN),
            json!({"minutes": 30}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let public = &boards(&s, round, None).await[0];
    assert!(public.frozen);
    assert!(public.by_time.is_empty());

    let (_, body) = s
        .get(&format!("/api/rounds/{round}/problems/sum/accepted"), None)
        .await;
    assert_eq!(body.as_array().unwrap().len(), 0, "history is frozen too");
}

#[tokio::test]
async fn extending_before_the_freeze_moves_it() {
    let s = server().await;
    let round = start_round(&s, json!({"duration_minutes": 60, "freeze_minutes": 10})).await;
    s.post(
        &format!("/api/admin/rounds/{round}/extend"),
        Some(ADMIN),
        json!({"minutes": 30}),
    )
    .await;
    // Freeze is still 10 minutes before the (new) end, so not frozen now.
    assert!(!boards(&s, round, None).await[0].frozen);
}

#[tokio::test]
async fn deleting_a_round_keeps_the_files_of_submissions_that_reuse_its_ids() {
    let s = server().await;
    let token = user(&s, "alice").await;
    let first = start_round(&s, json!({})).await;
    let old = submit(&s, &token, "AC").await;
    wait_judged(&s, &token, old.id).await;
    let (status, _) = s
        .post(
            &format!("/api/admin/rounds/{first}/stop"),
            Some(ADMIN),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    start_round(&s, json!({})).await;

    let (status, body) = s
        .request(
            axum::http::Request::delete(format!("/api/admin/rounds/{first}"))
                .header("authorization", format!("Bearer {ADMIN}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let submissions = s.state.config.data_dir.join("submissions");
    let left: Vec<_> = std::fs::read_dir(&submissions)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "leftover files: {left:?}");

    // SQLite gives the deleted id out again; the new submission's files are its own.
    let new = submit(&s, &token, "AC").await;
    assert_eq!(new.id, old.id, "the id is reused");
    let (status, _) = s
        .get(
            &format!("/api/admin/submissions/{}/source", new.id),
            Some(ADMIN),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        wait_judged(&s, &token, new.id).await.verdict,
        Some(Verdict::Accepted)
    );
}

#[tokio::test]
async fn submission_refusals_record_reasons_and_stay_private() {
    use arena_protocol::{SUBMISSION_CLIENT_TIME_HEADER, SubmissionRefusal};
    use axum::body::Body;
    use axum::http::{Request, header};

    let s = server().await;
    let alice = user(&s, "alice").await;
    let bob = user(&s, "bob").await;
    let before = arena_server::now();
    assert_eq!(
        s.submit(&alice, "sum", fake_solution("verdict=accepted"))
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        s.submit(&alice, "sum", b"invalid".to_vec()).await.0,
        StatusCode::BAD_REQUEST
    );
    let request = Request::post("/api/submissions")
        .header(header::AUTHORIZATION, format!("Bearer {alice}"))
        .header(SUBMISSION_CLIENT_TIME_HEADER, "12345")
        .body(Body::empty())
        .unwrap();
    assert_eq!(s.request(request).await.0, StatusCode::BAD_REQUEST);

    let path = "/api/me/submission-refusals";
    let (status, body) = s.get(path, Some(&alice)).await;
    assert_eq!(status, StatusCode::OK);
    let attempts: Vec<SubmissionRefusal> = serde_json::from_value(body).unwrap();
    let reasons: Vec<_> = attempts
        .iter()
        .map(|attempt| attempt.reason.as_str())
        .collect();
    assert_eq!(
        reasons,
        ["round_not_running", "bad_binary", "invalid_multipart"]
    );
    assert_eq!(attempts[0].problem.as_deref(), Some("sum"));
    assert_eq!(attempts[0].client_at, None);
    assert_eq!(attempts[2].client_at, Some(12345));
    for attempt in attempts {
        assert!(attempt.received_at >= before);
        assert!(attempt.refused_at >= attempt.received_at);
    }
    assert_eq!(s.get(path, Some(&bob)).await, (StatusCode::OK, json!([])));
    assert_eq!(s.get(path, None).await.0, StatusCode::UNAUTHORIZED);
}
