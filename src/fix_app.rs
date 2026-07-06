//! FIX application callbacks: session status tracking + inbound message streaming.

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use prost::Message as _;
use quickfix::{ApplicationCallback, FieldMap, Message, MsgFromAppError, SessionId};

use crate::proto;

/// Session id string -> connected?  (shared with the web layer).
pub type SharedStatus = Arc<Mutex<HashMap<String, bool>>>;

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
}

impl FixApp {
    pub fn new(session_status: SharedStatus, logged_on: SharedSession) -> Self {
        Self {
            session_status,
            logged_on,
        }
    }
}

impl ApplicationCallback for FixApp {
    fn on_create(&self, session: &SessionId) {
        self.session_status
            .lock()
            .unwrap()
            .insert(session.to_repr(), false);
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
        println!("Session {} has logged on.", session.to_repr());
    }

    fn on_logout(&self, session: &SessionId) {
        self.session_status
            .lock()
            .unwrap()
            .insert(session.to_repr(), false);
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
