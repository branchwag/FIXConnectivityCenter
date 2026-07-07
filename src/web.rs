//! axum web server: live session dashboard.
//!   GET  /sessions            -> JSON snapshot (one-shot, handy for curl)
//!   GET  /events              -> Server-Sent Events, pushes a snapshot on every change
//!   POST /sessions/start      -> enable (logon) a session,  ?id=<session-id>
//!   POST /sessions/disconnect -> disable (logout) a session, ?id=<session-id>
//!   /tools/counterparty*      -> in-app test counterparty control
//!   everything else           -> static files (index.html, styles.css, ...)

use std::convert::Infallible;

use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use quickfix::Session;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::services::ServeDir;

use crate::counterparty::CounterpartyControl;
use crate::fix_app::{self, Directions, SharedLastEvent, SharedStarted, SharedStatus, Snapshot};

#[derive(Clone)]
pub struct AppState {
    pub status: SharedStatus,
    pub last_event: SharedLastEvent,
    pub started: SharedStarted,
    pub directions: Directions,
    pub events: broadcast::Sender<String>,
    pub counterparty: CounterpartyControl,
}

async fn sessions(State(state): State<AppState>) -> Json<Snapshot> {
    Json(fix_app::snapshot(
        &state.status,
        &state.last_event,
        state.counterparty.is_running(),
        &state.started,
        &state.directions,
    ))
}

/// Push the current snapshot to SSE clients (used when a change originates
/// outside the FIX callbacks, e.g. the counterparty or a session being toggled).
fn broadcast_snapshot(state: &AppState) {
    let _ = state.events.send(fix_app::snapshot_json(
        &state.status,
        &state.last_event,
        state.counterparty.is_running(),
        &state.started,
        &state.directions,
    ));
}

async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Subscribe first, then read the current snapshot, so no change can slip
    // through between the two. Each event carries the full session list, so a
    // dropped/lagged message is harmless — the next one is authoritative.
    let live = BroadcastStream::new(state.events.subscribe()).filter_map(|r| r.ok());
    let initial = tokio_stream::once(fix_app::snapshot_json(
        &state.status,
        &state.last_event,
        state.counterparty.is_running(),
        &state.started,
        &state.directions,
    ));

    let stream = initial
        .chain(live)
        .map(|json| Ok(Event::default().data(json)));

    Sse::new(stream).keep_alive(KeepAlive::default())
}

// --- Per-session control ---

#[derive(Deserialize)]
struct SessionQuery {
    id: String,
}

/// Enable (`logon`) or disable (`logout`) a single session by its id string.
fn set_session_enabled(state: &AppState, id: &str, enabled: bool) {
    if let Some(sid) = fix_app::parse_session_id(id) {
        // SAFETY: the initiator that owns this session runs for the life of the
        // process, so the looked-up session outlives this call.
        match unsafe { Session::lookup(&sid) } {
            Ok(mut session) => {
                let result = if enabled {
                    session.logon()
                } else {
                    session.logout()
                };
                if let Err(e) = result {
                    eprintln!("Session {id} {}: {e}", if enabled { "logon" } else { "logout" });
                }
            }
            Err(e) => eprintln!("Session {id} not found: {e}"),
        }
    }
    state.started.lock().unwrap().insert(id.to_string(), enabled);
    broadcast_snapshot(state);
}

async fn session_start(
    State(state): State<AppState>,
    Query(q): Query<SessionQuery>,
) -> Json<serde_json::Value> {
    set_session_enabled(&state, &q.id, true);
    Json(json!({ "started": true }))
}

async fn session_disconnect(
    State(state): State<AppState>,
    Query(q): Query<SessionQuery>,
) -> Json<serde_json::Value> {
    set_session_enabled(&state, &q.id, false);
    Json(json!({ "started": false }))
}

// --- Tools: in-app test counterparty (FIX acceptor) ---

async fn counterparty_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "running": state.counterparty.is_running() }))
}

async fn counterparty_start(State(state): State<AppState>) -> Json<serde_json::Value> {
    let running = state.counterparty.start();
    broadcast_snapshot(&state); // push the new running state to all SSE clients
    Json(json!({ "running": running }))
}

async fn counterparty_stop(State(state): State<AppState>) -> Json<serde_json::Value> {
    let running = state.counterparty.stop();
    broadcast_snapshot(&state);
    Json(json!({ "running": running }))
}

pub async fn serve(state: AppState) {
    let app = Router::new()
        .route("/sessions", get(sessions))
        .route("/events", get(events))
        .route("/sessions/start", post(session_start))
        .route("/sessions/disconnect", post(session_disconnect))
        .route("/tools/counterparty", get(counterparty_status))
        .route("/tools/counterparty/start", post(counterparty_start))
        .route("/tools/counterparty/stop", post(counterparty_stop))
        .with_state(state)
        .fallback_service(ServeDir::new("."));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8081")
        .await
        .expect("failed to bind :8081");
    println!("Server starting on http://:8081");
    axum::serve(listener, app).await.expect("web server error");
}
