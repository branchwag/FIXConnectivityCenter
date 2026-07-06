//! Protobuf types (generated from `model/message.proto` at build time) plus the
//! FIX -> proto conversion, mirroring the Go `ConvertToProto`.

include!(concat!(env!("OUT_DIR"), "/_.rs"));

use quickfix::{FieldMap, Message};

/// Build a `FixMessage` proto out of an inbound quickfix `Message`.
pub fn from_fix_message(msg: &Message) -> FixMessage {
    let mut out = FixMessage::default();

    // Header fields.
    if let Some(v) = msg.with_header(|h| h.get_field(49)) {
        out.sender_comp_id = v;
    }
    if let Some(v) = msg.with_header(|h| h.get_field(56)) {
        out.target_comp_id = v;
    }
    if let Some(v) = msg.with_header(|h| h.get_field(34)) {
        if let Ok(n) = v.parse::<i32>() {
            out.msg_seq_num = n;
        }
    }
    if let Some(v) = msg.with_header(|h| h.get_field(35)) {
        // NOTE: parity with the original Go app. The lookup is keyed by the proto
        // enum *name* ("NEW_ORDER", ...), not the FIX code ("D"), so real FIX
        // message types fall through to UNKNOWN (0). Kept as-is on purpose.
        out.msg_type = MsgType::from_str_name(&v).map(|e| e as i32).unwrap_or_default();
    }
    if let Some(v) = msg.with_header(|h| h.get_field(52)) {
        out.sending_time = v;
    }

    // Body fields (New Order - Single).
    if let Some(v) = msg.get_field(11) {
        out.cl_ord_id = v;
    }
    if let Some(v) = msg.get_field(55) {
        out.symbol = v;
    }
    if let Some(v) = msg.get_field(54) {
        // Same name-keyed quirk as MsgType above.
        out.side = Side::from_str_name(&v).map(|e| e as i32).unwrap_or_default();
    }
    if let Some(v) = msg.get_field(38) {
        match v.parse::<f64>() {
            Ok(q) => out.order_qty = q,
            Err(e) => eprintln!("Failed to parse OrderQty: {e}"),
        }
    }
    if let Some(v) = msg.get_field(44) {
        match v.parse::<f64>() {
            Ok(p) => out.price = p,
            Err(e) => eprintln!("Failed to parse Price: {e}"),
        }
    }
    if let Some(v) = msg.get_field(60) {
        out.transact_time = v;
    }

    // Repeating group NoContraBrokers (382) -> ContraBroker (375) / ContraTrader (337).
    if let Some(count) = msg.get_field(382).and_then(|s| s.parse::<i32>().ok()) {
        for i in 1..=count {
            let group = ContraBrokerGroup {
                contra_broker: msg
                    .with_group(i, 382, |g| g.get_field(375))
                    .flatten()
                    .unwrap_or_default(),
                contra_trader: msg
                    .with_group(i, 382, |g| g.get_field(337))
                    .flatten()
                    .unwrap_or_default(),
                ..Default::default()
            };
            out.contra_brokers.push(group);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickfix::{FieldMap, Message};

    #[test]
    fn converts_new_order_single() {
        let mut msg = Message::new();
        msg.with_header_mut(|h| {
            h.set_field(49, "FIXDEV").unwrap();
            h.set_field(56, "TEST").unwrap();
            h.set_field(34, "42").unwrap();
            h.set_field(35, "D").unwrap();
            h.set_field(52, "20240808-19:25:05").unwrap();
        });
        msg.set_field(11, "ORDER123").unwrap();
        msg.set_field(55, "EUR/USD").unwrap();
        msg.set_field(54, "1").unwrap();
        msg.set_field(38, "1000").unwrap();
        msg.set_field(44, "1.12345").unwrap();
        msg.set_field(60, "20240808-19:25:05").unwrap();

        let out = from_fix_message(&msg);
        assert_eq!(out.sender_comp_id, "FIXDEV");
        assert_eq!(out.target_comp_id, "TEST");
        assert_eq!(out.msg_seq_num, 42);
        assert_eq!(out.cl_ord_id, "ORDER123");
        assert_eq!(out.symbol, "EUR/USD");
        assert_eq!(out.order_qty, 1000.0);
        assert_eq!(out.price, 1.12345);
        assert_eq!(out.transact_time, "20240808-19:25:05");
        // Parity quirk: FIX codes "D"/"1" are not the proto enum *names*
        // ("NEW_ORDER"/"SELL"), so both fall through to the default 0.
        assert_eq!(out.msg_type, 0);
        assert_eq!(out.side, 0);

        // The whole point of the conversion is to hand prost something encodable.
        let mut buf = Vec::new();
        prost::Message::encode(&out, &mut buf).unwrap();
        assert!(!buf.is_empty());
    }
}
