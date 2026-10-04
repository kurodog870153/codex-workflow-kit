//! Pure reconciliation request field checks.

use crate::error::{ExitCode, WorkError};
use serde_json::{Value, json};
use work_operations::protocol::valid_sha256;

pub fn contract_value(location: &str, validation_type: &str) -> WorkError {
    WorkError::new(
        ExitCode::Contract,
        "invalid_contract_value",
        "The JSON contract contains an invalid value.",
        json!({"location":location,"validation_type":validation_type,
            "issues":[{"location":location,"validation_type":validation_type}]}),
    )
}

pub fn position_error(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::Contract, reason, message, json!({}))
}

pub fn validate_semantic_fields(semantic: &Value) -> Result<(), WorkError> {
    let fields = semantic.as_object().ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "json_contract_not_object",
            "The JSON contract root must be an object.",
            json!({}),
        )
    })?;
    let required = [
        "schema",
        "requirement_id",
        "task_position",
        "attempt_position",
        "choice",
    ];
    let allowed = [
        "schema",
        "requirement_id",
        "task_position",
        "attempt_position",
        "choice",
        "deviation_positions",
        "reason",
        "edits",
        "semantic_decisions",
        "sources",
    ];
    let mut missing = required
        .iter()
        .filter(|key| !fields.contains_key(**key))
        .map(|key| (*key).to_owned())
        .collect::<Vec<_>>();
    let mut unknown = fields
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() || !unknown.is_empty() {
        missing.sort();
        unknown.sort();
        let mut issues = missing
            .iter()
            .map(|key| {
                json!({"location":key,
            "validation_type":"missing"})
            })
            .chain(
                unknown
                    .iter()
                    .map(|key| json!({"location":key,"validation_type":"extra_forbidden"})),
            )
            .collect::<Vec<_>>();
        issues.sort_by_key(|issue| issue["location"].as_str().unwrap().to_owned());
        return Err(WorkError::new(
            ExitCode::Contract,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location":"contract","missing":missing,"unknown":unknown,"issues":issues}),
        ));
    }
    if semantic["schema"] != "work-spec-reconciliation-prepare-request" {
        return Err(contract_value("schema", "literal_error"));
    }
    match semantic["requirement_id"].as_str() {
        None => return Err(contract_value("requirement_id", "string_type")),
        Some("") => return Err(contract_value("requirement_id", "string_too_short")),
        Some(value) if !value.chars().any(|character| !character.is_whitespace()) => {
            return Err(contract_value("requirement_id", "string_pattern_mismatch"));
        }
        _ => {}
    }
    for key in ["task_position", "attempt_position"] {
        if semantic[key].as_i64().is_some_and(|value| value <= 0) {
            return Err(contract_value(key, "greater_than"));
        }
        if semantic[key].as_u64().is_none() {
            return Err(contract_value(key, "int_type"));
        }
    }
    if semantic["choice"]
        .as_str()
        .is_none_or(|value| !matches!(value, "all" | "selective" | "retain_only"))
    {
        return Err(contract_value("choice", "literal_error"));
    }
    if semantic
        .get("reason")
        .is_some_and(|value| !value.is_null() && !value.is_string())
    {
        return Err(contract_value("reason", "string_type"));
    }
    for key in ["edits", "semantic_decisions", "sources"] {
        if semantic.get(key).is_some_and(|value| !value.is_array()) {
            return Err(contract_value(key, "list_type"));
        }
    }
    Ok(())
}

pub fn validate_preview_fields(request: &Value) -> Result<(), WorkError> {
    let fields = request.as_object().ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "json_contract_not_object",
            "The JSON contract root must be an object.",
            json!({}),
        )
    })?;
    let expected = [
        "schema",
        "attempt_path",
        "choice",
        "deviation_ids",
        "migration",
    ];
    let mut missing = expected
        .iter()
        .filter(|key| !fields.contains_key(**key))
        .map(|key| (*key).to_owned())
        .collect::<Vec<_>>();
    let mut unknown = fields
        .keys()
        .filter(|key| !expected.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() || !unknown.is_empty() {
        missing.sort();
        unknown.sort();
        let mut issues = missing
            .iter()
            .map(|key| {
                json!({"location":key,
            "validation_type":"missing"})
            })
            .chain(
                unknown
                    .iter()
                    .map(|key| json!({"location":key,"validation_type":"extra_forbidden"})),
            )
            .collect::<Vec<_>>();
        issues.sort_by_key(|issue| issue["location"].as_str().unwrap().to_owned());
        return Err(WorkError::new(
            ExitCode::Contract,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location":"contract","missing":missing,"unknown":unknown,"issues":issues}),
        ));
    }
    if request["schema"] != "work-spec-reconciliation-preview-request" {
        return Err(contract_value("schema", "literal_error"));
    }
    match request["attempt_path"].as_str() {
        None => return Err(contract_value("attempt_path", "string_type")),
        Some("") => return Err(contract_value("attempt_path", "string_too_short")),
        Some(value) if !value.chars().any(|character| !character.is_whitespace()) => {
            return Err(contract_value("attempt_path", "string_pattern_mismatch"));
        }
        _ => {}
    }
    let choice = request["choice"]
        .as_str()
        .filter(|choice| matches!(*choice, "all" | "selective" | "retain_only"))
        .ok_or_else(|| contract_value("choice", "literal_error"))?;
    let ids = request["deviation_ids"]
        .as_array()
        .ok_or_else(|| contract_value("deviation_ids", "list_type"))?;
    for (position, id) in ids.iter().enumerate() {
        if !id.is_string() {
            return Err(contract_value(
                &format!("deviation_ids[{position}]"),
                "string_type",
            ));
        }
    }
    if !request["migration"].is_null() && !request["migration"].is_object() {
        return Err(contract_value("migration", "model_type"));
    }
    if !request["migration"].is_null() {
        if request["migration"]["schema"] != "work-spec-migration-preview-request" {
            return Err(contract_value("migration.schema", "literal_error"));
        }
        let _: work_model::specification::SpecMigrationPreviewRequest =
            serde_json::from_value(request["migration"].clone())
                .map_err(|_| contract_value("migration", "value_error"))?;
    }
    let ids_sorted = ids
        .windows(2)
        .all(|pair| pair[0].as_str() < pair[1].as_str());
    if !ids_sorted
        || (choice == "selective" && (ids.is_empty() || request["migration"].is_null()))
        || (choice == "all" && (!ids.is_empty() || request["migration"].is_null()))
        || (choice == "retain_only" && (!ids.is_empty() || !request["migration"].is_null()))
    {
        return Err(contract_value("contract", "value_error"));
    }
    Ok(())
}

pub fn validate_ledger_entries(ledger: &Value) -> Result<(), WorkError> {
    let entries = ledger["entries"]
        .as_array()
        .ok_or_else(|| contract_value("entries", "list_type"))?;
    let mut previous = None;
    for (position, entry) in entries.iter().enumerate() {
        let fields = entry
            .as_object()
            .ok_or_else(|| contract_value(&format!("entries[{position}]"), "model_type"))?;
        let expected = [
            "deviation_id",
            "outcome",
            "target",
            "attempt_sha256",
            "reconciliation_fingerprint",
        ];
        if fields.len() != expected.len() || expected.iter().any(|key| !fields.contains_key(*key)) {
            return Err(contract_value(
                &format!("entries[{position}]"),
                "value_error",
            ));
        }
        let id = entry["deviation_id"].as_str().ok_or_else(|| {
            contract_value(&format!("entries[{position}].deviation_id"), "string_type")
        })?;
        let suffix = id.strip_prefix("DEVIATION-").unwrap_or("");
        if suffix.len() != 3 || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(contract_value(
                &format!("entries[{position}].deviation_id"),
                "string_pattern_mismatch",
            ));
        }
        for (field, choices) in [
            ("outcome", &["incorporated", "retained", "declined"][..]),
            (
                "target",
                &["task_only", "task_and_execution", "retain_only"][..],
            ),
        ] {
            if entry[field]
                .as_str()
                .is_none_or(|value| !choices.contains(&value))
            {
                return Err(contract_value(
                    &format!("entries[{position}].{field}"),
                    "literal_error",
                ));
            }
        }
        for field in ["attempt_sha256", "reconciliation_fingerprint"] {
            if entry[field]
                .as_str()
                .is_none_or(|value| !valid_sha256(value))
            {
                return Err(contract_value(
                    &format!("entries[{position}].{field}"),
                    "string_pattern_mismatch",
                ));
            }
        }
        if previous.is_some_and(|previous| previous >= id) {
            return Err(contract_value("contract", "value_error"));
        }
        previous = Some(id);
    }
    work_operations::specification::reconciliation_ledger::validate_ledger(ledger)
        .map_err(|_| contract_value("contract", "value_error"))?;
    Ok(())
}

pub fn unique_execution_source(mut matches: Vec<String>) -> Result<String, WorkError> {
    if matches.len() != 1 {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "reconciliation_execution_source_ambiguous",
            "Exactly one execution index must match the requirement.",
            json!({}),
        ));
    }
    Ok(matches.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_source_requires_exactly_one_match() {
        assert_eq!(
            unique_execution_source(vec!["outputs/work/executions/example".into()]).unwrap(),
            "outputs/work/executions/example"
        );
        for matches in [vec![], vec!["one".into(), "two".into()]] {
            assert_eq!(
                unique_execution_source(matches).unwrap_err().reason_code,
                "reconciliation_execution_source_ambiguous"
            );
        }
    }
}
