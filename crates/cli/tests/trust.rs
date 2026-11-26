//! Exercise trust's verification and saved config through the CLI.
#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::{Command, Output};

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;

const CERTIFICATE: &str = include_str!("fixtures/lan-ca.pem");
const FINGERPRINT: &str = "31:A7:9B:7D:C0:87:E6:2E:27:BF:AD:EE:8E:49:C8:42:E9:2C:D3:37:5B:30:53:53:30:A3:DD:F0:B2:79:5A:7B";

async fn arena(config: &Path, args: &[&str]) -> Output {
    let config = config.to_owned();
    let args: Vec<_> = args.iter().map(|arg| (*arg).to_owned()).collect();
    tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_arena"))
            .env("ARENA_CONFIG", config)
            .env_remove("ARENA_SERVER")
            .args(args)
            .output()
            .unwrap()
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn fingerprint_verifies_one_local_certificate_before_writing() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.toml");
    let file = temp.path().join("ca.pem");
    let file_arg = file.to_str().unwrap();
    let server = "https://localhost:8443";

    std::fs::write(&file, CERTIFICATE.replace('\n', "\r\n")).unwrap();
    let wrong = "00".repeat(32);
    let output = arena(
        &config,
        &[
            "trust",
            server,
            "--ca-file",
            file_arg,
            "--fingerprint",
            &wrong,
        ],
    )
    .await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("fingerprint mismatch"));
    assert!(!config.exists());

    let expected = FINGERPRINT.replace(':', "").to_ascii_lowercase();
    let output = arena(
        &config,
        &[
            "trust",
            server,
            "--ca-file",
            file_arg,
            "--fingerprint",
            &expected,
        ],
    )
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains(FINGERPRINT));
    let saved_bytes = std::fs::read(&config).unwrap();
    let saved: arena_cli::config::Config = toml::from_slice(&saved_bytes).unwrap();
    assert_eq!(saved.server.as_deref(), Some(server));
    assert_eq!(
        saved.ca_certificate.unwrap().replace("\r\n", "\n"),
        CERTIFICATE.trim()
    );

    std::fs::write(&file, CERTIFICATE.repeat(2)).unwrap();
    let output = arena(
        &config,
        &[
            "trust",
            server,
            "--ca-file",
            file_arg,
            "--fingerprint",
            FINGERPRINT,
        ],
    )
    .await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("single PEM certificate"));
    assert_eq!(std::fs::read(&config).unwrap(), saved_bytes);
}

#[tokio::test]
async fn http_trust_checks_fingerprint_and_reports_bootstrap_failures() {
    let router = Router::new()
        .route("/arena-ca.crt", get(|| async { CERTIFICATE }))
        .route("/empty/arena-ca.crt", get(|| async { "" }))
        .route(
            "/missing/arena-ca.crt",
            get(|| async { (StatusCode::NOT_FOUND, "missing") }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let config_dir = tempfile::tempdir().unwrap();
    let config = config_dir.path().join("config.toml");

    for (path, status, bytes) in [("empty", "200 OK", 0), ("missing", "404 Not Found", 7)] {
        let server = format!("https://{address}/{path}");
        let output = arena(&config, &["trust", &server, "--fingerprint", FINGERPRINT]).await;
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains(&format!(
                "http://{address}/{path}/arena-ca.crt returned HTTP {status} ({bytes} bytes)"
            )),
            "{error}"
        );
        assert!(error.contains("--ca-file"));
        assert!(!config.exists());
    }

    let server = format!("https://{address}");
    let output = arena(&config, &["trust", &server, "--fingerprint", FINGERPRINT]).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(config.exists());
}

#[tokio::test]
async fn missing_arguments_fail_with_piped_stdin() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.toml");
    let server = "https://127.0.0.1:1";
    for (args, error) in [
        (vec!["trust", server], "missing --fingerprint"),
        (
            vec!["trust", server, "--fingerprint", "invalid"],
            "64 hex digits",
        ),
        (vec!["problem"], "missing <SLUG>"),
        (
            vec!["login", server, "--username", "alice"],
            "missing password",
        ),
    ] {
        let output = arena(&config, &args).await;
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains(error));
    }
    assert!(!config.exists());
}
