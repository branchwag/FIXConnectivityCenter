//! FIX application callbacks: session status tracking + inbound message streaming.

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use prost::Message as _;
use quickfix::{ApplicationCallback, FieldMap, Message, MsgFromAppError, SessionId};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::proto;

/// Session id string -> connected?  (shared with the web layer).
pub type SharedStatus = Arc<Mutex<HashMap<String, bool>>>;

/// One row of the dashboard, serialized for both `/sessions` and the SSE stream.
#[derive(Serialize)]
pub struct SessionDetail {
    #[serde(rename = "SessionID")]
    pub session_id: String,
    #[serde(rename = "Status")]
    pub status: String,
}

/// Current status of every session the engine knows about, sorted by id for a
/// stable dashboard order.
pub fn snapshot(status: &SharedStatus) -> Vec<SessionDetail> {
    let mut rows: Vec<SessionDetail> = status
        .lock()
        .unwrap()
        .iter()
        .map(|(id, &connected)| SessionDetail {
            session_id: id.clone(),
            status: if connected { "Connected" } else { "Disconnected" }.to_string(),
        })
        .collect();
    rows.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    rows
}

/// The same snapshot as a JSON string (used as the SSE event payload).
pub fn snapshot_json(status: &SharedStatus) -> String {
    serde_json::to_string(&snapshot(status)).unwrap_or_else(|_| "[]".to_string())
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
    events: broadcast::Sender<String>,
}

impl FixApp {
    pub fn new(
        session_status: SharedStatus,
        logged_on: SharedSession,
        events: broadcast::Sender<String>,
    ) -> Self {
        Self {
            session_status,
            logged_on,
            events,
        }
    }

    /// Push the current session snapshot to any connected SSE clients. Called
    /// after every status change. Errors (no subscribers) are ignored.
    fn broadcast(&self) {
        let _ = self.events.send(snapshot_json(&self.session_status));
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
        self.broadcast();
        println!("Session {} has logged on.", session.to_repr());
    }

    fn on_logout(&self, session: &SessionId) {
        self.session_status
            .lock()
            .unwrap()
            .insert(session.to_repr(), false);
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
