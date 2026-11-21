use arena_protocol::{CreateUser, UserInfo, UserToken};
use axum::Json;
use axum::extract::{Path, State};

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

/// `POST /api/admin/users`: creates a participant and returns their token once.
pub async fn create(
    _: Admin,
    State(state): State<AppState>,
    Json(req): Json<CreateUser>,
) -> ApiResult<Json<UserToken>> {
    Ok(Json(insert(&state, req.name, req.display_name).await?))
}

/// Creates a participant with a new token (admin creation and `/join`).
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

/// `POST /api/admin/users/{name}/token`: replaces a lost token; the old one stops working.
pub async fn reset_token(
    _: Admin,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<Json<UserToken>> {
    let token = new_token();
    let hash = hash_token(&token);
    let updated = sqlx::query!("UPDATE users SET token_hash = ? WHERE name = ?", hash, name)
        .execute(&state.db)
        .await?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::not_found(format!("no user {name}")));
    }
    // A reset also logs out every device the participant logged in from.
    sqlx::query!(
        "DELETE FROM sessions WHERE user_id = (SELECT id FROM users WHERE name = ?)",
        name
    )
    .execute(&state.db)
    .await?;
    Ok(Json(UserToken { name, token }))
}
