//! Canonical reconciliation ledger rendering.

use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
struct Ledger<'a> {
    schema: &'static str,
    attempt_path: &'a str,
    entries: Vec<LedgerEntry<'a>>,
}

#[derive(Serialize)]
struct LedgerEntry<'a> {
    deviation_id: &'a str,
    outcome: &'a str,
    target: &'a str,
    attempt_sha256: &'a str,
    reconciliation_fingerprint: &'a str,
}

pub fn render_ledger(ledger: &Value) -> Result<Vec<u8>, &'static str> {
    let mut entries = Vec::new();
    for entry in ledger["entries"]
        .as_array()
        .ok_or("The reconciliation ledger is invalid.")?
    {
        entries.push(LedgerEntry {
            deviation_id: entry["deviation_id"]
                .as_str()
                .ok_or("A ledger deviation ID is missing.")?,
            outcome: entry["outcome"]
                .as_str()
                .ok_or("A ledger outcome is missing.")?,
            target: entry["target"]
                .as_str()
                .ok_or("A ledger target is missing.")?,
            attempt_sha256: entry["attempt_sha256"]
                .as_str()
                .ok_or("A ledger Attempt digest is missing.")?,
            reconciliation_fingerprint: entry["reconciliation_fingerprint"]
                .as_str()
                .ok_or("A ledger fingerprint is missing.")?,
        });
    }
    let ordered = Ledger {
        schema: "work-spec-reconciliation-ledger/v1",
        attempt_path: ledger["attempt_path"]
            .as_str()
            .ok_or("The ledger Attempt path is missing.")?,
        entries,
    };
    let mut raw =
        serde_json::to_vec_pretty(&ordered).map_err(|_| "The ledger cannot be rendered.")?;
    raw.push(b'\n');
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ledger_rendering_preserves_field_order_and_newline() {
        let raw = render_ledger(&json!({"attempt_path":"a","entries":[]})).unwrap();
        assert!(raw.starts_with(b"{\n  \"schema\": \"work-spec-reconciliation-ledger/v1\","));
        assert!(raw.ends_with(b"\n"));
    }
}
