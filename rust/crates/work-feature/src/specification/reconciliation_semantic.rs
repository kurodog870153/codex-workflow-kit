//! Select a closed Attempt and semantic reconciliation inputs through ports.

use crate::error::{ExitCode, WorkError};
use crate::specification::reconciliation_input::{
    contract_value, position_error, validate_semantic_fields,
};
use serde_json::{Value, json};
use work_operations::canonical::parse_json_contract;
use work_operations::protocol::TASK_ID_PREFIX;

pub trait ReconciliationSemanticRepository {
    fn execution_source(&self, requirement: &str) -> Result<String, WorkError>;
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError>;
    fn exists(&self, relative: &str) -> Result<bool, WorkError>;
}

pub struct SemanticSelection {
    pub requirement: String,
    pub choice: String,
    pub attempt_path: String,
    pub selected: Vec<String>,
    pub reason: Value,
    pub edits: Value,
    pub decisions: Value,
}

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

pub fn prepare_semantic_selection(
    repository: &impl ReconciliationSemanticRepository,
    semantic: &Value,
) -> Result<SemanticSelection, WorkError> {
    validate_semantic_fields(semantic)?;
    let _: work_model::specification::SpecReconciliationPrepareRequest =
        serde_json::from_value(semantic.clone()).map_err(|cause| {
            WorkError::new(
                ExitCode::Contract,
                "invalid_contract_value",
                "Reconciliation semantic fields must match their contract.",
                json!({"cause":cause.to_string()}),
            )
        })?;
    let requirement = semantic["requirement_id"].as_str().ok_or_else(|| {
        fail(
            "reconciliation_execution_source_ambiguous",
            "A requirement ID is required.",
        )
    })?;
    let task_position = semantic["task_position"]
        .as_u64()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            fail(
                "reconciliation_task_position",
                "A TASK position is required.",
            )
        })? as usize;
    let attempt_position = semantic["attempt_position"]
        .as_u64()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            fail(
                "reconciliation_attempt_position",
                "An Attempt position is required.",
            )
        })?;
    let choice = semantic["choice"]
        .as_str()
        .filter(|value| matches!(*value, "all" | "selective" | "retain_only"))
        .ok_or_else(|| contract_value("choice", "literal_error"))?;
    let positions = match semantic.get("deviation_positions") {
        None => Vec::new(),
        Some(value) => value
            .as_array()
            .ok_or_else(|| contract_value("deviation_positions", "list_type"))?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let location = format!("deviation_positions[{index}]");
                if value.as_i64().is_some_and(|position| position <= 0) {
                    return Err(contract_value(&location, "greater_than"));
                }
                value
                    .as_u64()
                    .ok_or_else(|| contract_value(&location, "int_type"))
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    let edits = semantic.get("edits").cloned().unwrap_or_else(|| json!([]));
    let decisions = semantic
        .get("semantic_decisions")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let reason = semantic.get("reason").cloned().unwrap_or(Value::Null);
    if positions.windows(2).any(|pair| pair[0] >= pair[1])
        || (choice == "retain_only"
            && (!positions.is_empty()
                || edits.as_array().is_some_and(|rows| !rows.is_empty())
                || !reason.is_null()
                || decisions.as_array().is_some_and(|rows| !rows.is_empty())))
        || (choice != "retain_only"
            && (reason.as_str().is_none_or(str::is_empty)
                || edits.as_array().is_none_or(Vec::is_empty)
                || (choice == "selective" && positions.is_empty())
                || (choice == "all" && !positions.is_empty())))
    {
        return Err(contract_value("contract", "value_error"));
    }
    if choice == "retain_only" {
        if semantic
            .get("sources")
            .is_some_and(|sources| sources.as_array().is_none_or(|rows| !rows.is_empty()))
        {
            return Err(contract_value("sources", "value_error"));
        }
    } else {
        crate::specification::migration_prepare::validate_semantic_request(&json!({
            "schema":"work-spec-migration-prepare-request", "mode":"revision",
            "requirement_id":requirement,"reason":reason,"edits":edits,
            "semantic_decisions":decisions,"sources":semantic["sources"],
        }))?;
    }
    let execution_dir = repository.execution_source(requirement)?;
    let index_path = format!("{execution_dir}/index.json");
    let index = parse_json_contract(&repository.read(&index_path)?).map_err(|_| {
        fail(
            "reconciliation_execution_source_ambiguous",
            "The execution index is invalid.",
        )
    })?;
    let _: work_model::execution::index::ExecutionIndex = serde_json::from_value(index.clone())
        .map_err(|_| {
            fail(
                "reconciliation_execution_source_ambiguous",
                "The execution index must match its current contract.",
            )
        })?;
    let row = index["tasks"]
        .as_array()
        .and_then(|rows| rows.get(task_position - 1))
        .filter(|row| row.is_object())
        .ok_or_else(|| {
            position_error(
                "reconciliation_task_position",
                "The TASK position does not exist.",
            )
        })?;
    let attempt_id = format!("ATTEMPT-{attempt_position:03}");
    if attempt_position > 999 || row["latest_attempt"] != attempt_id {
        return Err(position_error(
            "reconciliation_attempt_position",
            "Select the latest recorded Attempt position.",
        ));
    }
    let task_id = row["id"]
        .as_str()
        .filter(|id| id.starts_with(TASK_ID_PREFIX))
        .ok_or_else(|| {
            fail(
                "reconciliation_task_identity",
                "The execution TASK identity is invalid.",
            )
        })?;
    let attempt_path = format!("{execution_dir}/{task_id}/{attempt_id}/attempt.json");
    if !repository.exists(&attempt_path)? {
        return Err(fail(
            "reconciliation_attempt_missing",
            "The selected Attempt is missing.",
        ));
    }
    let raw = repository.read(&attempt_path).map_err(|_| {
        fail(
            "reconciliation_attempt_invalid",
            "The selected Attempt is unreadable.",
        )
    })?;
    let attempt = parse_json_contract(&raw).map_err(|_| {
        fail(
            "reconciliation_attempt_invalid",
            "The selected Attempt is unreadable.",
        )
    })?;
    work_operations::execution::attempt::validate_attempt_bytes(&attempt, &raw).map_err(
        |issue| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        },
    )?;
    if attempt["task_id"] != task_id
        || attempt["attempt_id"] != attempt_id
        || attempt["status"] == "in_progress"
    {
        return Err(fail(
            "reconciliation_attempt_identity",
            "The selected Attempt is not a matching closed Attempt.",
        ));
    }
    let deviations = attempt["execution_deviations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut selected = Vec::new();
    for position in positions {
        let position = position as usize;
        let id = deviations
            .get(position - 1)
            .and_then(|row| row["deviation_id"].as_str())
            .ok_or_else(|| {
                position_error(
                    "reconciliation_deviation_position",
                    "A selected deviation position does not exist.",
                )
            })?;
        selected.push(id.to_owned());
    }
    selected.sort();
    Ok(SemanticSelection {
        requirement: requirement.to_owned(),
        choice: choice.to_owned(),
        attempt_path,
        selected,
        reason,
        edits,
        decisions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MissingExecution;
    impl ReconciliationSemanticRepository for MissingExecution {
        fn execution_source(&self, requirement: &str) -> Result<String, WorkError> {
            assert_eq!(requirement, "example");
            Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "reconciliation_execution_source_ambiguous",
                "Exactly one execution index must match the requirement.",
                json!({}),
            ))
        }
        fn read(&self, _: &str) -> Result<Vec<u8>, WorkError> {
            panic!("missing execution must stop before reading")
        }
        fn exists(&self, _: &str) -> Result<bool, WorkError> {
            panic!("missing execution must stop before checking paths")
        }
    }

    #[test]
    fn missing_execution_stops_before_artifact_ports() {
        let semantic = json!({
            "schema":"work-spec-reconciliation-prepare-request",
            "requirement_id":"example",
            "task_position":1,
            "attempt_position":1,
            "choice":"retain_only"
        });
        let error = prepare_semantic_selection(&MissingExecution, &semantic)
            .err()
            .expect("source port must fail");
        assert_eq!(
            error.reason_code,
            "reconciliation_execution_source_ambiguous"
        );
    }
    #[test]
    fn incorporation_requires_reviewed_evidence_before_execution_discovery() {
        let request = json!({"schema":"work-spec-reconciliation-prepare-request","requirement_id":"example","task_position":1,"attempt_position":1,"choice":"all","reason":"Reviewed change",
            "edits":[{"target":{"artifact":"task_item","task_id":"TASK-001"},"field":"goal","after":"Reviewed goal"}],"semantic_decisions":[]});
        assert_eq!(
            prepare_semantic_selection(&MissingExecution, &request)
                .err()
                .unwrap()
                .reason_code,
            "invalid_contract_value"
        );
    }
}
