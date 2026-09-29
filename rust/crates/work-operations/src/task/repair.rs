//! Stable identity and complete path evidence for reviewed TASK repair.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::canonical::sha256_hex;
use crate::execution::index::build_initial_execution_index;
use crate::execution::{ExecutionIssue, derive_overall_status};

pub fn transaction_id(request_raw: &[u8]) -> String {
    format!(
        "TASK-REPAIR-{}",
        sha256_hex(request_raw)[..12].to_ascii_uppercase()
    )
}

pub fn content_fingerprints(contents: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, String> {
    contents
        .iter()
        .map(|(path, raw)| (path.clone(), sha256_hex(raw)))
        .collect()
}

pub fn evidence_fingerprints(
    contents: &BTreeMap<String, Vec<u8>>,
    paths: &[String],
) -> BTreeMap<String, Value> {
    paths
        .iter()
        .map(|path| {
            (
                path.clone(),
                contents
                    .get(path)
                    .map_or(Value::Null, |raw| Value::String(sha256_hex(raw))),
            )
        })
        .collect()
}

pub fn rebuild_execution_index(
    old: &Value,
    logical: &Value,
    validation: &Value,
    affected: &[String],
    change_id: &str,
) -> Result<Value, ExecutionIssue> {
    let mut result = build_initial_execution_index(logical, validation)?;
    let old_rows = old["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| Some((row["id"].as_str()?.to_owned(), row.clone())))
        .collect::<BTreeMap<_, _>>();
    let affected = affected.iter().collect::<BTreeSet<_>>();
    let rows = result["tasks"].as_array_mut().expect("initial index tasks");
    for fresh in rows.iter_mut() {
        let id = fresh["id"].as_str().unwrap_or("").to_owned();
        if let Some(mut prior) = old_rows.get(&id).cloned() {
            for field in ["skill_id", "task_item_sha256", "instructions_sha256"] {
                prior[field] = fresh[field].clone();
            }
            if affected.contains(&id) && prior["status"] != "pending" {
                if prior["status"] == "cancelled" {
                    return Err(ExecutionIssue {
                        reason_code: "spec_update_cancelled_task",
                        message: "A cancelled TASK requires an explicit lifecycle decision.",
                        details: json!({}),
                    });
                }
                prior["status"] = json!(if prior.get("latest_attempt").is_some() {
                    "pending_retry"
                } else {
                    "blocked"
                });
                prior["status_reason"] = json!({"kind":"task_change","ref":change_id});
            }
            *fresh = prior;
        }
    }
    let statuses = rows
        .iter()
        .map(|row| row["status"].as_str().unwrap_or("pending"))
        .collect::<Vec<_>>();
    result["overall_status"] = json!(derive_overall_status(&statuses));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_repair_identity_and_missing_evidence() {
        let request = b"{\"schema\":\"work-task-repair-request/v1\"}\n";
        assert_eq!(
            transaction_id(request),
            format!(
                "TASK-REPAIR-{}",
                sha256_hex(request)[..12].to_ascii_uppercase()
            )
        );
        let contents = BTreeMap::from([
            ("tasks/index.json".into(), b"index\n".to_vec()),
            ("tasks/tasks/TASK-001.json".into(), b"item\n".to_vec()),
        ]);
        let evidence = evidence_fingerprints(
            &contents,
            &["tasks/index.json".into(), "execution/index.json".into()],
        );
        assert_eq!(evidence["tasks/index.json"], sha256_hex(b"index\n"));
        assert!(evidence["execution/index.json"].is_null());
        assert_eq!(
            content_fingerprints(&contents)["tasks/index.json"],
            sha256_hex(b"index\n")
        );
        assert_eq!(
            content_fingerprints(&contents)["tasks/tasks/TASK-001.json"],
            sha256_hex(b"item\n")
        );
    }

    #[test]
    fn rebuild_preserves_history_and_requires_cancelled_decision() {
        let collection = json!({"requirement_id":"example","spec_id":"TASK-SPEC-001",
            "tasks":[{"id":"TASK-001"},{"id":"TASK-002"}]});
        let validation = json!({"task_skill_ids":{"TASK-001":"new","TASK-002":null},
            "task_item_sha256":{"TASK-001":"a".repeat(64),"TASK-002":"1".repeat(64)},
            "task_instructions_sha256":{"TASK-001":"b".repeat(64),"TASK-002":"2".repeat(64)},
            "task_collection_sha256":"c".repeat(64),"task_index_sha256":"d".repeat(64),
            "instructions_sha256":"e".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),"skill_selection_sha256":"0".repeat(64)});
        let mut old = build_initial_execution_index(&collection, &validation).unwrap();
        old["tasks"][0]["status"] = json!("completed");
        old["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        old["tasks"][0]["skill_id"] = json!("old");
        old["tasks"][1]["status"] = json!("blocked");
        old["tasks"][1]["skill_id"] = json!("old");
        let rebuilt = rebuild_execution_index(
            &old,
            &collection,
            &validation,
            &["TASK-001".into(), "TASK-002".into()],
            "TASK-REPAIR-ABC",
        )
        .unwrap();
        assert_eq!(rebuilt["tasks"][0]["status"], "pending_retry");
        assert_eq!(rebuilt["tasks"][0]["latest_attempt"], "ATTEMPT-001");
        assert_eq!(rebuilt["tasks"][0]["skill_id"], "new");
        assert_eq!(rebuilt["tasks"][0]["task_item_sha256"], "a".repeat(64));
        assert_eq!(rebuilt["tasks"][1]["status"], "blocked");
        assert_eq!(rebuilt["tasks"][1]["task_item_sha256"], "1".repeat(64));
        assert_eq!(
            rebuilt["tasks"][0]["status_reason"],
            json!({"kind":"task_change","ref":"TASK-REPAIR-ABC"})
        );
        assert_eq!(
            rebuilt["tasks"][1]["status_reason"],
            json!({"kind":"task_change","ref":"TASK-REPAIR-ABC"})
        );
        old["tasks"][0]["status"] = json!("cancelled");
        assert_eq!(
            rebuild_execution_index(
                &old,
                &collection,
                &validation,
                &["TASK-001".into()],
                "TASK-REPAIR-ABC"
            )
            .unwrap_err()
            .reason_code,
            "spec_update_cancelled_task"
        );
    }
}
