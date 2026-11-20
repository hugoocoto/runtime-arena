//! HTTP API tests against a fresh database, without isolate.

mod common;

use arena_protocol::{
    ProblemStatement, ProblemSummary, RoundInfo, RoundState, UserInfo, UserToken,
};
use axum::http::StatusCode;
use common::{ADMIN, TestServer};
use serde_json::json;

#[tokio::test]
async fn admin_endpoints_require_the_admin_token() {
    let s = TestServer::new().await;
    let (status, _) = s.get("/api/admin/users", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = s.get("/api/admin/users", Some("wrong-token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = s.get("/api/admin/users", Some(ADMIN)).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn participant_token_works_and_can_be_reset() {
    let s = TestServer::new().await;
    let (status, body) = s
        .post(
            "/api/admin/users",
            Some(ADMIN),
            json!({"name": "alice", "display_name": "Alice"}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let created: UserToken = serde_json::from_value(body).unwrap();

    let (status, body) = s.get("/api/me", Some(&created.token)).await;
    assert_eq!(status, StatusCode::OK);
    let me: UserInfo = serde_json::from_value(body).unwrap();
    assert_eq!(
        (me.name.as_str(), me.display_name.as_str()),
        ("alice", "Alice")
    );

    // Participants are not admins.
    let (status, _) = s.get("/api/admin/users", Some(&created.token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body) = s
        .post("/api/admin/users/alice/token", Some(ADMIN), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    let reset: UserToken = serde_json::from_value(body).unwrap();
    assert_ne!(reset.token, created.token);
    assert_eq!(
        s.get("/api/me", Some(&created.token)).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(s.get("/api/me", Some(&reset.token)).await.0, StatusCode::OK);
}

#[tokio::test]
async fn user_names_are_validated_and_unique() {
    let s = TestServer::new().await;
    let (status, _) = s
        .post("/api/admin/users", Some(ADMIN), json!({"name": "Bad Name"}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        s.post("/api/admin/users", Some(ADMIN), json!({"name": "bob"}))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        s.post("/api/admin/users", Some(ADMIN), json!({"name": "bob"}))
            .await
            .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn problem_upload_creates_versions() {
    let s = TestServer::new().await;
    let (status, body) = s
        .upload_problem("sum", common::problem_archive("sum"))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let p: ProblemSummary = serde_json::from_value(body).unwrap();
    assert_eq!((p.slug.as_str(), p.version, p.test_count), ("sum", 1, 2));
    assert_eq!((p.time_limit_ms, p.memory_limit_mb), (1000, 256));

    let (_, body) = s
        .upload_problem("sum", common::problem_archive("sum"))
        .await;
    let p: ProblemSummary = serde_json::from_value(body).unwrap();
    assert_eq!(p.version, 2);
}

#[tokio::test]
async fn invalid_problem_archives_are_rejected() {
    let s = TestServer::new().await;
    let (status, body) = s.upload_problem("sum", b"not a tarball".to_vec()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) = s
        .upload_problem("sum", common::archive_with_symlink())
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().unwrap().contains("only files"),
        "{body}"
    );

    let (status, _) = s
        .upload_problem("Bad_Slug", common::problem_archive("sum"))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn round_lifecycle() {
    let s = TestServer::new().await;
    s.upload_problem("sum", common::problem_archive("sum"))
        .await;
    let (_, body) = s
        .post("/api/admin/users", Some(ADMIN), json!({"name": "alice"}))
        .await;
    let alice: UserToken = serde_json::from_value(body).unwrap();

    // No round yet.
    assert_eq!(
        s.get("/api/rounds/current", None).await.0,
        StatusCode::NOT_FOUND
    );

    let (status, body) = s
        .post(
            "/api/admin/rounds",
            Some(ADMIN),
            json!({"name": "Round 1", "duration_minutes": 60, "freeze_minutes": 10, "problems": ["sum"]}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let round: RoundInfo = serde_json::from_value(body).unwrap();
    assert_eq!(round.state, RoundState::Draft);
    assert_eq!(round.cooldown_seconds, 30);

    // Statements stay hidden from participants before the start.
    assert_eq!(
        s.get("/api/problems/sum", Some(&alice.token)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        s.get("/api/problems/sum", Some(ADMIN)).await.0,
        StatusCode::OK
    );

    let (status, body) = s
        .post(
            &format!("/api/admin/rounds/{}/start", round.id),
            Some(ADMIN),
            json!({}),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let started: RoundInfo = serde_json::from_value(body).unwrap();
    assert_eq!(started.state, RoundState::Running);
    assert_eq!(started.ends_at.unwrap() - started.started_at.unwrap(), 3600);

    let (_, body) = s.get("/api/rounds/current", None).await;
    let current: RoundInfo = serde_json::from_value(body).unwrap();
    assert_eq!(current.id, round.id);
    assert!((current.server_time - arena_server::now()).abs() <= 2);
    assert_eq!(current.problems.len(), 1);

    let (status, body) = s.get("/api/problems/sum", Some(&alice.token)).await;
    assert_eq!(status, StatusCode::OK);
    let st: ProblemStatement = serde_json::from_value(body).unwrap();
    assert_eq!(st.samples.len(), 1);
    assert_eq!(st.samples[0].input, "3 4\n");

    // Problems of a started round are locked.
    let (status, _) = s
        .upload_problem("sum", common::problem_archive("sum"))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Starting twice or a second round at once is refused.
    let path = format!("/api/admin/rounds/{}/start", round.id);
    assert_eq!(
        s.post(&path, Some(ADMIN), json!({})).await.0,
        StatusCode::CONFLICT
    );

    let (_, body) = s
        .post(
            &format!("/api/admin/rounds/{}/extend", round.id),
            Some(ADMIN),
            json!({"minutes": 15}),
        )
        .await;
    let extended: RoundInfo = serde_json::from_value(body).unwrap();
    assert_eq!(extended.ends_at.unwrap(), started.ends_at.unwrap() + 900);
    assert_eq!(extended.duration_minutes, 75);

    let (_, body) = s
        .post(
            &format!("/api/admin/rounds/{}/stop", round.id),
            Some(ADMIN),
            json!({}),
        )
        .await;
    let stopped: RoundInfo = serde_json::from_value(body).unwrap();
    assert_eq!(stopped.state, RoundState::Finished);
    let stop = format!("/api/admin/rounds/{}/stop", round.id);
    assert_eq!(
        s.post(&stop, Some(ADMIN), json!({})).await.0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn round_creation_is_validated() {
    let s = TestServer::new().await;
    s.upload_problem("sum", common::problem_archive("sum"))
        .await;
    let bad = [
        json!({"name": "r", "duration_minutes": 0, "problems": ["sum"]}),
        json!({"name": "r", "duration_minutes": 10, "freeze_minutes": 11, "problems": ["sum"]}),
        json!({"name": "r", "duration_minutes": 10, "problems": []}),
        json!({"name": "r", "duration_minutes": 10, "problems": ["sum", "sum"]}),
    ];
    for body in bad {
        let (status, resp) = s.post("/api/admin/rounds", Some(ADMIN), body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body} -> {resp}");
    }
    let (status, _) = s
        .post(
            "/api/admin/rounds",
            Some(ADMIN),
            json!({"name": "r", "duration_minutes": 10, "problems": ["nope"]}),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn projector_page_is_served() {
    let s = TestServer::new().await;
    let (status, body) = s.get("/", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.as_str().unwrap().contains("new WebSocket"));
}
