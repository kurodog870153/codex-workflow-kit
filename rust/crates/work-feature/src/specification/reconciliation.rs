//! Pure reconciliation preview for closed Attempts and reviewed deviation choices.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use work_operations::derivation::fingerprint;

use crate::error::{ExitCode, WorkError};

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

pub fn preview_reconciliation(
    request: &Value,
    attempt_path: &str,
    attempt_raw: &[u8],
    attempt: &Value,
    execution_index: &Value,
    existing_ledger: &Value,
    migration_preview: Option<&Value>,
) -> Result<Value, WorkError> {
    crate::specification::reconciliation_input::validate_preview_fields(request)?;
    crate::specification::reconciliation_input::validate_ledger_entries(existing_ledger)?;
    if existing_ledger["schema"] != "work-spec-reconciliation-ledger/v1"
        || existing_ledger["attempt_path"] != attempt_path
    {
        return Err(fail(
            "reconciliation_ledger",
            "The ledger must belong to the selected Attempt.",
        ));
    }
    if request["schema"] != "work-spec-reconciliation-preview-request/v1" {
        return Err(fail(
            "reconciliation_preview_schema",
            "A reconciliation preview request is required.",
        ));
    }
    if request["attempt_path"] != attempt_path {
        return Err(fail(
            "reconciliation_attempt_identity",
            "The Attempt path changed.",
        ));
    }
    if attempt["status"] == "in_progress" {
        return Err(fail(
            "reconciliation_attempt_open",
            "Reconciliation requires a closed Attempt.",
        ));
    }
    let parent = attempt_path
        .rsplit_once("/TASK-")
        .map(|(prefix, _)| prefix)
        .ok_or_else(|| {
            fail(
                "reconciliation_attempt_identity",
                "The Attempt path is invalid.",
            )
        })?;
    let task_id = attempt["task_id"].as_str().ok_or_else(|| {
        fail(
            "reconciliation_attempt_identity",
            "The Attempt task ID is invalid.",
        )
    })?;
    let attempt_id = attempt["attempt_id"].as_str().ok_or_else(|| {
        fail(
            "reconciliation_attempt_identity",
            "The Attempt ID is invalid.",
        )
    })?;
    if attempt_path != format!("{parent}/{task_id}/{attempt_id}/attempt.json") {
        return Err(fail(
            "reconciliation_attempt_identity",
            "The Attempt path does not match its identity.",
        ));
    }
    let rows = execution_index["tasks"].as_array().ok_or_else(|| {
        fail(
            "reconciliation_attempt_not_latest",
            "The execution index has no TASK rows.",
        )
    })?;
    let matching = rows
        .iter()
        .filter(|row| row["id"] == task_id)
        .collect::<Vec<_>>();
    if execution_index["schema"] != "work-execution-index/v1"
        || matching.len() != 1
        || matching[0]["latest_attempt"] != attempt_id
        || execution_index["task_spec_id"] != attempt["task_spec_id"]
    {
        return Err(fail(
            "reconciliation_attempt_not_latest",
            "The closed Attempt is no longer the latest verified execution source.",
        ));
    }
    let ledger_path = format!("{parent}/{task_id}/{attempt_id}/reconciliation.json");
    let existing = existing_ledger["entries"].as_array().ok_or_else(|| {
        fail(
            "reconciliation_ledger",
            "The existing reconciliation ledger is invalid.",
        )
    })?;
    let recorded = existing
        .iter()
        .filter_map(|row| row["deviation_id"].as_str())
        .collect::<BTreeSet<_>>();
    let deviations = attempt["execution_deviations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| {
            row["decision"]["outcome"] == "approved"
                && row["reconciliation_status"] == "pending"
                && row["deviation_id"]
                    .as_str()
                    .is_some_and(|id| !recorded.contains(id))
        })
        .collect::<Vec<_>>();
    if deviations.is_empty() {
        return Err(fail(
            "reconciliation_nothing_pending",
            "The Attempt has no approved pending deviations.",
        ));
    }
    let pending = deviations
        .iter()
        .filter_map(|row| row["deviation_id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let selected = match request["choice"].as_str() {
        Some("all") => pending.clone(),
        Some("selective") => {
            let ids = request["deviation_ids"]
                .as_array()
                .ok_or_else(|| {
                    fail(
                        "reconciliation_unknown_deviation",
                        "Selected deviation IDs are required.",
                    )
                })?
                .iter()
                .filter_map(|row| row.as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            if ids.iter().any(|id| !pending.contains(id)) {
                return Err(fail(
                    "reconciliation_unknown_deviation",
                    "Selected deviations are not approved and pending.",
                ));
            }
            ids
        }
        Some("retain_only") => Vec::new(),
        _ => {
            return Err(fail(
                "reconciliation_choice",
                "The reconciliation choice is invalid.",
            ));
        }
    };
    let retained = pending
        .iter()
        .filter(|id| !selected.contains(id))
        .cloned()
        .collect::<Vec<_>>();
    let mut classifications = serde_json::Map::new();
    for deviation in &deviations {
        let id = deviation["deviation_id"]
            .as_str()
            .expect("validated deviation ID");
        let target = if !selected.iter().any(|selected_id| selected_id == id) {
            "retain_only"
        } else if [
            "requirement_changed",
            "scope_changed",
            "deliverables_changed",
            "acceptance_criteria_changed",
            "safety_boundary_changed",
            "external_side_effect_boundary_changed",
        ]
        .iter()
        .any(|field| deviation["proposal"]["impact"][*field] == true)
        {
            "task_and_execution"
        } else {
            "task_only"
        };
        classifications.insert(id.to_owned(), json!(target));
    }
    let attempt_sha = fingerprint::raw(attempt_raw);
    let migration_fingerprint =
        migration_preview.map_or(Value::Null, |value| value["fingerprint"].clone());
    let evidence = json!({"request":request,"attempt_sha256":attempt_sha,
        "execution_index":execution_index,"existing_ledger":existing_ledger,
        "pending_deviation_ids":pending,"selected_deviation_ids":selected,
        "retained_deviation_ids":retained,"deviation_classifications":classifications,
        "migration_fingerprint":migration_fingerprint});
    let fingerprint = fingerprint::structured(&evidence).map_err(|_| {
        fail(
            "invalid_contract_value",
            "Reconciliation evidence cannot be fingerprinted.",
        )
    })?;
    let mut entries = existing.clone();
    for id in &pending {
        entries.push(json!({"deviation_id":id,
            "outcome":if selected.contains(id) {"incorporated"} else {"retained"},
            "target":classifications[id],"attempt_sha256":attempt_sha,
            "reconciliation_fingerprint":fingerprint}));
    }
    entries.sort_by(|left, right| {
        left["deviation_id"]
            .as_str()
            .cmp(&right["deviation_id"].as_str())
    });
    let ledger = json!({"schema":"work-spec-reconciliation-ledger/v1",
        "attempt_path":attempt_path,"entries":entries});
    let ready = migration_preview.is_none_or(|value| value["writable_ready"] == true);
    Ok(work_model::specification::verified::<
        work_model::specification::SpecReconciliationPreview,
    >(json!({"schema":"work-spec-reconciliation-preview/v1",
        "status":if ready {"ready"} else {"blocked"},
        "attempt_path":attempt_path,"attempt_sha256":attempt_sha,
        "choice":request["choice"],"pending_deviation_ids":pending,
        "selected_deviation_ids":selected,"retained_deviation_ids":retained,
        "migration_preview":migration_preview,"deviation_classifications":classifications,
        "ledger_path":ledger_path,"ledger":ledger,"fingerprint":fingerprint,
        "publication_required":true,"publication_ready":ready})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    #[test]
    fn ledger_keeps_exact_reviewed_scope_and_task_execution_impact() {
        let fixture = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/fixtures/specification-reconciliation"
        ));
        let mut attempt: Value =
            serde_json::from_slice(&fs::read(fixture.join("attempt.json")).unwrap()).unwrap();
        let index: Value =
            serde_json::from_slice(&fs::read(fixture.join("execution-index.json")).unwrap())
                .unwrap();
        let mut request: Value =
            serde_json::from_slice(&fs::read(fixture.join("selective-request.json")).unwrap())
                .unwrap();
        let path = request["attempt_path"].as_str().unwrap().to_owned();
        attempt["execution_deviations"][0]["proposal"]["impact"]["acceptance_criteria_changed"] =
            json!(true);
        let mut retained = attempt["execution_deviations"][0].clone();
        retained["deviation_id"] = json!("DEVIATION-002");
        let mut unapproved = retained.clone();
        unapproved["deviation_id"] = json!("DEVIATION-003");
        unapproved["decision"]["outcome"] = json!("declined");
        attempt["execution_deviations"]
            .as_array_mut()
            .unwrap()
            .extend([retained, unapproved]);
        let raw = serde_json::to_vec(&attempt).unwrap();
        let ledger =
            json!({"schema":"work-spec-reconciliation-ledger/v1","attempt_path":path,"entries":[]});
        let migration: Value =
            serde_json::from_slice(&fs::read(fixture.join("migration-preview.json")).unwrap())
                .unwrap();
        let preview = preview_reconciliation(
            &request,
            &path,
            &raw,
            &attempt,
            &index,
            &ledger,
            Some(&migration),
        )
        .unwrap();
        assert_eq!(preview["selected_deviation_ids"], json!(["DEVIATION-001"]));
        assert_eq!(preview["retained_deviation_ids"], json!(["DEVIATION-002"]));
        assert_eq!(
            preview["deviation_classifications"]["DEVIATION-001"],
            "task_and_execution"
        );
        assert_eq!(
            preview["deviation_classifications"]["DEVIATION-002"],
            "retain_only"
        );
        assert_eq!(preview["ledger"]["entries"].as_array().unwrap().len(), 2);
        assert!(
            !preview["deviation_classifications"]
                .as_object()
                .unwrap()
                .contains_key("DEVIATION-003")
        );
        work_operations::specification::reconciliation_ledger::validate_ledger(&preview["ledger"])
            .unwrap();
        request["deviation_ids"] = json!(["DEVIATION-003"]);
        assert_eq!(
            preview_reconciliation(
                &request,
                &path,
                &raw,
                &attempt,
                &index,
                &ledger,
                Some(&migration)
            )
            .unwrap_err()
            .reason_code,
            "reconciliation_unknown_deviation"
        );
        request["deviation_ids"] = json!(["DEVIATION-001", "DEVIATION-001"]);
        assert!(
            preview_reconciliation(
                &request,
                &path,
                &raw,
                &attempt,
                &index,
                &ledger,
                Some(&migration)
            )
            .is_err()
        );
        request["deviation_ids"] = json!(["DEVIATION-001"]);
        let mut other = ledger.clone();
        other["attempt_path"] = json!("other");
        assert_eq!(
            preview_reconciliation(
                &request,
                &path,
                &raw,
                &attempt,
                &index,
                &other,
                Some(&migration)
            )
            .unwrap_err()
            .reason_code,
            "reconciliation_ledger"
        );
        let mut mixed = preview["ledger"].clone();
        mixed["entries"][0]["target"] = json!("plan_and_task");
        assert!(
            preview_reconciliation(
                &request,
                &path,
                &raw,
                &attempt,
                &index,
                &mixed,
                Some(&migration)
            )
            .is_err()
        );
    }

    #[test]
    fn reviewed_choices_and_fingerprints_match_python() {
        let fixture = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/fixtures/specification-reconciliation"
        ));
        let attempt: Value =
            serde_json::from_slice(&fs::read(fixture.join("attempt.json")).unwrap()).unwrap();
        let attempt_raw = fs::read(fixture.join("attempt-raw.txt")).unwrap();
        let index: Value =
            serde_json::from_slice(&fs::read(fixture.join("execution-index.json")).unwrap())
                .unwrap();
        let migration: Value =
            serde_json::from_slice(&fs::read(fixture.join("migration-preview.json")).unwrap())
                .unwrap();
        for variant in ["all", "selective", "retain-only"] {
            let request: Value = serde_json::from_slice(
                &fs::read(fixture.join(format!("{variant}-request.json"))).unwrap(),
            )
            .unwrap();
            let expected: Value = serde_json::from_slice(
                &fs::read(fixture.join(format!("{variant}-expected.json"))).unwrap(),
            )
            .unwrap();
            let supplied = (variant != "retain-only").then_some(&migration);
            let actual = preview_reconciliation(
                &request,
                request["attempt_path"].as_str().unwrap(),
                &attempt_raw,
                &attempt,
                &index,
                &json!({"schema":"work-spec-reconciliation-ledger/v1",
                    "attempt_path":request["attempt_path"],"entries":[]}),
                supplied,
            )
            .unwrap();
            assert_eq!(actual, expected, "variant {variant}");
        }
    }
}
