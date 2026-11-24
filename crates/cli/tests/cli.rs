//! The CLI's commands against an in-process server with a fake judge.
#![cfg(target_os = "linux")]

use std::path::Path;
use std::sync::Arc;

use arena_cli::client::Client;
use arena_cli::config::Config;
use arena_cli::{admin, participant};
use arena_protocol::{
    CreateRound, JudgeReport, LoginRequest, SignupRequest, TestResult, UserInfo, UserToken, Verdict,
};
use arena_server::backend::{JudgeFuture, ProgressFn};
use arena_server::{AppState, JudgeBackend};

const ADMIN: &str = "cli-test-admin-token-0123456789";
const JOIN_CODE: &str = "blue-falcon";

/// Accepts everything except files containing "wrong".
struct FakeJudge;

impl JudgeBackend for FakeJudge {
    fn judge<'a>(
        &'a self,
        _box_id: u32,
        problem: &'a arena_judge::Problem,
        executable: &'a Path,
        mut on_test: ProgressFn<'a>,
    ) -> JudgeFuture<'a> {
        Box::pin(async move {
            let wrong = std::fs::read_to_string(executable)
                .map_err(|e| e.to_string())?
                .contains("wrong");
            let mut tests = Vec::new();
            for (i, t) in problem.tests.iter().enumerate() {
                let verdict = if wrong && i == 1 {
                    Verdict::WrongAnswer
                } else {
                    Verdict::Accepted
                };
                let r = TestResult {
                    name: t.name.clone(),
                    verdict,
                    time_us: 7,
                    wall_time_us: 8,
                    memory_kib: 2048,
                    message: (verdict != Verdict::Accepted).then(|| "token 1: expected 30".into()),
                };
                on_test(i, &r);
                tests.push(r);
            }
            Ok(JudgeReport::from_tests(tests))
        })
    }
}

async fn start_server(data: &Path) -> String {
    let config = arena_server::Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        data_dir: data.to_path_buf(),
        admin_token: ADMIN.into(),
        join_code: Some(JOIN_CODE.into()),
        judge_boxes: vec![0],
        isolate: "isolate".into(),
        runs_per_test: 1,
        processes: 64,
        max_failures: 3,
        cg_root: "/sys/fs/cgroup/isolate".into(),
        box_storage: None,
        update_check: false,
        rate_limit: 0,
        trusted_proxies: Vec::new(),
    };
    let db = arena_server::open_db(data).await.unwrap();
    let state = AppState::new(db, config, Arc::new(FakeJudge));
    arena_server::start_background(&state).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = arena_server::app(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../judge/tests/fixtures/sum")
}

#[tokio::test]
async fn organiser_and_participant_flow() {
    let data = tempfile::tempdir().unwrap();
    let server = start_server(data.path()).await;
    let admin_client = Client::new(&server, Some(ADMIN.into())).unwrap();

    admin::problem_add(&admin_client, &fixture(), Some("sum"), false)
        .await
        .unwrap();
    let anonymous_client = Client::new(&server, None).unwrap();
    let _: UserInfo = anonymous_client
        .post(
            "/api/signup",
            &SignupRequest {
                name: "alice".into(),
                display_name: None,
                password: "secret1".into(),
                join_code: JOIN_CODE.into(),
            },
        )
        .await
        .unwrap();
    let alice: UserToken = anonymous_client
        .post(
            "/api/login",
            &LoginRequest {
                name: "alice".into(),
                password: "secret1".into(),
            },
        )
        .await
        .unwrap();
    admin::round_create(
        &admin_client,
        CreateRound {
            name: "CLI".into(),
            duration_minutes: 30,
            freeze_minutes: 0,
            cooldown_seconds: 0,
            problems: vec!["sum".into()],
        },
    )
    .await
    .unwrap();
    admin::round_action(&admin_client, 1, "start", None)
        .await
        .unwrap();

    let config = Config {
        server: Some(server.clone()),
        token: Some(alice.token.clone()),
        admin_token: None,
        ca_certificate: None,
    };
    let work = tempfile::tempdir().unwrap();
    let src = work.path().join("sol.c");
    std::fs::write(&src, "int main() {}").unwrap();
    let good = work.path().join("good");
    std::fs::write(&good, "#!/bin/sh\necho 7\n").unwrap();
    let bad = work.path().join("bad");
    std::fs::write(&bad, "#!/bin/sh\n# wrong\n").unwrap();

    let accepted = participant::submit(&config, "sum", &good, &src, true)
        .await
        .unwrap();
    assert!(accepted);
    let accepted = participant::submit(&config, "sum", &bad, &src, true)
        .await
        .unwrap();
    assert!(!accepted);

    // Without a participant token there is nothing to submit with.
    let anonymous = Config {
        token: None,
        ..config.clone()
    };
    assert!(
        participant::submit(&anonymous, "sum", &good, &src, true)
            .await
            .is_err()
    );

    // The admin can fetch the source that was uploaded with it.
    let out = work.path().join("src.tar.gz");
    admin::source(&admin_client, 1, &out).await.unwrap();
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(
        std::fs::File::open(&out).unwrap(),
    ));
    let names: Vec<String> = ar
        .entries()
        .unwrap()
        .map(|e| e.unwrap().path().unwrap().display().to_string())
        .collect();
    assert_eq!(names, ["sol.c"]);
}
