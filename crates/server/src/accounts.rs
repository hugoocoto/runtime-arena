//! Self-service accounts: sign up on `/join` or with `arena signup` using the
//! event code, then log in from the CLI with name and password.

use arena_protocol::{LoginRequest, PasswordReset, SignupRequest, UserInfo, UserToken};
use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{Extensions, HeaderMap, StatusCode};
use axum::response::Html;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::LazyLock;
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::sync::OwnedSemaphorePermit;

use crate::auth::{Admin, Participant, bearer_header, hash_token, new_token};
use crate::error::{ApiError, ApiResult};
use crate::net_limit::ClientIp;
use crate::{AppState, Event, now, valid_slug};

const MIN_PASSWORD: usize = 6;

fn wrong_login() -> ApiError {
    ApiError::unauthorized_with("wrong name or password")
}

fn too_many(what: &str, wait: Duration) -> ApiError {
    let minutes = wait.as_secs().div_ceil(60).max(1);
    let unit = if minutes == 1 { "minute" } else { "minutes" };
    ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        format!("too many {what}; try again in {minutes} {unit}"),
    )
}

/// The client's address, as [`crate::net_limit::limit_requests`] resolved it:
/// `X-Forwarded-For` counts only from a trusted proxy, since a client could set
/// it to any value and escape the limit.
fn peer_ip(extensions: &Extensions) -> IpAddr {
    extensions
        .get::<ClientIp>()
        .map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |c| c.0)
}

/// Hashes at once. Each takes about 19 MiB and a CPU core for a moment, so a
/// burst of logins can neither exhaust memory nor disturb the judge's timings.
pub(crate) const MAX_PASSWORD_HASHES: usize = 4;
const HASH_QUEUE_WAIT: Duration = Duration::from_secs(30);

async fn hash_slot(state: &AppState) -> ApiResult<OwnedSemaphorePermit> {
    let busy = || {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "the server is busy; try again shortly",
        )
    };
    tokio::time::timeout(HASH_QUEUE_WAIT, state.hash_slots.clone().acquire_owned())
        .await
        .map_err(|_| busy())?
        .map_err(|_| busy())
}

/// Hashing is deliberately slow; keep it off the async threads.
async fn hash_password(state: &AppState, password: String) -> ApiResult<String> {
    let slot = hash_slot(state).await?;
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|h| h.to_string())
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)
}

/// A hash no password is checked against for real. Names without a password
/// are checked against it, so they take as long as real ones and the timing
/// doesn't reveal which accounts exist.
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| {
    Argon2::default()
        .hash_password(b"no account has this password")
        .map(|h| h.to_string())
        .unwrap_or_default()
});

async fn verify_password(state: &AppState, password: String, hash: String) -> ApiResult<bool> {
    let slot = hash_slot(state).await?;
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        Argon2::default()
            .verify_password(password.as_bytes(), hash.as_str())
            .is_ok()
    })
    .await
    .map_err(ApiError::internal)
}

/// `POST /api/signup`: creates an account (needs `ARENA_JOIN_CODE`).
pub async fn signup(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<SignupRequest>,
) -> ApiResult<Json<UserInfo>> {
    let expected = state.config.join_code.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "sign-up is disabled on this server; ask the organiser for an account",
        )
    })?;
    let ip = peer_ip(&extensions);
    state
        .login_limit
        .begin_code(ip)
        .map_err(|wait| too_many("wrong event codes", wait))?;
    if !bool::from(req.join_code.trim().as_bytes().ct_eq(expected.as_bytes())) {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "wrong event code"));
    }
    state.login_limit.code_accepted(ip);
    if req.password.chars().count() < MIN_PASSWORD {
        return Err(ApiError::bad_request(format!(
            "the password needs at least {MIN_PASSWORD} characters"
        )));
    }
    let name = req.name.trim().to_ascii_lowercase();
    state
        .login_limit
        .begin_signup(ip)
        .map_err(|wait| too_many("sign-ups", wait))?;
    let hash = hash_password(&state, req.password).await?;
    let user = crate::users::insert(&state, name, req.display_name, &hash).await?;
    let display_name =
        sqlx::query_scalar!("SELECT display_name FROM users WHERE name = ?", user.name)
            .fetch_one(&state.db)
            .await?;
    tracing::info!(user = %user.name, "participant signed up");
    Ok(Json(UserInfo {
        name: user.name,
        display_name,
    }))
}

/// `POST /api/login`: name and password for a new device token.
pub async fn login(
    State(state): State<AppState>,
    extensions: Extensions,
    Json(req): Json<LoginRequest>,
) -> ApiResult<Json<UserToken>> {
    // Unknown names count like real ones, so a 429 doesn't reveal which exist.
    // Names that can't exist share one key, keeping the limiter's memory small.
    let name = req.name.trim().to_ascii_lowercase();
    let ip = peer_ip(&extensions);
    let limit_key = if valid_slug(&name) { name.as_str() } else { "" };
    state
        .login_limit
        .begin(limit_key, ip)
        .map_err(|wait| too_many("failed logins", wait))?;
    let row = sqlx::query!("SELECT id, password_hash FROM users WHERE name = ?", name)
        .fetch_optional(&state.db)
        .await?;
    let Some((id, Some(hash))) = row.map(|r| (r.id, r.password_hash)) else {
        let dummy = tokio::task::spawn_blocking(|| DUMMY_HASH.clone())
            .await
            .map_err(ApiError::internal)?;
        verify_password(&state, req.password, dummy).await?;
        return Err(wrong_login());
    };
    if !verify_password(&state, req.password, hash).await? {
        return Err(wrong_login());
    }
    state.login_limit.succeeded(limit_key, ip);
    let token = new_token();
    let token_hash = hash_token(&token);
    let t = now();
    sqlx::query!(
        "INSERT INTO sessions (token_hash, user_id, created_at) VALUES (?, ?, ?)",
        token_hash,
        id,
        t
    )
    .execute(&state.db)
    .await?;
    Ok(Json(UserToken { name, token }))
}

/// `POST /api/logout`: ends this device's session; other devices stay logged in.
pub async fn logout(
    user: Participant,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> ApiResult<StatusCode> {
    let token = bearer_header(&headers).ok_or_else(ApiError::unauthorized)?;
    let hash = hash_token(token);
    sqlx::query!("DELETE FROM sessions WHERE token_hash = ?", hash)
        .execute(&state.db)
        .await?;
    state.publish(Event::Revoked(user.id));
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/admin/users/{name}/password`: recovers an account with a new random
/// password. Every existing token of the participant stops working, including
/// the one of an account created by an admin in an older version.
pub async fn reset_password(
    _: Admin,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<Json<PasswordReset>> {
    let name = name.trim().to_ascii_lowercase();
    let mut bytes = [0u8; 12];
    rand::fill(&mut bytes);
    let password = hex::encode(bytes);
    let hash = hash_password(&state, password.clone()).await?;
    let token_hash = hash_token(&new_token());

    let mut tx = state.db.begin().await?;
    let updated = sqlx::query!(
        "UPDATE users SET password_hash = ?, token_hash = ? WHERE name = ?",
        hash,
        token_hash,
        name
    )
    .execute(&mut *tx)
    .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::not_found(format!("no user {name}")));
    }
    let id = sqlx::query_scalar!("SELECT id FROM users WHERE name = ?", name)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query!("DELETE FROM sessions WHERE user_id = ?", id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    state.publish(Event::Revoked(id));
    tracing::info!(user = %name, "password reset by the admin");
    Ok(Json(PasswordReset { name, password }))
}

/// `GET /join`: the sign-up page.
pub async fn page() -> Html<&'static str> {
    Html(include_str!("../../../web/join.html"))
}
