//! Fail closed when an unisolated execution record can bypass project-file publication.
use super::{ExecutionIssue, issue};

pub fn input_paths(collection: &Value, task: &Value) -> Result<Vec<String>, ExecutionIssue> {
    let mut paths = std::collections::BTreeSet::new();
    for input in task["inputs"].as_array().into_iter().flatten() {
        let path = if input["kind"] == "project_state" {
            input["source"].as_str()
        } else if input["kind"] == "task_output" {
            let (producer, file) = input["source"]
                .as_str()
                .and_then(|s| s.split_once('/'))
                .ok_or_else(|| {
                    issue(
                        "file_transaction_input_unverified",
                        "A file input must have a verified source.",
                        json!({}),
                    )
                })?;
            collection["tasks"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|t| t["id"] == producer)
                .and_then(|t| t["files"].as_array())
                .and_then(|fs| fs.iter().find(|f| f["id"] == file))
                .and_then(|f| f["path"].as_str().or_else(|| f["destination"].as_str()))
        } else {
            continue;
        };
        let path = path
            .filter(|p| crate::derivation::identity::runtime_relative_path(p))
            .ok_or_else(|| {
                issue(
                    "file_transaction_input_unverified",
                    "A file input must have a verified source.",
                    json!({}),
                )
            })?;
        paths.insert(path.to_owned());
    }
    Ok(paths.into_iter().collect())
}
use serde_json::{Value, json};
use work_model::execution::VerifiedEffectBoundary;

pub fn require_record_boundary(
    task: &Value,
    record_id: &str,
    capability: VerifiedEffectBoundary,
) -> Result<(), ExecutionIssue> {
    let manual = record_id.starts_with("VAL-")
        && task["validations"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|v| v["id"] == record_id && v["kind"] == "manual");
    if manual || capability != VerifiedEffectBoundary::Unknown {
        return Ok(());
    }
    Err(issue(
        "file_transaction_effect_boundary_unverified",
        "An unisolated CMD, automated VAL or local/external OP cannot bypass project-file publication. Prepare bytes in authorized staging and use manual acceptance, or review an enforceable effect boundary first.",
        json!({"task_id":task["id"],"record_id":record_id,"needs_confirmation":true}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn argv_shell_and_external_effects_never_imply_isolation() {
        let task = json!({"files":[{"path":"a"}],"validations":[{"id":"VAL-001","kind":"manual"},{"id":"VAL-002","kind":"automated"}]});
        for id in ["CMD-001", "OP-001", "VAL-002"] {
            assert_eq!(
                require_record_boundary(&task, id, VerifiedEffectBoundary::Unknown)
                    .unwrap_err()
                    .reason_code,
                "file_transaction_effect_boundary_unverified"
            );
        }
        assert!(require_record_boundary(&task, "VAL-001", VerifiedEffectBoundary::Unknown).is_ok());
        assert!(
            require_record_boundary(
                &json!({"files":[]}),
                "CMD-001",
                VerifiedEffectBoundary::Unknown
            )
            .is_err()
        );
        for capability in [
            VerifiedEffectBoundary::ReadOnly,
            VerifiedEffectBoundary::Isolated,
        ] {
            for id in ["CMD-001", "OP-001", "VAL-002"] {
                assert!(require_record_boundary(&task, id, capability).is_ok());
            }
        }
        for id in ["CMD-001", "OP-001", "VAL-002"] {
            let mut fileless = task.clone();
            fileless["files"] = json!([]);
            assert!(
                require_record_boundary(&fileless, id, VerifiedEffectBoundary::Unknown).is_err()
            );
        }
    }
}
