//! Test helpers: an app over a temporary data directory, driven with `oneshot`.

#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use arena_server::{AppState, Config, JudgeBackend, app, open_db};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

pub const ADMIN: &str = "test-admin-token-0123456789abcdef";
pub const JOIN_CODE: &str = "blue-falcon";

pub struct TestServer {
    pub app: Router,
    pub state: AppState,
    _data: tempfile::TempDir,
}

impl TestServer {
    pub async fn new() -> Self {
        Self::with_judge(Arc::new(FakeJudge)).await
    }

    pub async fn with_config(change: impl FnOnce(&mut Config)) -> Self {
        Self::build(Arc::new(FakeJudge), change).await
    }

    pub async fn with_judge(judge: Arc<dyn JudgeBackend>) -> Self {
        Self::build(judge, |_| {}).await
    }

    async fn build(judge: Arc<dyn JudgeBackend>, change: impl FnOnce(&mut Config)) -> Self {
        let data = tempfile::tempdir().unwrap();
        let mut config = Config {
            listen: "127.0.0.1:0".parse().unwrap(),
            data_dir: data.path().to_path_buf(),
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
        change(&mut config);
        let db = open_db(&config.data_dir).await.unwrap();
        let state = AppState::new(db, config, judge);
        TestServer {
            app: app(state.clone()),
            state,
            _data: data,
        }
    }

    /// Signs up `name` and logs in, returning the session token.
    pub async fn participant(&self, name: &str) -> String {
        let password = "secret1";
        let signup =
            serde_json::json!({"name": name, "password": password, "join_code": JOIN_CODE});
        let (status, body) = self.post("/api/signup", None, signup).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let login = serde_json::json!({"name": name, "password": password});
        let (status, body) = self.post("/api/login", None, login).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["token"].as_str().unwrap().to_owned()
    }

    pub async fn request(&self, req: Request<Body>) -> (StatusCode, Value) {
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        (status, body)
    }

    fn builder(method: &str, uri: &str, token: Option<&str>) -> axum::http::request::Builder {
        let mut b = Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        b
    }

    pub async fn get(&self, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
        self.request(
            Self::builder("GET", uri, token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
    }

    pub async fn post(&self, uri: &str, token: Option<&str>, json: Value) -> (StatusCode, Value) {
        let req = Self::builder("POST", uri, token)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json.to_string()))
            .unwrap();
        self.request(req).await
    }

    /// Multipart `POST /api/submissions`.
    pub async fn submit(
        &self,
        token: &str,
        problem: &str,
        executable: Vec<u8>,
    ) -> (StatusCode, Value) {
        let boundary = "arena-test-boundary";
        let mut body = Vec::new();
        let mut part = |name: &str, data: &[u8]| {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{name}\"\r\n\r\n"
                )
                .as_bytes(),
            );
            body.extend_from_slice(data);
            body.extend_from_slice(b"\r\n");
        };
        part("problem", problem.as_bytes());
        part("executable", &executable);
        part("source", b"int main() {}");
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        let req = Self::builder("POST", "/api/submissions", Some(token))
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        self.request(req).await
    }

    pub async fn upload_problem(&self, slug: &str, archive: Vec<u8>) -> (StatusCode, Value) {
        let req = Self::builder("PUT", &format!("/api/admin/problems/{slug}"), Some(ADMIN))
            .header(header::CONTENT_TYPE, "application/gzip")
            .body(Body::from(archive))
            .unwrap();
        self.request(req).await
    }
}

fn fixtures() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../judge/tests/fixtures"
    ))
}

/// `.tar.gz` of a judge fixture problem, with its files at the archive root.
pub fn problem_archive(name: &str) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tar = tar::Builder::new(gz);
    tar.append_dir_all(".", fixtures().join(name)).unwrap();
    tar.into_inner().unwrap().finish().unwrap()
}

/// A valid problem plus a symlink pointing outside it.
pub fn archive_with_symlink() -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tar = tar::Builder::new(gz);
    tar.append_dir_all(".", fixtures().join("sum")).unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    tar.append_link(&mut header, "tests/03.in", "/etc/passwd")
        .unwrap();
    tar.into_inner().unwrap().finish().unwrap()
}

/// Judges by reading `verdict=`, `time=` and `memory=` from the submitted file
/// instead of running it, e.g. `#!/bin/sh\n# verdict=accepted time=30 memory=900`.
/// `verdict=crash` makes the judge itself fail.
#[derive(Default)]
pub struct FakeJudge;

impl JudgeBackend for FakeJudge {
    fn judge<'a>(
        &'a self,
        _box_id: u32,
        problem: &'a arena_judge::Problem,
        executable: &'a Path,
        mut on_test: arena_server::backend::ProgressFn<'a>,
    ) -> arena_server::backend::JudgeFuture<'a> {
        Box::pin(async move {
            let text = std::fs::read_to_string(executable).map_err(|e| e.to_string())?;
            let field = |key: &str| {
                text.split_whitespace()
                    .find_map(|w| w.strip_prefix(&format!("{key}=")).map(str::to_owned))
            };
            let verdict = match field("verdict").as_deref() {
                Some("crash") => return Err("fake judge failure".into()),
                Some(v) => serde_json::from_value(Value::String(v.into())).unwrap(),
                None => arena_protocol::Verdict::Accepted,
            };
            let time_us = field("time").map_or(10, |t| t.parse().unwrap());
            let memory_kib = field("memory").map_or(1000, |m| m.parse().unwrap());
            let mut tests = Vec::new();
            for (i, t) in problem.tests.iter().enumerate() {
                let r = arena_protocol::TestResult {
                    name: t.name.clone(),
                    verdict,
                    time_us,
                    wall_time_us: time_us,
                    memory_kib,
                    message: None,
                };
                on_test(i, &r);
                tests.push(r);
            }
            Ok(arena_protocol::JudgeReport::from_tests(tests))
        })
    }
}

/// A shebang "executable" the fake judge understands.
pub fn fake_solution(spec: &str) -> Vec<u8> {
    format!("#!/bin/sh\n# {spec}\n").into_bytes()
}
