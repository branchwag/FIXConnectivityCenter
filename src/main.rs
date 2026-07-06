mod csv_send;
mod fix_app;
mod logger;
mod proto;
mod web;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quickfix::{
    Application, ConnectionHandler, FixSocketServerKind, Initiator, LogFactory,
    MemoryMessageStoreFactory, SessionId, SessionSettings,
};
use tokio::sync::broadcast;

use fix_app::{FixApp, SharedLastEvent, SharedSession, SharedStatus};
use web::AppState;

/// Run the FIX initiator: start the engine, wait for a logon, send the CSV
/// orders once, then keep the engine alive for the life of the process.
fn run_fix(
    status: SharedStatus,
    logged_on: SharedSession,
    last_event: SharedLastEvent,
    events: broadcast::Sender<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let logger = logger::FileLogger::new("./logs/engine.log")?;
    let log_factory = LogFactory::try_new(&logger)?;
    let settings = SessionSettings::try_from_path("sessions.cfg")?;
    let store_factory = MemoryMessageStoreFactory::new();
    let callbacks = FixApp::new(status.clone(), logged_on.clone(), last_event, events);
    let app = Application::try_new(&callbacks)?;

    let mut initiator = Initiator::try_new(
        &settings,
        &app,
        &store_factory,
        &log_factory,
        FixSocketServerKind::SingleThreaded,
    )?;

    initiator.start()?;

    // Send the CSV orders once, on the first logon. Then keep this thread (and
    // therefore the initiator) alive for the life of the process so the engine
    // keeps running and the dashboard keeps reflecting live session state
    // (logon / logout / reconnect) instead of stopping after the first logout.
    let mut sent = false;
    loop {
        if !sent {
            let key = logged_on.lock().unwrap().clone();
            if let Some(key) = key {
                let sid = SessionId::try_new(
                    &key.begin_string,
                    &key.sender_comp_id,
                    &key.target_comp_id,
                    "",
                )?;
                if let Err(e) = csv_send::send_from_csv("messages.csv", &sid) {
                    eprintln!("Error sending FIX message: {e}");
                }
                sent = true;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[tokio::main]
async fn main() {
    let status: SharedStatus = Arc::new(Mutex::new(HashMap::new()));
    let logged_on: SharedSession = Arc::new(Mutex::new(None));
    let last_event: SharedLastEvent = Arc::new(Mutex::new(None));
    // Status-change events pushed by the FIX callbacks and streamed to the
    // dashboard over SSE.
    let (events, _) = broadcast::channel::<String>(64);

    let fix_status = status.clone();
    let fix_logged = logged_on.clone();
    let fix_last_event = last_event.clone();
    let fix_events = events.clone();
    std::thread::spawn(move || {
        if let Err(e) = run_fix(fix_status, fix_logged, fix_last_event, fix_events) {
            eprintln!("FIX engine error: {e}");
        }
    });

    web::serve(AppState {
        status,
        last_event,
        events,
    })
    .await;
}
