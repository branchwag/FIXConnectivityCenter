//! axum web server: live session dashboard.
//!   GET /sessions  -> JSON snapshot (one-shot, handy for curl)
//!   GET /events    -> Server-Sent Events, pushes a fresh snapshot on every change
//!   everything else -> static files (index.html, styles.css, ...)

use std::convert::Infallible;

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::services::ServeDir;

use crate::counterparty::CounterpartyControl;
use crate::fix_app::{self, SharedLastEvent, SharedStatus, Snapshot};

#[derive(Clone)]
pub struct AppState {
    pub status: SharedStatus,
    pub last_event: SharedLastEvent,
    pub events: broadcast::Sender<String>,
    pub counterparty: CounterpartyControl,
}

async fn sessions(State(state): State<AppState>) -> Json<Snapshot> {
    Json(fix_app::snapshot(
        &state.status,
        &state.last_event,
        state.counterparty.is_running(),
    ))
}

/// Push the current snapshot to SSE clients (used when a change originates
/// outside the FIX callbacks, e.g. the counterparty being started/stopped).
fn broadcast_snapshot(state: &AppState) {
    let _ = state.events.send(fix_app::snapshot_json(
        &state.status,
        &state.last_event,
        state.counterparty.is_running(),
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
    ));

    let stream = initial
        .chain(live)
        .map(|json| Ok(Event::default().data(json)));

    Sse::new(stream).keep_alive(KeepAlive::default())
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
