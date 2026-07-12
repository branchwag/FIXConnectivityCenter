mod testcounterparty;
mod dict_defaults;
mod fix_app;
mod logger;
mod metrics;
mod send;
mod web;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quickfix::{
    Application, ConnectionHandler, FixSocketServerKind, Initiator, LogFactory,
    MemoryMessageStoreFactory, SessionSettings,
};
use tokio::sync::broadcast;

use fix_app::{Directions, FixApp, SharedLastEvent, SharedStarted, SharedStatus};
use web::AppState;

/// Run the FIX initiator: start the engine, then keep the thread (and therefore
/// the initiator) alive for the life of the process so the engine keeps running
/// and the dashboard keeps reflecting live session state (logon / logout /
/// reconnect) instead of stopping after the first logout. Messages are sent
/// on demand from the dashboard's Send tool, not automatically on logon.
#[allow(clippy::too_many_arguments)]
fn run_fix(
    status: SharedStatus,
    last_event: SharedLastEvent,
    started: SharedStarted,
    directions: Directions,
    events: broadcast::Sender<String>,
    testcounterparty: testcounterparty::TestCounterpartyControl,
) -> Result<(), Box<dyn std::error::Error>> {
    let logger = logger::FileLogger::new(logger::LOG_DIR)?;
    let log_factory = LogFactory::try_new(&logger)?;
    // Fill in a default dictionary for any session that lacks one, so validation
    // being on can't fail startup with "DataDictionary not defined".
    let settings_path = dict_defaults::prepare("sessions.cfg");
    let settings = SessionSettings::try_from_path(&settings_path)?;
    let store_factory = MemoryMessageStoreFactory::new();
    let callbacks = FixApp::new(
        status.clone(),
        last_event,
        started,
        directions,
        events,
        testcounterparty,
    );
    let app = Application::try_new(&callbacks)?;

    let mut initiator = Initiator::try_new(
        &settings,
        &app,
        &store_factory,
        &log_factory,
        FixSocketServerKind::SingleThreaded,
    )?;

    initiator.start()?;

    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[tokio::main]
async fn main() {
    let status: SharedStatus = Arc::new(Mutex::new(HashMap::new()));
    let last_event: SharedLastEvent = Arc::new(Mutex::new(None));
    // Per-session enabled state (absent = enabled) driven by the Start/Disconnect
    // buttons, and per-session direction read once from the config.
    let started: SharedStarted = Arc::new(Mutex::new(HashMap::new()));
    let directions: Directions = Arc::new(testcounterparty::session_directions("sessions.cfg"));
    // Status-change events pushed by the FIX callbacks and streamed to the
    // dashboard over SSE.
    let (events, _) = broadcast::channel::<String>(64);
    // Shared between the FIX callbacks (so its running state rides the SSE
    // stream) and the web layer (which starts/stops it).
    let testcounterparty = testcounterparty::TestCounterpartyControl::default();

    let fix_status = status.clone();
    let fix_last_event = last_event.clone();
    let fix_started = started.clone();
    let fix_directions = directions.clone();
    let fix_events = events.clone();
    let fix_testcounterparty = testcounterparty.clone();
    std::thread::spawn(move || {
        if let Err(e) = run_fix(
            fix_status,
            fix_last_event,
            fix_started,
            fix_directions,
            fix_events,
            fix_testcounterparty,
        ) {
            eprintln!("FIX engine error: {e}");
        }
    });

    web::serve(AppState {
        status,
        last_event,
        started,
        directions,
        events,
        testcounterparty,
        metrics: metrics::MetricsState::new(),
    })
    .await;
}
