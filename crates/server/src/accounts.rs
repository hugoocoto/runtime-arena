//! Self-service accounts: sign up on `/join` with the event code, then log in
//! from the CLI with name and password.

use arena_protocol::{LoginRequest, SignupRequest, UserInfo, UserToken};
use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Html;
use subtle::ConstantTimeEq;

use crate::auth::{hash_token, new_token};
use crate::error::{ApiError, ApiResult};
use crate::{AppState, now};

const MIN_PASSWORD: usize = 6;

fn wrong_login() -> ApiError {
    ApiError::unauthorized_with("wrong name or password")
}

/// Hashing is deliberately slow; keep it off the async threads.
async fn hash_password(password: String) -> ApiResult<String> {
    tokio::task::spawn_blocking(move || {
        Argon2::default()
            .hash_password(password.as_bytes())
            .map(|h| h.to_string())
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::internal)
}

async fn verify_password(password: String, hash: String) -> ApiResult<bool> {
    tokio::task::spawn_blocking(move || {
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
    Json(req): Json<SignupRequest>,
) -> ApiResult<Json<UserInfo>> {
    let expected = state.config.join_code.as_deref().ok_or_else(|| {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "sign-up is disabled on this server; ask the organiser for an account",
        )
    })?;
    if !bool::from(req.join_code.trim().as_bytes().ct_eq(expected.as_bytes())) {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "wrong event code"));
    }
    if req.password.chars().count() < MIN_PASSWORD {
        return Err(ApiError::bad_request(format!(
            "the password needs at least {MIN_PASSWORD} characters"
        )));
    }
    let name = req.name.trim().to_ascii_lowercase();
    let hash = hash_password(req.password).await?;
    let user = crate::users::insert(&state, name, req.display_name).await?;
    sqlx::query!(
        "UPDATE users SET password_hash = ? WHERE name = ?",
        hash,
        user.name
    )
    .execute(&state.db)
    .await?;
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
    Json(req): Json<LoginRequest>,
) -> ApiResult<Json<UserToken>> {
    let name = req.name.trim().to_ascii_lowercase();
    let row = sqlx::query!("SELECT id, password_hash FROM users WHERE name = ?", name)
        .fetch_optional(&state.db)
        .await?;
    let Some((id, Some(hash))) = row.map(|r| (r.id, r.password_hash)) else {
        return Err(wrong_login());
    };
    if !verify_password(req.password, hash).await? {
        return Err(wrong_login());
    }
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

/// `GET /join`: the sign-up page.
pub async fn page() -> Html<&'static str> {
    Html(include_str!("../../../web/join.html"))
}
