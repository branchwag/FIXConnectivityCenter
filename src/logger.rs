//! Custom FIX logger. Writes into a log directory, one file per session
//! (`<begin>-<sender>-<target>.log`) plus a shared `engine.log` for global /
//! non-session events. Line format mirrors the Go `screenLog` "FancyLog".
//!
//! Files are size-rotated: when a write would push a log past `max_bytes`, it is
//! rolled to `<name>.log.1` (older archives shift up to `<name>.log.<max_files>`
//! and the oldest is dropped). This bounds each stream to roughly
//! `max_bytes * (max_files + 1)` on disk, so logs never grow without limit.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use quickfix::{LogCallback, SessionId};

/// Roll a log file once a write would push it past this many bytes.
const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024; // 5 MiB
/// Number of rotated archives to keep per log (`<name>.log.1` .. `.<N>`).
const DEFAULT_MAX_FILES: u32 = 5;

/// An open log file plus its current size, so rotation doesn't need to stat.
struct LogFile {
    file: File,
    size: u64,
}

pub struct FileLogger {
    dir: PathBuf,
    max_bytes: u64,
    max_files: u32,
    // Open log per name (session id or "engine"), created lazily.
    files: Mutex<HashMap<String, LogFile>>,
}

impl FileLogger {
    /// `dir` is the directory logs are written into (e.g. "./logs").
    pub fn new(dir: &str) -> std::io::Result<Self> {
        Self::with_limits(dir, DEFAULT_MAX_BYTES, DEFAULT_MAX_FILES)
    }

    /// Like [`new`](Self::new) but with explicit rotation limits (used by tests).
    pub fn with_limits(dir: &str, max_bytes: u64, max_files: u32) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            dir: PathBuf::from(dir),
            max_bytes,
            max_files,
            files: Mutex::new(HashMap::new()),
        })
    }

    fn write(&self, session: Option<&SessionId>, line: String) {
        let name = match session {
            Some(s) => session_log_name(s),
            None => "engine".to_string(),
        };
        let bytes = line.len() as u64;

        let mut files = self.files.lock().unwrap();

        // Roll over before writing if this line would push us past the cap.
        // Skip when the file is still empty, so a single oversized line can't
        // trigger endless rotation.
        if let Some(existing) = files.get(&name) {
            if existing.size > 0 && existing.size + bytes > self.max_bytes {
                files.remove(&name); // drop the handle so the file can be renamed
                self.rotate(&name);
            }
        }

        if !files.contains_key(&name) {
            let path = self.dir.join(format!("{name}.log"));
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(file) => {
                    // Continue an existing file across restarts.
                    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
                    files.insert(name.clone(), LogFile { file, size });
                }
                Err(e) => {
                    eprintln!("Failed to open log file {}: {e}", path.display());
                    return;
                }
            }
        }

        if let Some(entry) = files.get_mut(&name) {
            if entry.file.write_all(line.as_bytes()).is_ok() {
                entry.size += bytes;
            }
        }
    }

    /// Shift archives up (`<name>.log` -> `.1`, `.1` -> `.2`, ...) dropping
    /// anything past `max_files`. The live handle for `name` must be closed
    /// first (removed from `files`).
    fn rotate(&self, name: &str) {
        // Move older archives up, highest index first, so each rename overwrites
        // the next; whatever was at `.max_files` is discarded.
        for i in (1..self.max_files).rev() {
            let src = self.dir.join(format!("{name}.log.{i}"));
            if src.exists() {
                let dst = self.dir.join(format!("{name}.log.{}", i + 1));
                let _ = std::fs::rename(&src, &dst);
            }
        }
        let base = self.dir.join(format!("{name}.log"));
        if self.max_files == 0 {
            // No archives kept: just drop the current file.
            let _ = std::fs::remove_file(&base);
        } else {
            let _ = std::fs::rename(&base, self.dir.join(format!("{name}.log.1")));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_and_bounds_archives() {
        let dir = std::env::temp_dir().join(format!("fixlog_rotate_{}", std::process::id()));
        let dir_s = dir.to_str().unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        // 100-byte cap, keep 2 archives. 61-byte lines force a rotation on
        // every second write.
        let logger = FileLogger::with_limits(dir_s, 100, 2).unwrap();
        let line = "x".repeat(60) + "\n"; // 61 bytes
        for _ in 0..10 {
            logger.write(None, line.clone());
        }

        let base = dir.join("engine.log");
        assert!(base.exists(), "live log should exist");
        assert!(dir.join("engine.log.1").exists(), ".1 archive should exist");
        assert!(dir.join("engine.log.2").exists(), ".2 archive should exist");
        // Retention: never keep more than max_files archives.
        assert!(
            !dir.join("engine.log.3").exists(),
            "must not keep archives beyond max_files"
        );
        // The live file always stays under the cap.
        let live = std::fs::metadata(&base).unwrap().len();
        assert!(live <= 100, "live log {live} exceeds cap");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
