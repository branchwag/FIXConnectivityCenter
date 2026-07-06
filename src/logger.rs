//! Custom FIX logger. Writes into a log directory, one file per session
//! (`<begin>-<sender>-<target>.log`) plus a shared `engine.log` for global /
//! non-session events. Line format mirrors the Go `screenLog` "FancyLog".

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use quickfix::{LogCallback, SessionId};

pub struct FileLogger {
    dir: PathBuf,
    // Open file handle per log name (session id or "engine"), created lazily.
    files: Mutex<HashMap<String, File>>,
}

impl FileLogger {
    /// `dir` is the directory logs are written into (e.g. "./logs").
    pub fn new(dir: &str) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            dir: PathBuf::from(dir),
            files: Mutex::new(HashMap::new()),
        })
    }

    fn write(&self, session: Option<&SessionId>, line: String) {
        let name = match session {
            Some(s) => session_log_name(s),
            None => "engine".to_string(),
        };

        let mut files = self.files.lock().unwrap();
        if !files.contains_key(&name) {
            let path = self.dir.join(format!("{name}.log"));
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(f) => {
                    files.insert(name.clone(), f);
                }
                Err(e) => {
                    eprintln!("Failed to open log file {}: {e}", path.display());
                    return;
                }
            }
        }
        if let Some(file) = files.get_mut(&name) {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

/// Filename stem for a session's log, e.g. "FIX.4.2-FIXDEV-TEST".
fn session_log_name(s: &SessionId) -> String {
    let begin = s.get_begin_string().unwrap_or_default();
    let sender = s.get_sender_comp_id().unwrap_or_default();
    let target = s.get_target_comp_id().unwrap_or_default();
    sanitize(&format!("{begin}-{sender}-{target}"))
}

/// Keep filenames filesystem-safe (session ids can contain `:`, `/`, `->`).
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl LogCallback for FileLogger {
    fn on_incoming(&self, session: Option<&SessionId>, msg: &str) {
        self.write(session, format!("<=== Incoming FIX Msg: <===\n{msg}\n"));
    }

    fn on_outgoing(&self, session: Option<&SessionId>, msg: &str) {
        self.write(session, format!("===> Outgoing FIX Msg: ===>\n{msg}\n"));
    }

    fn on_event(&self, session: Option<&SessionId>, msg: &str) {
        self.write(session, format!("==== Event: ====\n{msg}\n"));
    }
}
