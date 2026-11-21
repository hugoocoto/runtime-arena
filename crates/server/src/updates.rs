//! Tells the organiser when a newer release exists. It only notifies: the server
//! never downloads or installs anything, and a failed check changes nothing
//! (contests often run on networks without internet access).

use std::sync::PoisonError;
use std::time::Duration;

use arena_protocol::UpdateInfo;
use axum::Json;
use axum::extract::State;
use serde::Deserialize;

use crate::AppState;
use crate::auth::Admin;

const LATEST_RELEASE: &str = "https://api.github.com/repos/hugoocoto/runtime-arena/releases/latest";
const CURRENT: &str = env!("CARGO_PKG_VERSION");
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const TIMEOUT: Duration = Duration::from_secs(10);
/// Largest GitHub response read.
const MAX_RESPONSE: usize = 1024 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

/// Checks at startup and then periodically.
pub async fn clock(state: AppState) {
    let client = match client() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "update check disabled");
            return;
        }
    };
    let mut interval = tokio::time::interval(CHECK_EVERY);
    loop {
        interval.tick().await;
        match fetch_latest(&client).await {
            Ok(latest) => record(&state, latest),
            Err(e) => tracing::info!(error = %e, "update check failed"),
        }
    }
}

/// Uses the bundled root certificates: the server image has no CA store.
fn client() -> Result<reqwest::Client, reqwest::Error> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let roots = webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .filter_map(|c| reqwest::Certificate::from_der(c.as_ref()).ok());
    reqwest::Client::builder()
        .user_agent("runtime-arena-server")
        .https_only(true)
        .timeout(TIMEOUT)
        .tls_certs_only(roots)
        .build()
}

async fn fetch_latest(client: &reqwest::Client) -> Result<String, String> {
    let mut response = client
        .get(LATEST_RELEASE)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > MAX_RESPONSE {
            return Err("response too large".into());
        }
        body.extend_from_slice(&chunk);
    }
    let release: Release = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    let (major, minor, patch) =
        parse_version(&release.tag_name).ok_or("release tag is not a version")?;
    Ok(format!("{major}.{minor}.{patch}"))
}

fn record(state: &AppState, latest: String) {
    let mut saved = state
        .latest_version
        .write()
        .unwrap_or_else(PoisonError::into_inner);
    if saved.as_deref() != Some(latest.as_str()) && is_newer(&latest, CURRENT) {
        tracing::warn!(
            current = CURRENT,
            latest = %latest,
            "a newer version is available; update with `docker compose pull && docker compose up -d`"
        );
    }
    *saved = Some(latest);
}

/// `GET /api/admin/update`
pub async fn status(_: Admin, State(state): State<AppState>) -> Json<UpdateInfo> {
    let latest = state
        .latest_version
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Json(update_info(CURRENT, latest))
}

fn update_info(current: &str, latest: Option<String>) -> UpdateInfo {
    let update_available = latest.as_deref().is_some_and(|l| is_newer(l, current));
    let release_url = latest
        .as_deref()
        .filter(|_| update_available)
        .map(|l| format!("{}/releases/tag/v{l}", env!("CARGO_PKG_REPOSITORY")));
    UpdateInfo {
        current: current.to_owned(),
        latest,
        update_available,
        release_url,
    }
}

fn is_newer(candidate: &str, current: &str) -> bool {
    matches!(
        (parse_version(candidate), parse_version(current)),
        (Some(a), Some(b)) if a > b
    )
}

/// `1.2.3` or `v1.2.3`; anything else (such as a pre-release) is not a version.
fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).split('.').map(|p| {
        p.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| p.parse().ok())
            .flatten()
    });
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.4.0", "0.4.0"));
        assert!(!is_newer("0.3.9", "0.4.0"));
    }

    #[test]
    fn malformed_tags_are_never_newer() {
        for tag in [
            "",
            "latest",
            "v1.2",
            "1.2.3.4",
            "9.9.9-rc1",
            "+9.0.0",
            "9.0.x",
        ] {
            assert_eq!(parse_version(tag), None, "{tag}");
            assert!(!is_newer(tag, "0.1.0"), "{tag}");
        }
    }

    #[test]
    fn update_info_links_only_a_newer_release() {
        let newer = update_info("0.3.0", Some("0.4.0".into()));
        assert!(newer.update_available);
        assert!(newer.release_url.unwrap().ends_with("/releases/tag/v0.4.0"));
        let same = update_info("0.4.0", Some("0.4.0".into()));
        assert!(!same.update_available);
        assert_eq!(same.release_url, None);
        let unknown = update_info("0.4.0", None);
        assert!(!unknown.update_available);
    }
}
