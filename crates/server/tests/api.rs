//! HTTP API tests against a fresh database, without isolate.

mod common;

use arena_protocol::{HostInfo, ProblemStatement, ProblemSummary, RoundInfo, RoundState, UserInfo};
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
async fn host_specs_need_a_login_and_report_the_judge_settings() {
    let s = TestServer::with_config(|c| {
        c.judge_boxes = vec![0, 1];
        c.runs_per_test = 5;
        c.processes = 8;
        c.max_failures = 0;
        c.cg_root = "/nonexistent".into();
    })
    .await;
    assert_eq!(s.get("/api/host", None).await.0, StatusCode::UNAUTHORIZED);
    let alice = s.participant("alice").await;
    for token in [alice.as_str(), ADMIN] {
        let (status, body) = s.get("/api/host", Some(token)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let host: HostInfo = serde_json::from_value(body).unwrap();
        assert_eq!(host.arch, std::env::consts::ARCH);
        assert_eq!(
            (
                host.judge.workers,
                host.judge.runs_per_test,
                host.judge.processes
            ),
            (2, 5, 8)
        );
        assert_eq!(host.judge.max_failures, 0);
        assert!(!host.judge.microsecond_timing);
    }
}

#[tokio::test]
async fn update_status_is_admin_only() {
    let s = TestServer::new().await;
    let (status, _) = s.get("/api/admin/update", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = s.get("/api/admin/update", Some(ADMIN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["current"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body["latest"], serde_json::Value::Null);
    assert_eq!(body["update_available"], false);
}

#[tokio::test]
async fn participant_token_works_and_is_not_admin() {
    let s = TestServer::new().await;
    let token = s.participant("alice").await;
    let (status, body) = s.get("/api/me", Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    let me: UserInfo = serde_json::from_value(body).unwrap();
    assert_eq!(me.name, "alice");
    let (status, _) = s.get("/api/admin/users", Some(&token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn admins_cannot_create_accounts_or_issue_tokens() {
    let s = TestServer::new().await;
    s.participant("alice").await;
    let (status, _) = s
        .post("/api/admin/users", Some(ADMIN), json!({"name": "bob"}))
        .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    let (status, _) = s
        .post("/api/admin/users/alice/token", Some(ADMIN), json!({}))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn signing_up_with_a_taken_name_cannot_take_over_the_account() {
    let s = TestServer::new().await;
    let token = s.participant("alice").await;
    for name in ["alice", "ALICE", " Alice "] {
        let attack = json!({"name": name, "password": "attacker1", "join_code": common::JOIN_CODE});
        assert_eq!(
            s.post("/api/signup", None, attack).await.0,
            StatusCode::CONFLICT,
            "{name}"
        );
    }
    let login = |pw: &str| json!({"name": "alice", "password": pw});
    assert_eq!(
        s.post("/api/login", None, login("attacker1")).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        s.post("/api/login", None, login("secret1")).await.0,
        StatusCode::OK
    );
    assert_eq!(s.get("/api/me", Some(&token)).await.0, StatusCode::OK);
}

#[tokio::test]
async fn repeated_failed_logins_are_refused_per_address() {
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Request, header};

    let s = TestServer::new().await;
    s.participant("alice").await;
    let attempt = |from: [u8; 4], password: &str| {
        let body = json!({"name": "alice", "password": password}).to_string();
        let mut req = Request::post("/api/login")
            .header(header::CONTENT_TYPE, "application/json")
            // A client can't escape the limit by claiming another address.
            .header("x-forwarded-for", format!("198.51.100.{}", from[3]))
            .body(Body::from(body))
            .unwrap();
        let peer = arena_server::net_limit::Peer(std::net::SocketAddr::from((from, 4000)));
        req.extensions_mut().insert(ConnectInfo(peer));
        req
    };
    for _ in 0..10 {
        let (status, _) = s.request(attempt([10, 0, 0, 1], "wrong!")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, body) = s.request(attempt([10, 0, 0, 1], "wrong!")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        body["error"].as_str().unwrap().contains("try again"),
        "{body}"
    );
    // Even the right password waits on that address, but the owner elsewhere doesn't.
    assert_eq!(
        s.request(attempt([10, 0, 0, 1], "secret1")).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        s.request(attempt([10, 0, 0, 2], "secret1")).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn repeated_wrong_event_codes_are_refused_per_address() {
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Request, header};

    let s = TestServer::new().await;
    let attempt = |from: [u8; 4], name: &str, code: &str| {
        let body = json!({"name": name, "password": "secret1", "join_code": code}).to_string();
        let mut req = Request::post("/api/signup")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();
        let peer = arena_server::net_limit::Peer(std::net::SocketAddr::from((from, 4000)));
        req.extensions_mut().insert(ConnectInfo(peer));
        req
    };
    for _ in 0..100 {
        let (status, _) = s.request(attempt([10, 0, 0, 1], "eve", "guess1")).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let (status, body) = s.request(attempt([10, 0, 0, 1], "eve", "guess2")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(
        body["error"].as_str().unwrap().contains("event codes"),
        "{body}"
    );
    // Even the right code waits on that address, but others can still join.
    assert_eq!(
        s.request(attempt([10, 0, 0, 1], "eve", common::JOIN_CODE))
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        s.request(attempt([10, 0, 0, 2], "frank", common::JOIN_CODE))
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn request_flood_is_refused_per_address_except_health_checks() {
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Request, header};
    use tower::ServiceExt;

    let s = TestServer::with_config(|c| c.rate_limit = 1).await;
    let get = |path: &str, from: [u8; 4]| {
        let mut req = Request::get(path).body(Body::empty()).unwrap();
        let peer = arena_server::net_limit::Peer(std::net::SocketAddr::from((from, 4000)));
        req.extensions_mut().insert(ConnectInfo(peer));
        s.app.clone().oneshot(req)
    };
    for _ in 0..5 {
        assert_eq!(
            get("/api/info", [10, 0, 0, 1]).await.unwrap().status(),
            StatusCode::OK
        );
    }
    let refused = get("/api/info", [10, 0, 0, 1]).await.unwrap();
    assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(refused.headers().contains_key(header::RETRY_AFTER));
    assert_eq!(
        get("/healthz", [10, 0, 0, 1]).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        get("/api/info", [10, 0, 0, 2]).await.unwrap().status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn password_reset_recovers_an_account_and_ends_its_sessions() {
    let s = TestServer::new().await;
    let old = s.participant("alice").await;
    let path = "/api/admin/users/alice/password";
    assert_eq!(
        s.post(path, Some(&old), json!({})).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        s.post("/api/admin/users/nobody/password", Some(ADMIN), json!({}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let (status, body) = s.post(path, Some(ADMIN), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let password = body["password"].as_str().unwrap();
    assert_eq!(
        s.get("/api/me", Some(&old)).await.0,
        StatusCode::UNAUTHORIZED
    );
    let login = |pw: &str| json!({"name": "alice", "password": pw});
    assert_eq!(
        s.post("/api/login", None, login("secret1")).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        s.post("/api/login", None, login(password)).await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn password_reset_moves_old_admin_created_accounts_to_passwords() {
    let s = TestServer::new().await;
    let token = "arena_issued_by_an_older_version";
    sqlx::query(
        "INSERT INTO users (name, display_name, token_hash, created_at) VALUES ('old', 'Old', ?, 0)",
    )
    .bind(arena_server::auth::hash_token(token))
    .execute(&s.state.db)
    .await
    .unwrap();
    assert_eq!(s.get("/api/me", Some(token)).await.0, StatusCode::OK);

    let (status, body) = s
        .post("/api/admin/users/old/password", Some(ADMIN), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        s.get("/api/me", Some(token)).await.0,
        StatusCode::UNAUTHORIZED
    );
    let login = json!({"name": "old", "password": body["password"]});
    assert_eq!(s.post("/api/login", None, login).await.0, StatusCode::OK);
}

#[tokio::test]
async fn user_names_are_validated_and_unique() {
    let s = TestServer::new().await;
    let req =
        |name: &str| json!({"name": name, "password": "secret1", "join_code": common::JOIN_CODE});
    assert_eq!(
        s.post("/api/signup", None, req("Bad Name")).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        s.post("/api/signup", None, req("bob")).await.0,
        StatusCode::OK
    );
    assert_eq!(
        s.post("/api/signup", None, req("bob")).await.0,
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
    let alice = s.participant("alice").await;

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
        s.get("/api/problems/sum", Some(&alice)).await.0,
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

    let (status, body) = s.get("/api/problems/sum", Some(&alice)).await;
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

#[tokio::test]
async fn sign_up_then_log_in_from_two_devices() {
    let s = TestServer::new().await;
    let signup = json!({"name": "Dana", "display_name": "Dana R.", "password": "secret1", "join_code": common::JOIN_CODE});
    let (status, body) = s.post("/api/signup", None, signup.clone()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], "dana", "names are lowercased");
    // The name is taken now.
    assert_eq!(
        s.post("/api/signup", None, signup).await.0,
        StatusCode::CONFLICT
    );

    let login = |pw: &str| json!({"name": "dana", "password": pw});
    assert_eq!(
        s.post("/api/login", None, login("wrong!")).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, a) = s.post("/api/login", None, login("secret1")).await;
    assert_eq!(status, StatusCode::OK, "{a}");
    let (_, b) = s.post("/api/login", None, login("secret1")).await;
    let (ta, tb) = (a["token"].as_str().unwrap(), b["token"].as_str().unwrap());
    assert_ne!(ta, tb);
    // Both devices stay logged in.
    for t in [ta, tb] {
        let (status, me) = s.get("/api/me", Some(t)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(me["display_name"], "Dana R.");
    }
    // Logging out ends only that device's session.
    let (status, _) = s.post("/api/logout", Some(ta), json!({})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(s.get("/api/me", Some(ta)).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(s.get("/api/me", Some(tb)).await.0, StatusCode::OK);
    assert_eq!(
        s.post("/api/logout", Some(ta), json!({})).await.0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn sign_up_needs_the_event_code_and_a_password() {
    let s = TestServer::new().await;
    let req = |code: &str, pw: &str| json!({"name": "eve", "password": pw, "join_code": code});
    assert_eq!(
        s.post("/api/signup", None, req("nope", "secret1")).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        s.post("/api/signup", None, req(common::JOIN_CODE, "12345"))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    // An unknown name gets the same answer as a wrong password.
    let (status, _) = s
        .post(
            "/api/login",
            None,
            json!({"name": "frank", "password": "anything"}),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sign_up_is_off_without_a_join_code() {
    let s = TestServer::with_config(|c| c.join_code = None).await;
    let (status, _) = s
        .post(
            "/api/signup",
            None,
            json!({"name": "eve", "password": "secret1", "join_code": "x"}),
        )
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = s.get("/join", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.as_str().unwrap().contains("/api/signup"));
}

#[tokio::test]
async fn participant_guide_is_public() {
    let s = TestServer::new().await;
    let (status, body) = s.get("/help", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.as_str().unwrap().contains("arena submit"));
}
