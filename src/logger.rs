//! Custom FIX logger writing incoming/outgoing/event lines to a file, mirroring
//! the Go `screenLog` "FancyLog".

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use quickfix::{LogCallback, SessionId};

pub struct FileLogger {
    file: Mutex<std::fs::File>,
}

impl FileLogger {
    pub fn new(path: &str) -> std::io::Result<Self> {
        // Create the log directory (e.g. ./logs) if it doesn't exist yet.
        if let Some(dir) = Path::new(path).parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    fn write(&self, s: String) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.write_all(s.as_bytes());
        }
    }
}

impl LogCallback for FileLogger {
    fn on_incoming(&self, _session: Option<&SessionId>, msg: &str) {
        self.write(format!("<=== Incoming FIX Msg: <===\n{msg}\n"));
    }

    fn on_outgoing(&self, _session: Option<&SessionId>, msg: &str) {
        self.write(format!("===> Outgoing FIX Msg: ===>\n{msg}\n"));
    }

    fn on_event(&self, _session: Option<&SessionId>, msg: &str) {
        self.write(format!("==== Event: ====\n{msg}\n"));
    }
}
