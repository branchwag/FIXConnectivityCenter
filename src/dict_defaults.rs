//! Fill in a default FIX data dictionary for any session that doesn't configure
//! one, so the engine never fails startup with "DataDictionary not defined" when
//! validation (`UseDataDictionary=Y`) is on.
//!
//! QuickFIX's Rust binding can't mutate an already-loaded session (`set` throws
//! on a duplicate session id, and dictionary access is read-only), so we
//! preprocess the config text instead: a session with no `DataDictionary` gets
//! the bundled `spec/FIX<ver>.xml` for its `BeginString` injected. If no bundled
//! spec matches the version and validation would otherwise be on, we disable
//! validation for that session rather than let it crash the engine.

use std::path::PathBuf;

/// Map a `BeginString` ("FIX.4.2") to its bundled spec path ("spec/FIX42.xml").
/// Returns `None` for versions we don't recognise (e.g. FIXT transport).
fn spec_path_for(begin: &str) -> Option<String> {
    let rest = begin.strip_prefix("FIX.")?;
    let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    Some(format!("spec/FIX{digits}.xml"))
}

/// Return the path the engine should load session settings from: the original
/// config if every session already has a dictionary decision it can honour, or a
/// temp file with defaults injected otherwise.
pub fn prepare(cfg_path: &str) -> PathBuf {
    let Ok(text) = std::fs::read_to_string(cfg_path) else {
        return PathBuf::from(cfg_path);
    };
    let (augmented, changed) = augment(&text);
    if !changed {
        return PathBuf::from(cfg_path);
    }
    let tmp = std::env::temp_dir().join(format!("fixstatus-sessions-{}.cfg", std::process::id()));
    match std::fs::write(&tmp, augmented) {
        Ok(()) => tmp,
        Err(e) => {
            eprintln!("Could not write effective session config ({e}); using original");
            PathBuf::from(cfg_path)
        }
    }
}

/// Rewrite the config, appending a `DataDictionary` (or a validation-off) line to
/// any `[SESSION]` that lacks a dictionary. Returns `(text, changed)`.
fn augment(text: &str) -> (String, bool) {
    // Pass 1: DEFAULT-level values a session inherits.
    let (mut default_dd, mut default_udd_y) = (false, false);
    let mut section = String::new();
    for raw in text.lines() {
        let l = raw.trim();
        if l.starts_with('[') && l.ends_with(']') {
            section = l[1..l.len() - 1].to_ascii_uppercase();
        } else if section == "DEFAULT" && !l.starts_with('#') {
            if let Some((k, v)) = l.split_once('=') {
                match k.trim() {
                    k if k.eq_ignore_ascii_case("DataDictionary") => default_dd = true,
                    k if k.eq_ignore_ascii_case("UseDataDictionary") => {
                        default_udd_y = v.trim().eq_ignore_ascii_case("Y")
                    }
                    _ => {}
                }
            }
        }
    }

    // Pass 2: emit each line, flushing an injection at the end of any [SESSION]
    // block that has no dictionary.
    let mut out = String::new();
    let mut changed = false;
    let mut in_session = false;
    let mut begin: Option<String> = None;
    let mut has_dd = default_dd;
    let mut udd_y = default_udd_y;

    for raw in text.lines() {
        let l = raw.trim();
        if l.starts_with('[') && l.ends_with(']') {
            if in_session {
                flush(&mut out, &mut changed, &begin, has_dd, udd_y);
            }
            in_session = l[1..l.len() - 1].eq_ignore_ascii_case("SESSION");
            begin = None;
            has_dd = default_dd;
            udd_y = default_udd_y;
            out.push_str(raw);
            out.push('\n');
            continue;
        }
        if in_session && !l.starts_with('#') {
            if let Some((k, v)) = l.split_once('=') {
                match k.trim() {
                    k if k.eq_ignore_ascii_case("BeginString") => {
                        begin = Some(v.trim().to_string())
                    }
                    k if k.eq_ignore_ascii_case("DataDictionary") => has_dd = true,
                    k if k.eq_ignore_ascii_case("UseDataDictionary") => {
                        udd_y = v.trim().eq_ignore_ascii_case("Y")
                    }
                    _ => {}
                }
            }
        }
        out.push_str(raw);
        out.push('\n');
    }
    if in_session {
        flush(&mut out, &mut changed, &begin, has_dd, udd_y);
    }

    (out, changed)
}

/// Append the injected line(s) for a just-finished `[SESSION]` block, if it needs
/// a dictionary default.
fn flush(out: &mut String, changed: &mut bool, begin: &Option<String>, has_dd: bool, udd_y: bool) {
    if has_dd {
        return;
    }
    let Some(begin) = begin else { return };
    match spec_path_for(begin).and_then(|p| std::fs::canonicalize(p).ok()) {
        Some(abs) => {
            eprintln!("Session {begin}: no DataDictionary configured, using bundled {}", abs.display());
            out.push_str(&format!("# auto-added: standard dictionary for {begin} (none was set)\n"));
            out.push_str(&format!("DataDictionary={}\n", abs.display()));
            *changed = true;
        }
        None if udd_y => {
            eprintln!("Session {begin}: no DataDictionary and no bundled spec for this version; disabling validation to avoid startup failure");
            out.push_str(&format!("# auto-added: no bundled dictionary for {begin}; validation off to avoid startup failure\n"));
            out.push_str("UseDataDictionary=N\n");
            *changed = true;
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_begin_string_to_spec() {
        assert_eq!(spec_path_for("FIX.4.2").as_deref(), Some("spec/FIX42.xml"));
        assert_eq!(spec_path_for("FIX.4.4").as_deref(), Some("spec/FIX44.xml"));
        assert_eq!(spec_path_for("FIX.5.0").as_deref(), Some("spec/FIX50.xml"));
        assert_eq!(spec_path_for("FIXT.1.1"), None);
    }

    #[test]
    fn injects_default_dictionary_when_missing() {
        // Relies on spec/FIX42.xml existing at the crate root (it does).
        let cfg = "[DEFAULT]\nUseDataDictionary=Y\n\n\
                   [SESSION]\nBeginString=FIX.4.2\nSenderCompID=A\nTargetCompID=B\n";
        let (out, changed) = augment(cfg);
        assert!(changed);
        assert!(out.contains("DataDictionary="));
        assert!(out.contains("FIX42.xml"));
    }

    #[test]
    fn leaves_config_untouched_when_dictionary_present() {
        let cfg = "[DEFAULT]\nUseDataDictionary=Y\n\n\
                   [SESSION]\nBeginString=FIX.4.2\nDataDictionary=spec/FIX42.xml\n\
                   SenderCompID=A\nTargetCompID=B\n";
        let (_out, changed) = augment(cfg);
        assert!(!changed);
    }
}
