//! Read `messages.csv` and send each row as a FIX message over the session,
//! mirroring the Go `ReadCSV` + `SendFIXMessageFromCSV`.

use quickfix::{send_to_target, FieldMap, Message, SessionId};

pub fn send_from_csv(
    path: &str,
    session_id: &SessionId,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = csv::Reader::from_path(path)?;
    let headers = reader.headers()?.clone();

    for result in reader.records() {
        let record = result?;
        let mut msg = Message::new();

        for (i, value) in record.iter().enumerate() {
            match headers.get(i).unwrap_or("") {
                "MsgType" => {
                    msg.with_header_mut(|h| h.set_field(35, value))?;
                }
                // SenderCompID / TargetCompID are filled from the session on send.
                "SenderCompID" | "TargetCompID" => {}
                other => match other.parse::<i32>() {
                    Ok(tag) => msg.set_field(tag, value)?,
                    Err(_) => eprintln!("Invalid FIX tag: {other}"),
                },
            }
        }

        println!(
            "Attempting to send message to SessionID: {}",
            session_id.to_repr()
        );
        send_to_target(msg, session_id)?;
        println!("Message sent.");
    }

    Ok(())
}
