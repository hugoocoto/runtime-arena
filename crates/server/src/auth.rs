//! Bearer-token authentication for participants and the admin.

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::AppState;
use crate::error::ApiError;

/// Prefix of participant tokens, so a leaked one is recognisable.
const TOKEN_PREFIX: &str = "arena_";

/// A new random participant token (256 bits from the OS-seeded CSPRNG).
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    format!("{TOKEN_PREFIX}{}", hex::encode(bytes))
}

/// What the database stores instead of the token.
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn bearer(parts: &Parts) -> Option<&str> {
    bearer_header(&parts.headers)
}

pub(crate) fn bearer_header(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

/// A request made with a participant's token.
#[derive(Debug, Clone)]
pub struct Participant {
    pub id: i64,
    pub name: String,
    pub display_name: String,
}

impl FromRequestParts<AppState> for Participant {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = bearer(parts).ok_or_else(ApiError::unauthorized)?;
        let hash = hash_token(token);
        sqlx::query_as!(
            Participant,
            "SELECT id, name, display_name FROM users WHERE token_hash = ?",
            hash
        )
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(ApiError::unauthorized)
    }
}

fn admin_token_matches(token: &str, state: &AppState) -> bool {
    // Compare hashes in constant time so the comparison leaks nothing about the token.
    let given = Sha256::digest(token.as_bytes());
    let expected = Sha256::digest(state.config.admin_token.as_bytes());
    bool::from(given.as_slice().ct_eq(expected.as_slice()))
}

/// Whether the request carries the admin token, for endpoints that are public
/// but show more to the admin.
pub fn is_admin(headers: &axum::http::HeaderMap, state: &AppState) -> bool {
    bearer_header(headers).is_some_and(|t| admin_token_matches(t, state))
}

/// The participant whose token is in `headers`, if any.
pub(crate) async fn participant_from_headers(
    headers: &axum::http::HeaderMap,
    state: &AppState,
) -> Result<Option<Participant>, ApiError> {
    let Some(token) = bearer_header(headers) else {
        return Ok(None);
    };
    let hash = hash_token(token);
    Ok(sqlx::query_as!(
        Participant,
        "SELECT id, name, display_name FROM users WHERE token_hash = ?",
        hash
    )
    .fetch_optional(&state.db)
    .await?)
}

/// A request made with `ARENA_ADMIN_TOKEN`.
#[derive(Debug, Clone, Copy)]
pub struct Admin;

impl FromRequestParts<AppState> for Admin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = bearer(parts).ok_or_else(ApiError::unauthorized)?;
        if admin_token_matches(token, state) {
            Ok(Admin)
        } else {
            Err(ApiError::unauthorized())
        }
    }
}

/// Either kind of authenticated caller.
#[derive(Debug, Clone)]
pub enum Caller {
    Admin,
    Participant(Participant),
}

impl FromRequestParts<AppState> for Caller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        if Admin::from_request_parts(parts, state).await.is_ok() {
            return Ok(Caller::Admin);
        }
        Participant::from_request_parts(parts, state)
            .await
            .map(Caller::Participant)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_prefixed() {
        let (a, b) = (new_token(), new_token());
        assert_ne!(a, b);
        assert!(a.starts_with(TOKEN_PREFIX));
        assert_eq!(a.len(), TOKEN_PREFIX.len() + 64);
    }

    #[test]
    fn hash_is_sha256_hex() {
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
