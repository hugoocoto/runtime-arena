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

#[tokio::test]
async fn build_and_pull_select_judge_platform_and_allow_offline_override() {
    let data = tempfile::tempdir().unwrap();
    let docker = data.path().join("docker");
    std::fs::write(&docker, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755)).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new().route(
        "/api/host",
        axum::routing::get(|| async {
            axum::Json(serde_json::json!({
                "arch": "aarch64",
                "cpu_flags": [],
                "caches": [],
                "judge": {
                    "workers": 1,
                    "runs_per_test": 1,
                    "processes": 1,
                    "max_failures": 1,
                    "microsecond_timing": false,
                },
            }))
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let config_path = data.path().join("config.toml");
    let path = std::env::join_paths(
        std::iter::once(data.path().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let invoke = move |args: &[&str], server: &str| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_arena"))
            .args(args)
            .env("PATH", &path)
            .env("ARENA_CONFIG", &config_path)
            .env("ARENA_SERVER", server)
            .env("ARENA_TOKEN", "participant-token")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };

    tokio::task::spawn_blocking(move || {
        let build = invoke(&["build", "--", "gcc", "main.c"], &server);
        assert!(build.starts_with("run\n--rm\n--platform\nlinux/arm64\n"));
        assert!(build.ends_with("gcc\nmain.c\n"));
        let pull = invoke(&["pull"], &server);
        assert!(pull.starts_with("pull\n--platform\nlinux/arm64\n"));

        let offline = invoke(&["pull", "--platform", "linux/amd64"], "http://127.0.0.1:1");
        assert!(offline.starts_with("pull\n--platform\nlinux/amd64\n"));
        let unconfigured = invoke(&["build", "--", "gcc", "main.c"], "");
        assert!(unconfigured.starts_with("run\n--rm\n--platform\nlinux/amd64\n"));
    })
    .await
    .unwrap();
}
