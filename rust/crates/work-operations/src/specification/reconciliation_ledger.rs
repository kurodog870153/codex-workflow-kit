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

pub fn validate_ledger(ledger: &Value) -> Result<(), &'static str> {
    let typed: work_model::specification::ReconciliationLedger =
        serde_json::from_value(ledger.clone())
            .map_err(|_| "The reconciliation ledger is invalid.")?;
    if typed.schema != "work-spec-reconciliation-ledger" || typed.attempt_path.is_empty() {
        return Err("The reconciliation ledger identity is invalid.");
    }
    let mut previous: Option<&str> = None;
    for entry in &typed.entries {
        let id = entry.deviation_id.as_str();
        let suffix = id.strip_prefix("DEVIATION-").unwrap_or("");
        if suffix.len() != 3
            || !suffix.bytes().all(|b| b.is_ascii_digit())
            || previous.is_some_and(|p| p >= id)
            || !matches!(
                entry.outcome.as_str(),
                "incorporated" | "retained" | "declined"
            )
            || !crate::protocol::valid_sha256(&entry.attempt_sha256)
            || !crate::protocol::valid_sha256(&entry.reconciliation_fingerprint)
        {
            return Err("A reconciliation ledger entry is invalid.");
        }
        previous = Some(id);
    }
    Ok(())
}

pub fn render_ledger(ledger: &Value) -> Result<Vec<u8>, &'static str> {
    validate_ledger(ledger)?;
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
        schema: "work-spec-reconciliation-ledger",
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
    fn ledger_rejects_plan_targets_duplicates_and_invalid_proofs() {
        let mut ledger = json!({"schema":"work-spec-reconciliation-ledger","attempt_path":"execution/TASK-001/ATTEMPT-001/attempt.json","entries":[{"deviation_id":"DEVIATION-001","outcome":"incorporated","target":"task_and_execution","attempt_sha256":"a".repeat(64),"reconciliation_fingerprint":"b".repeat(64)}]});
        render_ledger(&ledger).unwrap();
        ledger["entries"][0]["target"] = json!("plan_and_task");
        assert!(render_ledger(&ledger).is_err());
        ledger["entries"][0]["target"] = json!("task_and_execution");
        ledger["entries"][0]["attempt_sha256"] = json!("invalid");
        assert!(render_ledger(&ledger).is_err());
        ledger["entries"][0]["attempt_sha256"] = json!("a".repeat(64));
        let duplicate = ledger["entries"][0].clone();
        ledger["entries"].as_array_mut().unwrap().push(duplicate);
        assert!(render_ledger(&ledger).is_err());
    }

    #[test]
    fn ledger_rendering_preserves_field_order_and_newline() {
        let raw = render_ledger(
            &json!({"schema":"work-spec-reconciliation-ledger","attempt_path":"a","entries":[]}),
        )
        .unwrap();
        assert!(raw.starts_with(b"{\n  \"schema\": \"work-spec-reconciliation-ledger\","));
        assert!(raw.ends_with(b"\n"));
    }
}
