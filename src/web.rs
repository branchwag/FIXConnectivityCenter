//! axum web server: live session dashboard.
//!   GET  /sessions            -> JSON snapshot (one-shot, handy for curl)
//!   GET  /events              -> Server-Sent Events, pushes a snapshot on every change
//!   POST /sessions/start      -> enable (logon) a session,  ?id=<session-id>
//!   POST /sessions/disconnect -> disable (logout) a session, ?id=<session-id>
//!   GET  /sessions/log        -> tail a session's log file,  ?id=<session-id>&offset=<u64>
//!   GET  /config              -> JSON dashboard config (environment label)
//!   /tools/testcounterparty*      -> in-app test counterparty control
//!   everything else           -> static files served from ./static (index.html, styles.css, ...)

use std::convert::Infallible;
use std::io::{Read, Seek, SeekFrom};

use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use quickfix::{Session, SessionId};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::services::ServeDir;

use crate::testcounterparty::TestCounterpartyControl;
use crate::fix_app::{self, Directions, SharedLastEvent, SharedStarted, SharedStatus, Snapshot};
use crate::logger;
use crate::metrics::{self, MetricsState};
use crate::send;

#[derive(Clone)]
pub struct AppState {
    pub status: SharedStatus,
    pub last_event: SharedLastEvent,
    pub started: SharedStarted,
    pub directions: Directions,
    pub events: broadcast::Sender<String>,
    pub testcounterparty: TestCounterpartyControl,
    pub metrics: MetricsState,
}

async fn sessions(State(state): State<AppState>) -> Json<Snapshot> {
    Json(fix_app::snapshot(
        &state.status,
        &state.last_event,
        state.testcounterparty.is_running(),
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
        state.testcounterparty.is_running(),
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
        state.testcounterparty.is_running(),
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

// --- Per-session log tail ---

/// First-view window (bytes read from the end when the client has no offset).
const TAIL_BYTES: u64 = 64 * 1024;
/// Cap on how much a single incremental poll returns (bytes).
const MAX_CHUNK: u64 = 256 * 1024;

#[derive(Deserialize)]
struct LogQuery {
    id: String,
    offset: Option<u64>,
}

/// Read a slice of a session's log for the tail view.
///
/// Returns `(text, new_offset, reset)`:
/// - `offset == None` (first view): the last [`TAIL_BYTES`], trimmed to whole lines.
/// - `offset == Some(o)` and `o <= size`: bytes since `o` (capped at [`MAX_CHUNK`]).
/// - `offset == Some(o)` and `o > size`: the file rotated/shrank — fresh tail, `reset = true`.
///
/// Missing or unreadable files yield `("", 0, false)`. The path comes from
/// [`logger::session_log_path`], which confines it to the log dir.
fn read_log_tail(id: &str, offset: Option<u64>) -> (String, u64, bool) {
    let empty = (String::new(), 0, false);
    let Some(path) = logger::session_log_path(id) else {
        return empty;
    };
    let Ok(mut file) = std::fs::File::open(&path) else {
        return empty;
    };
    let Ok(size) = file.metadata().map(|m| m.len()) else {
        return empty;
    };

    let (start, reset, trim_partial) = match offset {
        None => (size.saturating_sub(TAIL_BYTES), false, true),
        Some(o) if o > size => (size.saturating_sub(TAIL_BYTES), true, true),
        Some(o) => (o.max(size.saturating_sub(MAX_CHUNK)), false, false),
    };

    if start >= size {
        return (String::new(), size, reset);
    }
    if file.seek(SeekFrom::Start(start)).is_err() {
        return empty;
    }
    let mut buf = Vec::with_capacity((size - start) as usize);
    if file.take(size - start).read_to_end(&mut buf).is_err() {
        return empty;
    }

    let mut text = String::from_utf8_lossy(&buf).into_owned();
    // When starting mid-file, drop the leading partial line.
    if trim_partial && start > 0 {
        if let Some(nl) = text.find('\n') {
            text.drain(..=nl);
        }
    }
    (text, size, reset)
}

async fn session_log(Query(q): Query<LogQuery>) -> Json<serde_json::Value> {
    let (data, offset, reset) = read_log_tail(&q.id, q.offset);
    Json(json!({ "data": data, "offset": offset, "reset": reset }))
}

// --- Tools: in-app test counterparty (FIX acceptor) ---

// Environment label for the dashboard banner, from `FIX_ENVIRONMENT`
// (defaults to "UNKNOWN" when unset or empty). Read per-request so it always
// reflects the current process environment.
async fn config() -> Json<serde_json::Value> {
    let environment = std::env::var("FIX_ENVIRONMENT")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "UNKNOWN".to_string());
    Json(json!({ "environment": environment }))
}

async fn testcounterparty_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({ "running": state.testcounterparty.is_running() }))
}

async fn testcounterparty_start(State(state): State<AppState>) -> Json<serde_json::Value> {
    let running = state.testcounterparty.start();
    broadcast_snapshot(&state); // push the new running state to all SSE clients
    Json(json!({ "running": running }))
}

async fn testcounterparty_stop(State(state): State<AppState>) -> Json<serde_json::Value> {
    let running = state.testcounterparty.stop();
    broadcast_snapshot(&state);
    Json(json!({ "running": running }))
}

// --- Tools: host machine-health metrics ---

/// Current host resource snapshot (CPU, memory, swap, disk, load, uptime).
/// Polled by the machine-health page; served from the background-refreshed
/// [`MetricsState`] so the response is a cheap read.
async fn metrics(State(state): State<AppState>) -> Json<metrics::Metrics> {
    Json(state.metrics.snapshot())
}

// --- Tools: manual message send ---

#[derive(Deserialize)]
struct SendQuery {
    id: String,
}

/// Resolve a session id string to a `SessionId`, but only if that session is
/// currently connected — sending on a down session is rejected so the operator
/// gets a clear error rather than a silently-queued message.
fn resolve_connected_session(state: &AppState, id: &str) -> Result<SessionId, String> {
    let sid = fix_app::parse_session_id(id).ok_or_else(|| format!("invalid session id: {id}"))?;
    let connected = state.status.lock().unwrap().get(id).copied().unwrap_or(false);
    if !connected {
        return Err(format!("session {id} is not connected"));
    }
    Ok(sid)
}

/// Send a single pasted message (raw FIX or JSON) on `?id=<session>`.
async fn send_message(
    State(state): State<AppState>,
    Query(q): Query<SendQuery>,
    body: String,
) -> Json<serde_json::Value> {
    match resolve_connected_session(&state, &q.id) {
        Err(e) => Json(json!({ "ok": false, "error": e })),
        Ok(sid) => {
            let r = send::send_single(&body, &sid);
            Json(json!({ "ok": r.ok, "error": r.error }))
        }
    }
}

/// Send a CSV batch (one message per row) on `?id=<session>`. The request body
/// is the CSV text.
async fn send_csv(
    State(state): State<AppState>,
    Query(q): Query<SendQuery>,
    body: String,
) -> Json<serde_json::Value> {
    match resolve_connected_session(&state, &q.id) {
        Err(e) => Json(json!({ "ok": false, "error": e, "sent": 0, "total": 0, "errors": [] })),
        Ok(sid) => {
            let r = send::send_csv(&body, &sid);
            Json(json!({
                "ok": r.ok,
                "sent": r.sent,
                "total": r.total,
                "errors": r.errors,
            }))
        }
    }
}

pub async fn serve(state: AppState) {
    // Keep the host metrics fresh in the background so /tools/metrics is a cheap
    // read and CPU usage reflects a real sampling interval.
    metrics::spawn_refresher(state.metrics.clone());

    let app = Router::new()
        .route("/sessions", get(sessions))
        .route("/events", get(events))
        .route("/sessions/start", post(session_start))
        .route("/sessions/disconnect", post(session_disconnect))
        .route("/sessions/log", get(session_log))
        .route("/config", get(config))
        .route("/tools/testcounterparty", get(testcounterparty_status))
        .route("/tools/testcounterparty/start", post(testcounterparty_start))
        .route("/tools/testcounterparty/stop", post(testcounterparty_stop))
        .route("/tools/metrics", get(metrics))
        .route("/tools/send", post(send_message))
        .route("/tools/send/csv", post(send_csv))
        .with_state(state)
        .fallback_service(ServeDir::new("static"));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8081")
        .await
        .expect("failed to bind :8081");
    println!("Server starting on http://:8081");
    axum::serve(listener, app).await.expect("web server error");
}
