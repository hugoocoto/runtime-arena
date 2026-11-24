//! `GET /ws`: live events. With a participant token the connection also gets that
//! participant's submission progress; without one it is a public viewer (projector).

use std::net::{IpAddr, Ipv4Addr};

use arena_protocol::ServerEvent;
use axum::Extension;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use tokio::sync::broadcast::error::RecvError;

use crate::auth::{Participant, bearer_header, find_by_token, participant_from_headers};
use crate::error::{ApiError, ApiResult};
use crate::net_limit::ClientIp;
use crate::{AppState, Event};

pub async fn handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
    client: Option<Extension<ClientIp>>,
) -> ApiResult<Response> {
    let user = participant_from_headers(&headers, &state).await?;
    // A token that matches nobody is an error, not a silent downgrade to anonymous.
    if user.is_none() && headers.contains_key(axum::http::header::AUTHORIZATION) {
        return Err(ApiError::unauthorized());
    }
    let ip = client.map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |c| c.0.0);
    let slot = state
        .ws_limit
        .acquire(user.as_ref().map(|u| u.id), ip)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "too many live connections; close some and try again",
            )
        })?;
    // Kept to check again when the participant's tokens are revoked.
    let token = user
        .as_ref()
        .and_then(|_| bearer_header(&headers))
        .map(str::to_owned);
    Ok(ws.on_upgrade(move |socket| async move {
        let _slot = slot;
        serve(socket, state, user, token).await;
    }))
}

/// Whether `token` still belongs to participant `id`.
async fn still_valid(state: &AppState, token: &str, id: i64) -> bool {
    match find_by_token(state, token).await {
        Ok(found) => found.is_some_and(|p| p.id == id),
        // Can't tell: keep the connection rather than drop it on a hiccup.
        Err(e) => {
            tracing::warn!(error = %e.message, "cannot recheck a websocket token");
            true
        }
    }
}

async fn send(socket: &mut WebSocket, event: &ServerEvent) -> bool {
    match serde_json::to_string(event) {
        Ok(text) => socket.send(Message::Text(text.into())).await.is_ok(),
        Err(_) => true,
    }
}

/// The current round and its (public) boards, so a new client needs nothing else.
async fn snapshot(state: &AppState) -> Result<Vec<ServerEvent>, ApiError> {
    let mut events = Vec::new();
    let Some(round) = crate::rounds::latest(state).await? else {
        return Ok(events);
    };
    if let Some(info) = crate::rounds::current_info(state).await? {
        events.push(ServerEvent::Round { round: info });
    }
    for problem in crate::rounds::problems(state, round.id).await? {
        let board = crate::boards::board(state, &round, &problem, false).await?;
        events.push(ServerEvent::Board { board });
    }
    Ok(events)
}

async fn serve(
    mut socket: WebSocket,
    state: AppState,
    user: Option<Participant>,
    token: Option<String>,
) {
    // Subscribe first so nothing published during the snapshot is lost.
    let mut events = state.events.subscribe();
    let user_id = user.as_ref().map(|u| u.id);
    // A logout between the handler's check and the subscription published its
    // revocation before we listened, so look again now.
    if let (Some(id), Some(token)) = (user_id, &token)
        && !still_valid(&state, token, id).await
    {
        let _ = socket.send(Message::Close(None)).await;
        return;
    }
    let hello = ServerEvent::Hello {
        user: user.as_ref().map(|u| u.name.clone()),
    };
    if !send(&mut socket, &hello).await {
        return;
    }
    match snapshot(&state).await {
        Ok(initial) => {
            for e in &initial {
                if !send(&mut socket, e).await {
                    return;
                }
            }
        }
        Err(e) => tracing::error!(error = %e.message, "websocket snapshot failed"),
    }

    loop {
        tokio::select! {
            event = events.recv() => {
                let event = match event {
                    Ok(Event::Public(e)) => e,
                    Ok(Event::ForUser(id, e)) if Some(id) == user_id => e,
                    Ok(Event::ForUser(..)) => continue,
                    Ok(Event::Revoked(id)) if Some(id) == user_id => {
                        if let Some(token) = &token
                            && !still_valid(&state, token, id).await
                        {
                            let _ = socket.send(Message::Close(None)).await;
                            return;
                        }
                        continue;
                    }
                    Ok(Event::Revoked(_)) => continue,
                    // Too slow to keep up: events were lost, possibly this
                    // participant's results or a round change. Close, so the
                    // client reconnects and gets a fresh snapshot (or polls).
                    Err(RecvError::Lagged(_)) => {
                        let _ = socket.send(Message::Close(None)).await;
                        return;
                    }
                    Err(RecvError::Closed) => return,
                };
                if !send(&mut socket, &event).await {
                    return;
                }
            }
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                // Clients don't send anything we act on; pings are answered by axum.
                Some(Ok(_)) => {}
            },
        }
    }
}
