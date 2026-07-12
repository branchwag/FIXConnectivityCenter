//! Manual message send tool: parse a single pasted message (raw FIX or JSON) or
//! a CSV batch (one message per row) and send each over a chosen session.
//!
//! Header/engine-managed tags are never taken from user input — quickfix fills
//! `BeginString`/`BodyLength`/`CheckSum`/`MsgSeqNum`/`SenderCompID`/`SendingTime`/
//! `TargetCompID` from the session and message store on send. The user supplies
//! `MsgType` (35) plus the application body fields.

use quickfix::{send_to_target, FieldMap, Message, SessionId};
use serde::Serialize;

/// Tags quickfix owns; ignored if present in user input.
const MANAGED_TAGS: &[i32] = &[8, 9, 10, 34, 49, 52, 56];

fn is_managed(tag: i32) -> bool {
    MANAGED_TAGS.contains(&tag)
}

/// Build a quickfix `Message` from (tag, value) pairs. Tag 35 (MsgType) goes in
/// the header; managed tags are dropped; everything else is a body field.
/// Errors if MsgType is missing.
fn build_message(fields: &[(i32, String)]) -> Result<Message, String> {
    let mut msg = Message::new();
    let mut have_msgtype = false;
    for (tag, value) in fields {
        if is_managed(*tag) {
            continue;
        }
        if *tag == 35 {
            if value.trim().is_empty() {
                continue; // empty MsgType is treated as missing (caught below)
            }
            msg.with_header_mut(|h| h.set_field(35, value.as_str()))
                .map_err(|e| format!("set MsgType: {e}"))?;
            have_msgtype = true;
        } else {
            msg.set_field(*tag, value.as_str())
                .map_err(|e| format!("set tag {tag}: {e}"))?;
        }
    }
    if !have_msgtype {
        return Err("missing MsgType (tag 35)".to_string());
    }
    Ok(msg)
}

/// Map a field name (from JSON keys or CSV headers) to a FIX tag. `MsgType` is
/// an alias for 35; `SenderCompID`/`TargetCompID` are session-filled, so they
/// resolve to `None` (skip). Anything else must be a numeric tag.
fn name_to_tag(name: &str) -> Result<Option<i32>, String> {
    if name.eq_ignore_ascii_case("MsgType") {
        Ok(Some(35))
    } else if name.eq_ignore_ascii_case("SenderCompID") || name.eq_ignore_ascii_case("TargetCompID") {
        Ok(None)
    } else {
        name.parse::<i32>()
            .map(Some)
            .map_err(|_| format!("invalid field/column {name:?} (want a numeric FIX tag or MsgType)"))
    }
}

/// Parse a single pasted message, auto-detecting JSON (`{...}`) vs raw FIX.
fn parse_single(input: &str) -> Result<Vec<(i32, String)>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("empty input".to_string());
    }
    if trimmed.starts_with('{') {
        parse_json(trimmed)
    } else {
        parse_fix(trimmed)
    }
}

/// Raw FIX: `tag=value` pairs separated by SOH (`\x01`) or the display
/// delimiters `|` / `^`.
fn parse_fix(input: &str) -> Result<Vec<(i32, String)>, String> {
    let mut out = Vec::new();
    for part in input.split(['\x01', '|', '^']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (tag, value) = part
            .split_once('=')
            .ok_or_else(|| format!("bad field (no '='): {part:?}"))?;
        let tag: i32 = tag
            .trim()
            .parse()
            .map_err(|_| format!("non-numeric tag: {:?}", tag.trim()))?;
        out.push((tag, value.to_string()));
    }
    if out.is_empty() {
        return Err("no fields parsed".to_string());
    }
    Ok(out)
}

/// JSON object of field -> value, e.g. `{"35":"D","55":"EUR/USD","38":1000}`.
/// Keys may be numeric tags or `MsgType`; values may be strings or numbers.
fn parse_json(input: &str) -> Result<Vec<(i32, String)>, String> {
    let val: serde_json::Value =
        serde_json::from_str(input).map_err(|e| format!("invalid JSON: {e}"))?;
    let obj = val
        .as_object()
        .ok_or("JSON must be an object of field -> value")?;
    let mut out = Vec::new();
    for (k, v) in obj {
        let Some(tag) = name_to_tag(k)? else {
            continue; // Sender/TargetCompID: filled from the session
        };
        let value = match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            _ => return Err(format!("field {k} must be a string or number")),
        };
        out.push((tag, value));
    }
    Ok(out)
}

/// Outcome of a single-message send.
#[derive(Serialize)]
pub struct SendResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SendResult {
    fn ok() -> Self {
        Self { ok: true, error: None }
    }
    fn err(e: impl Into<String>) -> Self {
        Self { ok: false, error: Some(e.into()) }
    }
}

/// Parse and send a single pasted message (FIX or JSON) over `sid`.
pub fn send_single(input: &str, sid: &SessionId) -> SendResult {
    let msg = match parse_single(input).and_then(|f| build_message(&f)) {
        Ok(m) => m,
        Err(e) => return SendResult::err(e),
    };
    match send_to_target(msg, sid) {
        Ok(_) => SendResult::ok(),
        Err(e) => SendResult::err(format!("send failed: {e}")),
    }
}

/// A row that failed to parse or send, for the batch report.
#[derive(Serialize)]
pub struct RowError {
    /// 1-based line number in the uploaded file (header is line 1).
    pub row: usize,
    pub error: String,
}

/// Outcome of a CSV batch send.
#[derive(Serialize)]
pub struct CsvResult {
    pub ok: bool,
    pub sent: usize,
    pub total: usize,
    pub errors: Vec<RowError>,
}

/// Parse a CSV (header row of FIX tags + `MsgType`, one message per data row)
/// and send each row over `sid`. A bad row is reported, not fatal — the rest of
/// the batch still sends.
pub fn send_csv(csv_text: &str, sid: &SessionId) -> CsvResult {
    let mut reader = csv::Reader::from_reader(csv_text.as_bytes());
    let headers = match reader.headers() {
        Ok(h) => h.clone(),
        Err(e) => {
            return CsvResult {
                ok: false,
                sent: 0,
                total: 0,
                errors: vec![RowError { row: 1, error: format!("bad header row: {e}") }],
            }
        }
    };

    let mut sent = 0;
    let mut total = 0;
    let mut errors = Vec::new();

    for (i, rec) in reader.records().enumerate() {
        let row = i + 2; // 1-based; +1 for the header line
        total += 1;
        let record = match rec {
            Ok(r) => r,
            Err(e) => {
                errors.push(RowError { row, error: format!("parse error: {e}") });
                continue;
            }
        };

        let mut fields = Vec::new();
        let mut bad = None;
        for (col, value) in record.iter().enumerate() {
            let name = headers.get(col).unwrap_or("");
            match name_to_tag(name) {
                Ok(Some(tag)) => fields.push((tag, value.to_string())),
                Ok(None) => {} // session-filled column
                Err(e) => {
                    bad = Some(e);
                    break;
                }
            }
        }
        if let Some(e) = bad {
            errors.push(RowError { row, error: e });
            continue;
        }

        let result = build_message(&fields)
            .and_then(|m| send_to_target(m, sid).map_err(|e| format!("send failed: {e}")));
        match result {
            Ok(_) => sent += 1,
            Err(e) => errors.push(RowError { row, error: e }),
        }
    }

    CsvResult { ok: errors.is_empty() && total > 0, sent, total, errors }
}
