//! In-app test testcounterparty: a FIX acceptor that mirrors the configured
//! initiator sessions (comp ids swapped) so they have something to connect to
//! for testing. Accepts logons and auto-acks orders with an ExecutionReport.
//! Controlled (start/stop) from the web UI's Tools section.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quickfix::{
    send_to_target, Acceptor, Application, ApplicationCallback, ConnectionHandler, Dictionary,
    FieldMap, FixSocketServerKind, LogFactory, MemoryMessageStoreFactory, Message, MsgFromAppError,
    SessionId, SessionSettings,
};

use crate::logger::FileLogger;

const CFG_PATH: &str = "sessions.cfg";

/// Acceptor-side callbacks: accept logons, auto-ack orders.
#[derive(Default)]
struct TestCounterparty;

impl ApplicationCallback for TestCounterparty {
    fn on_logon(&self, session: &SessionId) {
        println!("Test testcounterparty: logon from {}", session.to_repr());
    }

    fn on_msg_from_app(&self, msg: &Message, session: &SessionId) -> Result<(), MsgFromAppError> {
        // Auto-ack New Order - Single (35=D) with an ExecutionReport (35=8),
        // echoing the order's key fields back to the initiator's session.
        if msg.with_header(|h| h.get_field(35)).as_deref() != Some("D") {
            return Ok(());
        }

        let mut exec = Message::new();
        let _ = exec.with_header_mut(|h| h.set_field(35, "8"));
        for tag in [11, 55, 54, 38, 44] {
            if let Some(v) = msg.get_field(tag) {
                let _ = exec.set_field(tag, v);
            }
        }
        let _ = exec.set_field(37, "ORDER-XYZ"); // OrderID
        let _ = exec.set_field(17, "EXEC-1"); // ExecID
        let _ = exec.set_field(150, "0"); // ExecType = New
        let _ = exec.set_field(39, "0"); // OrdStatus = New

        if let Err(e) = send_to_target(exec, session) {
            eprintln!("Test testcounterparty: failed to send ExecutionReport: {e}");
        }
        Ok(())
    }
}

/// Parse `sessions.cfg` into one map per `[SESSION]` block (a tiny ini reader;
/// `[DEFAULT]` and other sections are ignored).
fn parse_sessions(cfg_path: &str) -> std::io::Result<Vec<HashMap<String, String>>> {
    let text = std::fs::read_to_string(cfg_path)?;
    let mut sessions = Vec::new();
    let mut current: Option<HashMap<String, String>> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(m) = current.take() {
                sessions.push(m);
            }
            if line[1..line.len() - 1].eq_ignore_ascii_case("SESSION") {
                current = Some(HashMap::new());
            }
            continue;
        }
        if let (Some(m), Some((k, v))) = (current.as_mut(), line.split_once('=')) {
            m.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    if let Some(m) = current.take() {
        sessions.push(m);
    }
    Ok(sessions)
}

/// session-id-repr ("FIX.4.2:FIXDEV->TEST") -> `ConnectionType`, for the
/// dashboard's per-session direction (Connecting vs Listening).
pub(crate) fn session_directions(cfg_path: &str) -> HashMap<String, String> {
    parse_sessions(cfg_path)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| {
            let key = format!(
                "{}:{}->{}",
                s.get("BeginString")?,
                s.get("SenderCompID")?,
                s.get("TargetCompID")?
            );
            let dir = s
                .get("ConnectionType")
                .cloned()
                .unwrap_or_else(|| "initiator".to_string());
            Some((key, dir))
        })
        .collect()
}

/// Build acceptor settings that mirror each configured initiator session:
/// swap comp ids, `ConnectionType=acceptor`, and listen on the port the
/// initiator dials.
fn build_acceptor_settings(cfg_path: &str) -> Result<SessionSettings, Box<dyn std::error::Error>> {
    let mut settings = SessionSettings::new();

    let mut global = Dictionary::new();
    global.set("ConnectionType", "acceptor")?;
    global.set("StartTime", "00:00:00")?;
    global.set("EndTime", "00:00:00")?;
    global.set("UseDataDictionary", "N")?;
    global.set("ResetOnLogon", "Y")?; // clean re-listen across Start/Stop cycles
    settings.set(None, global)?;

    for s in parse_sessions(cfg_path)? {
        let begin = s.get("BeginString").ok_or("session missing BeginString")?;
        let isender = s.get("SenderCompID").ok_or("session missing SenderCompID")?;
        let itarget = s.get("TargetCompID").ok_or("session missing TargetCompID")?;
        let port = s
            .get("SocketConnectPort")
            .ok_or("session missing SocketConnectPort")?;
        let hb = s.get("HeartBtInt").map(String::as_str).unwrap_or("30");

        // Acceptor session = initiator session with comp ids swapped.
        let sid = SessionId::try_new(begin, itarget, isender, "")?;
        let mut d = Dictionary::new();
        d.set("BeginString", begin.as_str())?;
        d.set("SenderCompID", itarget.as_str())?;
        d.set("TargetCompID", isender.as_str())?;
        d.set("SocketAcceptPort", port.as_str())?;
        d.set("HeartBtInt", hb)?;
        settings.set(Some(&sid), d)?;
    }
    Ok(settings)
}

/// Run the acceptor until `stop` is set. All FFI objects live on this thread.
fn run_acceptor(stop: &AtomicBool) -> Result<(), Box<dyn std::error::Error>> {
    let settings = build_acceptor_settings(CFG_PATH)?;
    let store = MemoryMessageStoreFactory::new();
    let logger = FileLogger::new(crate::logger::TESTCOUNTERPARTY_LOG_DIR)?;
    let log_factory = LogFactory::try_new(&logger)?;
    let callbacks = TestCounterparty;
    let app = Application::try_new(&callbacks)?;

    let mut acceptor = Acceptor::try_new(
        &settings,
        &app,
        &store,
        &log_factory,
        FixSocketServerKind::SingleThreaded,
    )?;

    acceptor.start()?;
    println!("Test testcounterparty: started");
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(200));
    }
    acceptor.stop()?;
    println!("Test testcounterparty: stopped");
    Ok(())
}

/// Shareable start/stop handle for the test testcounterparty. `Some(flag)` means a
/// thread is running; setting the flag asks it to stop.
#[derive(Clone, Default)]
pub struct TestCounterpartyControl {
    inner: Arc<Mutex<Option<Arc<AtomicBool>>>>,
}

impl TestCounterpartyControl {
    pub fn is_running(&self) -> bool {
        self.inner.lock().unwrap().is_some()
    }

    /// Start the acceptor if not already running. Returns the resulting state.
    pub fn start(&self) -> bool {
        let mut guard = self.inner.lock().unwrap();
        if guard.is_some() {
            return true;
        }
        let stop = Arc::new(AtomicBool::new(false));
        *guard = Some(stop.clone());
        drop(guard);

        let this = self.clone();
        std::thread::spawn(move || {
            if let Err(e) = run_acceptor(&stop) {
                eprintln!("Test counterparty error: {e}");
            }
            // Clear the running slot on exit (stop or error), unless a newer
            // start already replaced our flag.
            let mut guard = this.inner.lock().unwrap();
            if guard.as_ref().is_some_and(|s| Arc::ptr_eq(s, &stop)) {
                *guard = None;
            }
        });
        true
    }

    /// Ask a running acceptor to stop (no-op if not running).
    pub fn stop(&self) -> bool {
        if let Some(stop) = self.inner.lock().unwrap().take() {
            stop.store(true, Ordering::SeqCst);
        }
        false
    }
}
