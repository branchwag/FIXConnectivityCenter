//! FIX application callbacks: session status tracking + inbound message streaming.

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use prost::Message as _;
use quickfix::{ApplicationCallback, FieldMap, Message, MsgFromAppError, SessionId};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::counterparty::CounterpartyControl;
use crate::proto;

/// Session id string -> connected?  (shared with the web layer).
pub type SharedStatus = Arc<Mutex<HashMap<String, bool>>>;

/// Epoch-millis of the last real connect/disconnect (shared with the web layer).
/// Server-owned so the dashboard's "last session event" time doesn't move on a
/// page refresh — only when a session actually logs on or out.
pub type SharedLastEvent = Arc<Mutex<Option<u64>>>;

/// Session id string -> enabled?  Absent means enabled (sessions auto-connect by
/// default). Set false by a manual Disconnect, true by a manual Start.
pub type SharedStarted = Arc<Mutex<HashMap<String, bool>>>;

/// Session id string -> `ConnectionType` ("initiator" | "acceptor"), read once
/// from `sessions.cfg` at startup.
pub type Directions = Arc<HashMap<String, String>>;

/// Rebuild a `SessionId` from its string form ("FIX.4.2:FIXDEV->TEST").
pub fn parse_session_id(repr: &str) -> Option<SessionId> {
    let (begin, rest) = repr.split_once(':')?;
    let (sender, target) = rest.split_once("->")?;
    SessionId::try_new(begin, sender, target, "").ok()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// One row of the dashboard.
#[derive(Serialize)]
pub struct SessionDetail {
    #[serde(rename = "SessionID")]
    pub session_id: String,
    #[serde(rename = "Status")]
    pub status: String,
    /// "initiator" | "acceptor" (from config) — drives Connecting vs Listening.
    pub direction: String,
    /// Whether the session is enabled (auto-connecting / logged on) vs manually
    /// disconnected.
    pub started: bool,
}

/// Full dashboard payload for `/sessions` and the SSE stream: every session the
/// engine knows about (sorted by id for a stable order), the time of the last
/// actual connect/disconnect, and whether the test counterparty is running.
#[derive(Serialize)]
pub struct Snapshot {
    pub sessions: Vec<SessionDetail>,
    #[serde(rename = "lastEventAt")]
    pub last_event_at: Option<u64>,
    #[serde(rename = "counterpartyRunning")]
    pub counterparty_running: bool,
}

pub fn snapshot(
    status: &SharedStatus,
    last_event: &SharedLastEvent,
    counterparty_running: bool,
    started: &SharedStarted,
    directions: &HashMap<String, String>,
) -> Snapshot {
    // Lock order: started before status (the control endpoints only take started).
    let started_map = started.lock().unwrap();
    let status_map = status.lock().unwrap();
    let mut sessions: Vec<SessionDetail> = status_map
        .iter()
        .map(|(id, &connected)| SessionDetail {
            session_id: id.clone(),
            status: if connected { "Connected" } else { "Disconnected" }.to_string(),
            direction: directions
                .get(id)
                .cloned()
                .unwrap_or_else(|| "initiator".to_string()),
            started: started_map.get(id).copied().unwrap_or(true),
        })
        .collect();
    drop(status_map);
    drop(started_map);
    sessions.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    Snapshot {
        sessions,
        last_event_at: *last_event.lock().unwrap(),
        counterparty_running,
    }
}

/// The same snapshot as a JSON string (used as the SSE event payload).
pub fn snapshot_json(
    status: &SharedStatus,
    last_event: &SharedLastEvent,
    counterparty_running: bool,
    started: &SharedStarted,
    directions: &HashMap<String, String>,
) -> String {
    serde_json::to_string(&snapshot(
        status,
        last_event,
        counterparty_running,
        started,
        directions,
    ))
    .unwrap_or_else(
        |_| "{\"sessions\":[],\"lastEventAt\":null,\"counterpartyRunning\":false}".to_string(),
    )
}

/// The pieces of the last logged-on session, used to rebuild a `SessionId` for
/// sending. Stored as plain strings so it can cross the thread boundary.
#[derive(Clone)]
pub struct SessionKey {
    pub begin_string: String,
    pub sender_comp_id: String,
    pub target_comp_id: String,
}

pub type SharedSession = Arc<Mutex<Option<SessionKey>>>;

pub struct FixApp {
    session_status: SharedStatus,
    logged_on: SharedSession,
    last_event: SharedLastEvent,
    started: SharedStarted,
    directions: Directions,
    events: broadcast::Sender<String>,
    counterparty: CounterpartyControl,
}

impl FixApp {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_status: SharedStatus,
        logged_on: SharedSession,
        last_event: SharedLastEvent,
        started: SharedStarted,
        directions: Directions,
        events: broadcast::Sender<String>,
        counterparty: CounterpartyControl,
    ) -> Self {
        Self {
            session_status,
            logged_on,
            last_event,
            started,
            directions,
            events,
            counterparty,
        }
    }

    /// Push the current session snapshot to any connected SSE clients. Called
    /// after every status change. Errors (no subscribers) are ignored.
    fn broadcast(&self) {
        let _ = self.events.send(snapshot_json(
            &self.session_status,
            &self.last_event,
            self.counterparty.is_running(),
            &self.started,
            &self.directions,
        ));
    }

    /// Record the wall-clock time of a real connect/disconnect. Not called on
    /// session creation — only logon/logout count as "session events".
    fn mark_event(&self) {
        *self.last_event.lock().unwrap() = Some(now_ms());
    }
}

impl ApplicationCallback for FixApp {
    fn on_create(&self, session: &SessionId) {
        self.session_status
            .lock()
            .unwrap()
            .insert(session.to_repr(), false);
        self.broadcast();
        println!("Session {} created.", session.to_repr());
    }

    fn on_logon(&self, session: &SessionId) {
        self.session_status
            .lock()
            .unwrap()
            .insert(session.to_repr(), true);
        *self.logged_on.lock().unwrap() = Some(SessionKey {
            begin_string: session.get_begin_string().unwrap_or_default(),
            sender_comp_id: session.get_sender_comp_id().unwrap_or_default(),
            target_comp_id: session.get_target_comp_id().unwrap_or_default(),
        });
        self.mark_event();
        self.broadcast();
        println!("Session {} has logged on.", session.to_repr());
    }

    fn on_logout(&self, session: &SessionId) {
        self.session_status
            .lock()
            .unwrap()
            .insert(session.to_repr(), false);
        self.mark_event();
        self.broadcast();
        println!("Session {} has logged out.", session.to_repr());
    }

    fn on_msg_to_admin(&self, msg: &mut Message, session: &SessionId) {
        if msg.with_header(|h| h.get_field(35)).as_deref() == Some("A") {
            println!("Logon message sent for session {}", session.to_repr());
        }
    }

    fn on_msg_from_app(&self, msg: &Message, session: &SessionId) -> Result<(), MsgFromAppError> {
        println!("on_msg_from_app called");

        let proto_msg = proto::from_fix_message(msg);
        let mut buf = Vec::new();
        if let Err(e) = proto_msg.encode(&mut buf) {
            eprintln!("Failed to encode proto message: {e}");
            return Ok(());
        }

        // Stream the encoded protobuf to the downstream TCP server, off the
        // engine's callback thread (mirrors the Go `go func()` + net.Dial).
        let session_repr = session.to_repr();
        std::thread::spawn(move || match TcpStream::connect("localhost:9090") {
            Ok(mut conn) => {
                if let Err(e) = conn.write_all(&buf) {
                    eprintln!("Failed to send protobuf msg over TCP: {e}");
                    return;
                }
                println!("FIX msg from session {session_repr} has been streamed via TCP");
            }
            Err(e) => eprintln!("Failed to connect to TCP server: {e}"),
        });

        Ok(())
    }
}
