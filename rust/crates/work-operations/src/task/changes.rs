//! Revision history shape for formal TASK indexes.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::task::TaskIssue;

fn issue(reason_code: &'static str, message: &'static str) -> TaskIssue {
    TaskIssue {
        reason_code,
        message,
        details: json!({}),
    }
}

fn exact(value: &Value, required: &[&str], optional: &[&str]) -> Result<(), TaskIssue> {
    let Some(object) = value.as_object() else {
        return Err(issue("expected_object", "A JSON object is required."));
    };
    if required.iter().any(|field| !object.contains_key(*field))
        || object
            .keys()
            .any(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
    {
        return Err(issue(
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
        ));
    }
    Ok(())
}

fn numbered(value: &Value, prefix: &str) -> Option<u32> {
    let suffix = value.as_str()?.strip_prefix(prefix)?.strip_prefix('-')?;
    (suffix.len() == 3 && suffix.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| suffix.parse().ok())
        .flatten()
}

fn nonempty(value: &Value) -> bool {
    value.as_str().is_some_and(|text| !text.trim().is_empty())
}

fn strings(value: &Value) -> Result<Vec<&str>, TaskIssue> {
    let Some(values) = value.as_array().filter(|values| !values.is_empty()) else {
        return Err(issue(
            "invalid_string_array",
            "A string array with the required cardinality is required.",
        ));
    };
    let result: Vec<&str> = values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| issue("empty_text_value", "A non-empty string is required."))
        })
        .collect::<Result<_, _>>()?;
    if result.iter().collect::<BTreeSet<_>>().len() != result.len() {
        return Err(issue(
            "duplicate_array_value",
            "Array values must be unique.",
        ));
    }
    Ok(result)
}

fn date(value: &Value) -> bool {
    let Some(value) = value.as_str() else {
        return false;
    };
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return false;
    }
    let year: u32 = value[0..4].parse().unwrap_or(0);
    let month: usize = value[5..7].parse().unwrap_or(0);
    let day: u32 = value[8..10].parse().unwrap_or(0);
    if year == 0 || !(1..=12).contains(&month) {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    (1..=days[month - 1]).contains(&day)
}

pub fn validate_index_changes(
    value: &Value,
    spec_id: &str,
    task_ids: &[String],
) -> Result<(), TaskIssue> {
    let Some(changes) = value.as_array().filter(|changes| !changes.is_empty()) else {
        return Err(issue("invalid_item_array", "changes must be non-empty."));
    };
    let current_spec = numbered(&json!(spec_id), "TASK-SPEC")
        .ok_or_else(|| issue("invalid_task_spec_id", "Invalid TASK spec ID."))?;
    let mut previous = 0;
    let mut previous_spec = 0;
    for (position, change) in changes.iter().enumerate() {
        let latest = position + 1 == changes.len();
        exact(
            change,
            &["id", "spec_id", "date", "reason", "affected_ids", "edits"],
            &[],
        )?;
        let number = numbered(&change["id"], "TASK-CHANGE")
            .filter(|number| *number > previous)
            .ok_or_else(|| issue("invalid_or_unsorted_id", "Invalid TASK change ID."))?;
        previous = number;
        let change_spec = numbered(&change["spec_id"], "TASK-SPEC")
            .filter(|number| *number > previous_spec && *number <= current_spec)
            .ok_or_else(|| {
                issue(
                    "change_spec_mismatch",
                    "Change spec_id history must be strictly increasing through the current spec.",
                )
            })?;
        previous_spec = change_spec;
        if !date(&change["date"]) {
            return Err(issue("invalid_change_date", "Use YYYY-MM-DD."));
        }
        if !nonempty(&change["reason"]) {
            return Err(issue("empty_text_value", "A non-empty string is required."));
        }
        let affected = strings(&change["affected_ids"])?;
        for id in affected {
            let task_id = id.split('/').next().expect("split yields first segment");
            if numbered(&json!(task_id), "TASK").is_none()
                || (latest && !task_ids.iter().any(|known| known == task_id))
            {
                return Err(issue(
                    "invalid_reference",
                    "Unknown or invalid affected TASK ID.",
                ));
            }
        }
        let edits = change["edits"]
            .as_array()
            .filter(|edits| !edits.is_empty())
            .ok_or_else(|| issue("invalid_item_array", "Change edits must be non-empty."))?;
        for edit in edits {
            exact(
                edit,
                &["artifact", "operation", "path"],
                &["task_id", "before", "after"],
            )?;
            let artifact = edit["artifact"].as_str();
            if artifact == Some("task_item") {
                let id = &edit["task_id"];
                if numbered(id, "TASK").is_none()
                    || (latest
                        && edit["operation"] != "remove"
                        && !task_ids.iter().any(|known| id == known))
                {
                    return Err(issue(
                        "invalid_reference",
                        "A TASK item edit requires a known task_id.",
                    ));
                }
            } else if artifact != Some("task_index") || edit.get("task_id").is_some() {
                return Err(issue(
                    "invalid_change_edit",
                    "Invalid TASK change artifact.",
                ));
            }
            let expected: BTreeSet<&str> = match edit["operation"].as_str() {
                Some("add") => ["artifact", "operation", "path", "after"]
                    .into_iter()
                    .collect(),
                Some("replace") => ["artifact", "operation", "path", "before", "after"]
                    .into_iter()
                    .collect(),
                Some("remove") => ["artifact", "operation", "path", "before"]
                    .into_iter()
                    .collect(),
                _ => {
                    return Err(issue(
                        "invalid_change_edit",
                        "Invalid TASK change edit fields.",
                    ));
                }
            };
            let actual: BTreeSet<&str> = edit
                .as_object()
                .expect("checked edit")
                .keys()
                .map(String::as_str)
                .collect();
            let mut expected = expected;
            if artifact == Some("task_item") {
                expected.insert("task_id");
            }
            if actual != expected {
                return Err(issue(
                    "invalid_change_edit",
                    "Invalid TASK change edit fields.",
                ));
            }
            if !edit["path"]
                .as_str()
                .is_some_and(|path| path.starts_with('/'))
            {
                return Err(issue(
                    "invalid_json_pointer",
                    "Change path must be a JSON Pointer.",
                ));
            }
        }
    }
    if previous_spec != current_spec {
        return Err(issue(
            "change_spec_mismatch",
            "The latest change must belong to the current spec.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_task_change_cases_match_formal_artifact_edits() {
        let ids = vec!["TASK-001".into()];
        let change = json!({
            "id":"TASK-CHANGE-001","spec_id":"TASK-SPEC-001","date":"2026-09-07",
            "reason":"Update task details.","affected_ids":["TASK-001","TASK-001/FILE-001"],
            "edits":[
                {"artifact":"task_index","operation":"add","path":"/summary","after":"New"},
                {"artifact":"task_item","task_id":"TASK-001","operation":"replace","path":"/goal","before":"Old","after":"New"},
                {"artifact":"task_index","operation":"remove","path":"/risks/0","before":{}}
            ]
        });
        let valid = json!([change]);
        validate_index_changes(&valid, "TASK-SPEC-001", &ids).unwrap();

        let mut unsorted = json!([valid[0].clone(), valid[0].clone()]);
        unsorted[0]["id"] = json!("TASK-CHANGE-002");
        assert_eq!(
            validate_index_changes(&unsorted, "TASK-SPEC-001", &ids)
                .unwrap_err()
                .reason_code,
            "invalid_or_unsorted_id"
        );
        let mut invalid = valid.clone();
        invalid[0]["spec_id"] = json!("TASK-SPEC-002");
        assert_eq!(
            validate_index_changes(&invalid, "TASK-SPEC-001", &ids)
                .unwrap_err()
                .reason_code,
            "change_spec_mismatch"
        );
        invalid = valid.clone();
        invalid[0]["date"] = json!("2026-02-30");
        assert_eq!(
            validate_index_changes(&invalid, "TASK-SPEC-001", &ids)
                .unwrap_err()
                .reason_code,
            "invalid_change_date"
        );
        invalid = valid.clone();
        invalid[0]["affected_ids"] = json!(["TASK-999"]);
        assert_eq!(
            validate_index_changes(&invalid, "TASK-SPEC-001", &ids)
                .unwrap_err()
                .reason_code,
            "invalid_reference"
        );
        invalid = valid.clone();
        invalid[0]["plan_change_ids"] = json!(["PLAN-CHANGE-001"]);
        assert_eq!(
            validate_index_changes(&invalid, "TASK-SPEC-001", &ids)
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
        invalid = valid.clone();
        invalid[0]["edits"] =
            json!([{"artifact":"task_index","operation":"add","path":"/summary","before":"Old"}]);
        assert_eq!(
            validate_index_changes(&invalid, "TASK-SPEC-001", &ids)
                .unwrap_err()
                .reason_code,
            "invalid_change_edit"
        );
        invalid = valid.clone();
        invalid[0]["edits"] =
            json!([{"artifact":"task_index","operation":"add","path":"summary","after":"New"}]);
        assert_eq!(
            validate_index_changes(&invalid, "TASK-SPEC-001", &ids)
                .unwrap_err()
                .reason_code,
            "invalid_json_pointer"
        );
        invalid = valid.clone();
        invalid[0]["edits"] = json!([{"operation":"add","path":"/summary","after":"New"}]);
        assert_eq!(
            validate_index_changes(&invalid, "TASK-SPEC-001", &ids)
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
        invalid = valid.clone();
        invalid[0]["edits"] = json!([{"artifact":"task_index","task_id":"TASK-001","operation":"add","path":"/summary","after":"New"}]);
        assert_eq!(
            validate_index_changes(&invalid, "TASK-SPEC-001", &ids)
                .unwrap_err()
                .reason_code,
            "invalid_change_edit"
        );
        invalid = valid.clone();
        invalid[0]["edits"] = json!([{"artifact":"task_item","task_id":"TASK-999","operation":"add","path":"/goal","after":"New"}]);
        assert_eq!(
            validate_index_changes(&invalid, "TASK-SPEC-001", &ids)
                .unwrap_err()
                .reason_code,
            "invalid_reference"
        );
    }

    #[test]
    fn revision_edits_require_matching_spec_and_complete_fields() {
        let ids = vec!["TASK-001".into()];
        let changes = json!([{"id":"TASK-CHANGE-001","spec_id":"TASK-SPEC-002","date":"2026-09-25","reason":"Update task.",
            "affected_ids":["TASK-001"],"edits":[{"artifact":"task_item","task_id":"TASK-001","operation":"replace","path":"/goal","before":"old","after":"new"}]}]);
        validate_index_changes(&changes, "TASK-SPEC-002", &ids).unwrap();
        let mut invalid = changes;
        invalid[0]["edits"][0]
            .as_object_mut()
            .unwrap()
            .remove("before");
        assert_eq!(
            validate_index_changes(&invalid, "TASK-SPEC-002", &ids)
                .unwrap_err()
                .reason_code,
            "invalid_change_edit"
        );
    }
}
