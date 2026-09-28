//! Reviewed source refresh transaction construction.

use crate::instruction::refresh_build::RefreshCandidate;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use work_operations::canonical::sha256_hex;
use work_operations::specification::transaction::{approval_sha256, encode_snapshot};

pub fn transaction(
    candidate: &RefreshCandidate,
    request: &Value,
    short: &str,
    history_sha256: &BTreeMap<String, String>,
) -> (Value, String) {
    let files = Value::Array(
        candidate
            .after
            .iter()
            .map(|(path, raw)| {
                json!({
                    "phase":10,"path":path,"operation":"replace",
                    "before":encode_snapshot(&candidate.before[path]),"after":encode_snapshot(raw),
                })
            })
            .collect(),
    );
    let source_sha = candidate
        .before
        .iter()
        .map(|(path, raw)| (path.clone(), json!(sha256_hex(raw))))
        .collect::<serde_json::Map<String, Value>>();
    let candidate_sha = candidate
        .after
        .iter()
        .map(|(path, raw)| (path.clone(), json!(sha256_hex(raw))))
        .collect::<serde_json::Map<String, Value>>();
    let metadata = json!({"request":request,"artifacts":candidate.artifacts,
        "affected_task_ids":[],"history_sha256":history_sha256,
        "source_sha256":source_sha,"candidate_sha256":candidate_sha});
    let approval = approval_sha256(&files, &metadata);
    let journal = json!({"schema":"work-spec-transaction/v1",
        "transaction_id":format!("SOURCE-REFRESH-{short}"),"approval_sha256":approval,
        "state":"prepared","published_count":0,"metadata":metadata,"files":files});
    (journal, approval)
}
