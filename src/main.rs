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

use fix_app::{FixApp, SharedSession, SharedStatus};

/// Run the FIX initiator: start the engine, wait for a logon, send the CSV
/// orders once, then keep the session alive. Runs on its own (blocking) thread.
fn run_fix(status: SharedStatus, logged_on: SharedSession) -> Result<(), Box<dyn std::error::Error>> {
    let logger = logger::FileLogger::new("./logfile.log")?;
    let log_factory = LogFactory::try_new(&logger)?;
    let settings = SessionSettings::try_from_path("sessions.cfg")?;
    let store_factory = MemoryMessageStoreFactory::new();
    let callbacks = FixApp::new(status.clone(), logged_on.clone());
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

    let fix_status = status.clone();
    let fix_logged = logged_on.clone();
    std::thread::spawn(move || {
        if let Err(e) = run_fix(fix_status, fix_logged) {
            eprintln!("FIX engine error: {e}");
        }
    });

    web::serve(status).await;
}
