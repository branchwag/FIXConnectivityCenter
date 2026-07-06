//! axum web server: live session dashboard.
//!   GET /sessions  -> JSON snapshot (one-shot, handy for curl)
//!   GET /events    -> Server-Sent Events, pushes a fresh snapshot on every change
//!   everything else -> static files (index.html, styles.css, ...)

use std::convert::Infallible;

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Json, Router,
};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::services::ServeDir;

use crate::fix_app::{self, SharedLastEvent, SharedStatus, Snapshot};

#[derive(Clone)]
pub struct AppState {
    pub status: SharedStatus,
    pub last_event: SharedLastEvent,
    pub events: broadcast::Sender<String>,
}

async fn sessions(State(state): State<AppState>) -> Json<Snapshot> {
    Json(fix_app::snapshot(&state.status, &state.last_event))
}

async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Subscribe first, then read the current snapshot, so no change can slip
    // through between the two. Each event carries the full session list, so a
    // dropped/lagged message is harmless — the next one is authoritative.
    let live = BroadcastStream::new(state.events.subscribe()).filter_map(|r| r.ok());
    let initial = tokio_stream::once(fix_app::snapshot_json(&state.status, &state.last_event));

    let stream = initial
        .chain(live)
        .map(|json| Ok(Event::default().data(json)));

    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn serve(state: AppState) {
    let app = Router::new()
        .route("/sessions", get(sessions))
        .route("/events", get(events))
        .with_state(state)
        .fallback_service(ServeDir::new("."));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8081")
        .await
        .expect("failed to bind :8081");
    println!("Server starting on http://:8081");
    axum::serve(listener, app).await.expect("web server error");
}
