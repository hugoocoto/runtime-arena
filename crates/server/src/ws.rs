//! `GET /ws`: live events. With a participant token the connection also gets that
//! participant's submission progress; without one it is a public viewer (projector).

use arena_protocol::ServerEvent;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::Response;
use tokio::sync::broadcast::error::RecvError;

use crate::auth::{Participant, participant_from_headers};
use crate::error::{ApiError, ApiResult};
use crate::{AppState, Event};

pub async fn handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let user = participant_from_headers(&headers, &state).await?;
    // A token that matches nobody is an error, not a silent downgrade to anonymous.
    if user.is_none() && headers.contains_key(axum::http::header::AUTHORIZATION) {
        return Err(ApiError::unauthorized());
    }
    Ok(ws.on_upgrade(move |socket| serve(socket, state, user)))
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

async fn serve(mut socket: WebSocket, state: AppState, user: Option<Participant>) {
    // Subscribe first so nothing published during the snapshot is lost.
    let mut events = state.events.subscribe();
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

    let user_id = user.map(|u| u.id);
    loop {
        tokio::select! {
            event = events.recv() => {
                let event = match event {
                    Ok(Event::Public(e)) => e,
                    Ok(Event::ForUser(id, e)) if Some(id) == user_id => e,
                    Ok(Event::ForUser(..)) => continue,
                    // Too slow to keep up: skip ahead; the next board snapshot is complete anyway.
                    Err(RecvError::Lagged(_)) => continue,
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
