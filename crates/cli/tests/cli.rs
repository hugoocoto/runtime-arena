//! The CLI's commands against an in-process server with a fake judge.
#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use arena_cli::board::{self, BoardSnapshot};
use arena_cli::client::Client;
use arena_cli::config::Config;
use arena_cli::errors::EXIT_USAGE;
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
    let exports = tempfile::tempdir().unwrap();
    let bundle = exports.path().join("round.tar.gz");
    admin::round_export(&admin_client, 1, &bundle)
        .await
        .unwrap();
    let gzip = flate2::read::GzDecoder::new(std::fs::File::open(&bundle).unwrap());
    let mut tar = tar::Archive::new(gzip);
    let files: std::collections::BTreeMap<_, _> = tar
        .entries()
        .unwrap()
        .map(|entry| {
            use std::io::Read;
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap().into_owned();
            let mut contents = String::new();
            entry.read_to_string(&mut contents).unwrap();
            (path.to_str().unwrap().to_owned(), contents)
        })
        .collect();
    assert_eq!(
        files.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "manifest.json",
            "sum/samples/01.in",
            "sum/samples/01.out",
            "sum/statement.md",
        ]
    );
    let manifest: serde_json::Value = serde_json::from_str(&files["manifest.json"]).unwrap();
    assert_eq!(manifest["round_id"], 1);
    assert_eq!(manifest["problems"][0]["time_limit_ms"], 1000);
    assert_eq!(manifest["problems"][0]["memory_limit_mb"], 256);
    assert_eq!(files["sum/samples/01.in"], "3 4\n");
    assert!(
        admin::round_export(&admin_client, 1, &bundle)
            .await
            .is_err()
    );

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
    let samples = work.path().join("samples");
    participant::problem(&config, "sum", Some(&samples), false)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(samples.join("01.out")).unwrap(),
        "7\n"
    );
    assert_eq!(
        std::fs::read_to_string(samples.join("01.in")).unwrap(),
        "3 4\n"
    );

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

#[tokio::test]
async fn board_snapshot_and_non_tty_behavior() {
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
                display_name: Some("Alice".into()),
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
            name: "Snapshot".into(),
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
    assert!(
        participant::submit(&config, "sum", &good, &src, true)
            .await
            .unwrap()
    );

    let snap = board::fetch_snapshot(&anonymous_client).await.unwrap();
    assert_eq!(snap.round.name, "Snapshot");
    assert_eq!(snap.boards.len(), 1);
    assert_eq!(snap.boards[0].problem, "sum");
    assert_eq!(snap.boards[0].by_time[0].user, "alice");
    assert_eq!(snap.boards[0].by_time[0].time_us, 7);
    assert_eq!(snap.boards[0].by_time[0].memory_kib, 2048);

    // Run the compiled CLI binary with non-TTY stdin/stdout against this server.
    let bin = env!("CARGO_BIN_EXE_arena");
    let cfg_path = work.path().join("config.toml");
    std::fs::write(&cfg_path, "").unwrap();

    let plain = tokio::task::spawn_blocking({
        let server = server.clone();
        let cfg_path = cfg_path.clone();
        move || {
            Command::new(bin)
                .arg("board")
                .env("ARENA_CONFIG", &cfg_path)
                .env("ARENA_SERVER", &server)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .unwrap()
        }
    })
    .await
    .unwrap();
    assert_eq!(plain.status.code(), Some(i32::from(EXIT_USAGE)));
    let stderr = String::from_utf8_lossy(&plain.stderr);
    assert!(
        stderr.contains("--once") && stderr.contains("--json"),
        "expected actionable non-TTY error, got: {stderr}"
    );

    let json_out = tokio::task::spawn_blocking({
        let server = server.clone();
        let cfg_path = cfg_path.clone();
        move || {
            Command::new(bin)
                .args(["board", "--json"])
                .env("ARENA_CONFIG", &cfg_path)
                .env("ARENA_SERVER", &server)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .unwrap()
        }
    })
    .await
    .unwrap();
    assert!(
        json_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&json_out.stderr)
    );
    let parsed: BoardSnapshot = serde_json::from_slice(&json_out.stdout).unwrap();
    assert_eq!(parsed.round.name, "Snapshot");
    assert_eq!(parsed.boards[0].by_time[0].submission_id, 1);

    let once_out = tokio::task::spawn_blocking({
        let server = server.clone();
        let cfg_path = cfg_path.clone();
        let token = alice.token.clone();
        move || {
            Command::new(bin)
                .args(["board", "--once"])
                .env("ARENA_CONFIG", &cfg_path)
                .env("ARENA_SERVER", &server)
                .env("ARENA_TOKEN", &token)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .unwrap()
        }
    })
    .await
    .unwrap();
    assert!(
        once_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&once_out.stderr)
    );
    let text = String::from_utf8_lossy(&once_out.stdout);
    assert!(text.contains("Round 1: Snapshot"), "{text}");
    assert!(text.contains("Fastest"), "{text}");
    assert!(text.contains("Leanest"), "{text}");
    assert!(text.contains("Alice (alice) *"), "{text}");
}
