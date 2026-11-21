use arena_protocol::{UserInfo, UserToken};
use axum::Json;
use axum::extract::State;

use crate::auth::{Admin, Participant, hash_token, new_token};
use crate::error::{ApiError, ApiResult};
use crate::{AppState, now, valid_slug};

/// `GET /api/me`
pub async fn me(user: Participant) -> Json<UserInfo> {
    Json(UserInfo {
        name: user.name,
        display_name: user.display_name,
    })
}

/// `GET /api/admin/users`
pub async fn list(_: Admin, State(state): State<AppState>) -> ApiResult<Json<Vec<UserInfo>>> {
    let users = sqlx::query_as!(
        UserInfo,
        "SELECT name, display_name FROM users ORDER BY name"
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(users))
}

/// Creates a participant for sign-up. The account token is never handed out:
/// devices authenticate with the session tokens from `/api/login`.
pub(crate) async fn insert(
    state: &AppState,
    name: String,
    display_name: Option<String>,
) -> ApiResult<UserToken> {
    if !valid_slug(&name) {
        return Err(ApiError::bad_request(
            "name must be 1-40 lowercase letters, digits or '-'",
        ));
    }
    let display_name = display_name
        .map(|d| d.trim().chars().take(60).collect::<String>())
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| name.clone());
    let token = new_token();
    let hash = hash_token(&token);
    let created_at = now();
    let inserted = sqlx::query!(
        "INSERT INTO users (name, display_name, token_hash, created_at) VALUES (?, ?, ?, ?)
         ON CONFLICT (name) DO NOTHING",
        name,
        display_name,
        hash,
        created_at
    )
    .execute(&state.db)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::conflict(format!(
            "the name {name} is already taken"
        )));
    }
    Ok(UserToken { name, token })
}
