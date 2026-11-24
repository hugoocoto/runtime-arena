//! `GET /ws` over a real socket.

mod common;

use std::time::Duration;

use arena_protocol::{ServerEvent, Verdict};
use arena_server::Event;
use axum::http::StatusCode;
use common::{ADMIN, TestServer, fake_solution};
use futures_util::StreamExt;
use serde_json::json;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn listen(s: &TestServer) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = s.app.clone();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("ws://{addr}/ws")
}

async fn connect(url: &str, token: Option<&str>) -> Result<Ws, WsError> {
    let mut req = url.into_client_request().unwrap();
    if let Some(t) = token {
        req.headers_mut()
            .insert("authorization", format!("Bearer {t}").parse().unwrap());
    }
    tokio_tungstenite::connect_async(req)
        .await
        .map(|(ws, _)| ws)
}

async fn next(ws: &mut Ws) -> ServerEvent {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("no event within 5 s")
            .unwrap()
            .unwrap();
        if let Message::Text(text) = msg {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

/// Reads events until `pick` returns something.
async fn until<T>(ws: &mut Ws, mut pick: impl FnMut(ServerEvent) -> Option<T>) -> T {
    loop {
        if let Some(t) = pick(next(ws).await) {
            return t;
        }
    }
}

#[tokio::test]
async fn participants_get_their_own_progress_and_everyone_gets_boards() {
    let s = TestServer::new().await;
    arena_server::start_background(&s.state).await.unwrap();
    s.upload_problem("sum", common::problem_archive("sum"))
        .await;
    let alice = s.participant("alice").await;
    let bob = s.participant("bob").await;
    let (_, round) = s
        .post(
            "/api/admin/rounds",
            Some(ADMIN),
            json!({"name": "R", "duration_minutes": 60, "cooldown_seconds": 0, "problems": ["sum"]}),
        )
        .await;
    let url = listen(&s).await;

    let mut viewer = connect(&url, None).await.unwrap();
    assert_eq!(next(&mut viewer).await, ServerEvent::Hello { user: None });
    let mut alice_ws = connect(&url, Some(&alice)).await.unwrap();
    assert_eq!(
        next(&mut alice_ws).await,
        ServerEvent::Hello {
            user: Some("alice".into())
        }
    );
    let mut bob_ws = connect(&url, Some(&bob)).await.unwrap();
    next(&mut bob_ws).await;

    // Starting the round reaches everyone.
    s.post(
        &format!("/api/admin/rounds/{}/start", round["id"]),
        Some(ADMIN),
        json!({}),
    )
    .await;
    until(&mut viewer, |e| {
        matches!(e, ServerEvent::Round { .. }).then_some(())
    })
    .await;

    let (_, sub) = s.submit(&alice, "sum", fake_solution("time=25")).await;
    let sub_id = sub["id"].as_i64().unwrap();

    // Alice sees per-test progress, then the result.
    let progress = until(&mut alice_ws, |e| match e {
        ServerEvent::Progress {
            submission_id,
            test_count,
            ..
        } => Some((submission_id, test_count)),
        _ => None,
    })
    .await;
    assert_eq!(progress, (sub_id, 2));
    let done = until(&mut alice_ws, |e| match e {
        ServerEvent::Submission { submission } => Some(submission),
        _ => None,
    })
    .await;
    assert_eq!(done.verdict, Some(Verdict::Accepted));

    // The anonymous viewer gets the updated board but no private events.
    let board = until(&mut viewer, |e| match e {
        ServerEvent::Board { board } if !board.by_time.is_empty() => Some(board),
        ServerEvent::Progress { .. } | ServerEvent::Submission { .. } => {
            panic!("leaked private event")
        }
        _ => None,
    })
    .await;
    assert_eq!(board.by_time[0].user, "alice");

    // Bob also only sees the public board.
    until(&mut bob_ws, |e| match e {
        ServerEvent::Board { board } if !board.by_time.is_empty() => Some(()),
        ServerEvent::Progress { .. } | ServerEvent::Submission { .. } => {
            panic!("bob saw alice's events")
        }
        _ => None,
    })
    .await;
}

#[tokio::test]
async fn new_connections_get_a_snapshot_and_bad_tokens_are_refused() {
    let s = TestServer::new().await;
    s.upload_problem("sum", common::problem_archive("sum"))
        .await;
    let (_, round) = s
        .post(
            "/api/admin/rounds",
            Some(ADMIN),
            json!({"name": "R", "duration_minutes": 60, "problems": ["sum"]}),
        )
        .await;
    s.post(
        &format!("/api/admin/rounds/{}/start", round["id"]),
        Some(ADMIN),
        json!({}),
    )
    .await;
    let url = listen(&s).await;

    let mut ws = connect(&url, None).await.unwrap();
    next(&mut ws).await; // hello
    assert!(matches!(next(&mut ws).await, ServerEvent::Round { .. }));
    assert!(matches!(next(&mut ws).await, ServerEvent::Board { .. }));

    match connect(&url, Some("not-a-token")).await {
        Err(WsError::Http(resp)) => assert_eq!(resp.status(), 401),
        other => panic!("expected 401, got {other:?}"),
    }
}

/// Waits for the server to close the connection.
async fn closed(ws: &mut Ws) {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("the connection stayed open")
        {
            None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
            Some(Ok(_)) => {}
        }
    }
}

#[tokio::test]
async fn revoked_tokens_close_their_open_connections() {
    let s = TestServer::new().await;
    let first = s.participant("alice").await;
    let (_, body) = s
        .post(
            "/api/login",
            None,
            json!({"name": "alice", "password": "secret1"}),
        )
        .await;
    let second = body["token"].as_str().unwrap().to_owned();
    let url = listen(&s).await;
    let mut a = connect(&url, Some(&first)).await.unwrap();
    let mut b = connect(&url, Some(&second)).await.unwrap();
    next(&mut a).await; // hello
    next(&mut b).await;

    // Logging out ends that device's connection only.
    let (status, _) = s.post("/api/logout", Some(&first), json!({})).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    closed(&mut a).await;
    s.state
        .events
        .send(Event::Public(ServerEvent::Hello { user: None }))
        .unwrap();
    assert!(matches!(next(&mut b).await, ServerEvent::Hello { .. }));

    // A password reset ends all of them.
    let (status, _) = s
        .post("/api/admin/users/alice/password", Some(ADMIN), json!({}))
        .await;
    assert_eq!(status, StatusCode::OK);
    closed(&mut b).await;
}

#[tokio::test]
async fn a_connection_that_falls_behind_is_closed_to_resync() {
    let s = TestServer::new().await;
    let url = listen(&s).await;
    let mut ws = connect(&url, None).await.unwrap();
    next(&mut ws).await; // hello; no round, so no snapshot follows
    // No await in between: the connection can't keep up and lags.
    for _ in 0..2000 {
        s.state
            .events
            .send(Event::Public(ServerEvent::Hello { user: None }))
            .unwrap();
    }
    closed(&mut ws).await;
}

#[tokio::test]
async fn one_participant_cannot_hold_unlimited_connections() {
    let s = TestServer::new().await;
    let token = s.participant("alice").await;
    let url = listen(&s).await;
    let mut held = Vec::new();
    loop {
        match connect(&url, Some(&token)).await {
            Ok(ws) => held.push(ws),
            Err(WsError::Http(resp)) => {
                assert_eq!(resp.status(), 429);
                break;
            }
            Err(e) => panic!("{e}"),
        }
        assert!(held.len() <= 64, "no limit");
    }
    assert_eq!(held.len(), 16);
    // Anonymous viewers are counted separately.
    connect(&url, None).await.unwrap();
}
