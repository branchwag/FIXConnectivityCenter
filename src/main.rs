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

    let mut sent = false;
    loop {
        let key = logged_on.lock().unwrap().clone();
        if let Some(key) = key {
            if !sent {
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

            // Keep the session alive; bail out once it drops.
            let active = status.lock().unwrap().values().any(|&c| c);
            if !active {
                println!("Session is no longer active. Exiting.");
                break;
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }

    initiator.stop()?;
    Ok(())
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
