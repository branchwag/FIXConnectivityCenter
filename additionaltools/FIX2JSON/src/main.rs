use std::io::Write;

use serde::Serialize;
use serde_json::{Map, Value};

/// FIX tag -> human-readable field name.
// mebbe replace with xml dictionary later
fn field_names() -> Map<String, Value> {
    [
        ("8", "BeginString"),
        ("9", "BodyLength"),
        ("35", "MessageType"),
        ("34", "MsgSeqNum"),
        ("49", "SenderCompID"),
        ("52", "SendingTime"),
        ("56", "TargetCompID"),
        ("6", "Price"),
        ("11", "ClOrdID"),
        ("14", "OrderQty"),
        ("15", "Currency"),
        ("17", "ExecTransType"),
        ("21", "HandlInst"),
        ("31", "LastPx"),
        ("32", "LastShares"),
        ("37", "OrderID"),
        ("38", "OrderQty2"),
        ("39", "OrdStatus"),
        ("40", "OrdType"),
        ("54", "Side"),
        ("55", "Symbol"),
        ("60", "TransactTime"),
        ("150", "ExecType"),
        ("151", "LeavesQty"),
        ("453", "NoLinesOfText"),
        ("448", "OtherPartyID"),
        ("447", "OtherPartyIDSource"),
        ("452", "NoPartyIDs"),
        ("10", "CheckSum"),
    ]
    .into_iter()
    .map(|(tag, name)| (tag.to_string(), Value::String(name.to_string())))
    .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let field_names = field_names();

    // Prompt for the file containing the FIX message.
    print!("Enter path to FIX message file: ");
    std::io::stdout().flush()?;
    let mut path = String::new();
    std::io::stdin().read_line(&mut path)?;
    let path = path.trim();

    let fix_msg = std::fs::read_to_string(path)?;

    let mut json = Map::new();
    for field in fix_msg.trim().split('\u{1}') {
        if let Some((tag, value)) = field.split_once('=') {
            let key = match field_names.get(tag) {
                Some(Value::String(name)) => name.clone(),
                _ => tag.to_string(),
            };
            json.insert(key, Value::String(value.to_string()));
        }
    }

    // 4 space indent
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    Value::Object(json).serialize(&mut ser)?;
    println!("{}", String::from_utf8(buf)?);

    Ok(())
}
