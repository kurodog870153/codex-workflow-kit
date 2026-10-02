//! Pure semantic TASK candidate keys and references.

use std::collections::{BTreeMap, HashSet};

use serde_json::{Value, json};

use crate::task::TaskIssue;

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> TaskIssue {
    TaskIssue {
        reason_code,
        message,
        details,
    }
}

fn key(value: &Value, location: &str) -> Result<String, TaskIssue> {
    let Some(key) = value.as_str() else {
        return Err(issue(
            "invalid_semantic_key",
            "Use a local lowercase semantic key, not a formal ID.",
            json!({"location": location}),
        ));
    };
    let valid = key
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase())
        && key.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        });
    if !valid {
        return Err(issue(
            "invalid_semantic_key",
            "Use a local lowercase semantic key, not a formal ID.",
            json!({"location": location}),
        ));
    }
    Ok(key.into())
}

fn fields(
    value: &Value,
    location: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<(), TaskIssue> {
    let Some(object) = value.as_object() else {
        return Err(issue(
            "expected_object",
            "A JSON object is required.",
            json!({"location": location}),
        ));
    };
    let missing: Vec<_> = required
        .iter()
        .filter(|field| !object.contains_key(**field))
        .copied()
        .collect();
    let unknown: Vec<_> = object
        .keys()
        .filter(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
        .cloned()
        .collect();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(issue(
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location": location, "missing": missing, "unknown": unknown}),
        ));
    }
    Ok(())
}

fn group_fields(group: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match group {
        "inputs" => (
            &["key", "kind", "precondition"],
            &["source", "dependency_position", "file_key"],
        ),
        "decisions" => (&["key", "statement", "rationale"], &[]),
        "files" => (&["key", "action"], &["path", "source", "destination"]),
        "risks" => (&["key", "condition", "impact", "mitigation"], &[]),
        "steps" => (&["key", "action", "references"], &[]),
        "commands" => (&["key", "mode"], &["argv", "script", "execution"]),
        "operations" => (
            &["key", "kind", "action", "target", "validation_key"],
            &["command_key"],
        ),
        "validations" => (
            &["key", "kind"],
            &[
                "command_keys",
                "pass_condition",
                "confirmer",
                "criteria",
                "acceptance_ids",
            ],
        ),
        _ => unreachable!(),
    }
}

fn resolve(
    keys: &BTreeMap<&str, HashSet<String>>,
    group: &str,
    value: &Value,
) -> Result<(), TaskIssue> {
    let local = key(value, group)?;
    if !keys.get(group).is_some_and(|known| known.contains(&local)) {
        return Err(issue(
            "invalid_semantic_reference",
            "A local semantic reference is unknown.",
            json!({"group": group, "key": local}),
        ));
    }
    Ok(())
}

fn validate_known_acceptance(value: &Value, known: Option<&[String]>) -> Result<(), TaskIssue> {
    let ids = value
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| {
            issue(
                "invalid_task_acceptance",
                "Acceptance references must be a nonempty stable-ID array.",
                json!({}),
            )
        })?;
    let mut seen = HashSet::new();
    for value in ids {
        let id = value.as_str().filter(|id| !id.is_empty()).ok_or_else(|| {
            issue(
                "invalid_task_acceptance",
                "Acceptance references must be stable IDs.",
                json!({}),
            )
        })?;
        let valid_main = crate::task::item::numbered(id, "ACCEPTANCE").is_some();
        let valid_technical = id.split_once("-ACCEPTANCE-").is_some_and(|(task, number)| {
            crate::task::item::numbered(task, "TASK").is_some()
                && crate::task::item::numbered(&format!("ACCEPTANCE-{number}"), "ACCEPTANCE")
                    .is_some()
        });
        if !valid_main && !valid_technical {
            return Err(issue(
                "invalid_task_acceptance",
                "Acceptance references must use stable main or Task-prefixed IDs.",
                json!({"id":id}),
            ));
        }
        if !seen.insert(id) || known.is_some_and(|rows| !rows.iter().any(|row| row == id)) {
            return Err(issue(
                "unknown_acceptance",
                "Acceptance IDs must be unique and known to the Task collection.",
                json!({"id":id}),
            ));
        }
    }
    Ok(())
}

pub fn validate_semantic_candidate(value: &Value, refined: bool) -> Result<(), TaskIssue> {
    let groups = [
        "inputs",
        "decisions",
        "files",
        "risks",
        "steps",
        "commands",
        "operations",
        "validations",
    ];
    let required: &[&str] = if refined {
        &[
            "steps",
            "validations",
            "acceptance_ids",
            "acceptance_criteria",
        ]
    } else {
        &[]
    };
    let mut optional = groups.to_vec();
    optional.extend(["acceptance_ids", "acceptance_criteria"]);
    fields(value, "task_candidate", required, &optional)?;
    if let Some(ids) = value.get("acceptance_ids") {
        let rows = ids.as_array().ok_or_else(|| {
            issue(
                "invalid_task_acceptance",
                "Responsibilities must be stable acceptance IDs.",
                json!({}),
            )
        })?;
        if !rows.is_empty() {
            validate_known_acceptance(ids, None)?;
        }
    }
    if let Some(criteria) = value.get("acceptance_criteria") {
        let criteria: Vec<work_model::task::source::TaskAcceptance> =
            serde_json::from_value(criteria.clone()).map_err(|_| {
                issue(
                    "invalid_task_acceptance",
                    "Technical acceptance needs stable IDs and criteria.",
                    json!({}),
                )
            })?;
        if criteria.is_empty() {
            return Err(issue(
                "missing_task_acceptance",
                "Technical acceptance definitions are required.",
                json!({}),
            ));
        }
        for criterion in &criteria {
            let prefix = criterion
                .id
                .split_once("-ACCEPTANCE-")
                .map(|(task, _)| format!("{task}-ACCEPTANCE-"))
                .unwrap_or_default();
            if criterion
                .id
                .split_once("-ACCEPTANCE-")
                .is_none_or(|(task, _)| crate::task::item::numbered(task, "TASK").is_none())
            {
                return Err(issue(
                    "invalid_task_acceptance",
                    "Technical acceptance IDs must identify a Task.",
                    json!({}),
                ));
            }
            crate::task::source::validate_acceptance(std::slice::from_ref(criterion), &prefix)?;
        }
        let mut seen = HashSet::new();
        if criteria.iter().any(|row| !seen.insert(&row.id)) {
            return Err(issue(
                "invalid_task_acceptance",
                "Technical acceptance IDs must be unique.",
                json!({}),
            ));
        }
    }
    let mut keys = BTreeMap::new();
    for group in groups {
        let Some(rows) = value.get(group) else {
            continue;
        };
        let Some(rows) = rows.as_array().filter(|rows| !refined || !rows.is_empty()) else {
            return Err(issue(
                "invalid_semantic_items",
                "A refined candidate group must be a nonempty array.",
                json!({"location": group}),
            ));
        };
        let mut seen = HashSet::new();
        let (required, optional) = group_fields(group);
        for (position, row) in rows.iter().enumerate() {
            fields(
                row,
                &format!("task_candidate.{group}[{}]", position + 1),
                required,
                optional,
            )?;
            let local = key(&row["key"], &format!("{group}[{}].key", position + 1))?;
            if !seen.insert(local.clone()) {
                return Err(issue(
                    "duplicate_semantic_key",
                    "Local semantic keys must be unique within a group.",
                    json!({"location": group, "key": local}),
                ));
            }
            if group == "steps" {
                let references = row["references"]
                    .as_array()
                    .filter(|refs| !refs.is_empty())
                    .ok_or_else(|| {
                        issue(
                            "invalid_semantic_reference",
                            "A step requires semantic references.",
                            json!({}),
                        )
                    })?;
                for reference in references {
                    fields(reference, "step.reference", &["kind", "key"], &[])?;
                }
            }
            if group == "validations" {
                if let Some(command_keys) = row.get("command_keys") {
                    if command_keys.as_array().is_none_or(Vec::is_empty) {
                        return Err(issue(
                            "invalid_semantic_reference",
                            "Automated validations require command keys.",
                            json!({}),
                        ));
                    }
                }
                if let Some(ids) = row.get("acceptance_ids") {
                    validate_known_acceptance(ids, None)?;
                }
            }
        }
        keys.insert(group, seen);
    }
    for row in value["steps"].as_array().into_iter().flatten() {
        for reference in row["references"].as_array().into_iter().flatten() {
            let group = reference["kind"].as_str().unwrap_or("");
            if !groups.contains(&group) || group == "steps" {
                return Err(issue(
                    "invalid_semantic_reference",
                    "A step reference kind is unsupported.",
                    json!({}),
                ));
            }
            resolve(&keys, group, &reference["key"])?;
        }
    }
    for row in value["validations"].as_array().into_iter().flatten() {
        for command in row["command_keys"].as_array().into_iter().flatten() {
            resolve(&keys, "commands", command)?;
        }
    }
    for row in value["operations"].as_array().into_iter().flatten() {
        resolve(&keys, "validations", &row["validation_key"])?;
        if let Some(command) = row.get("command_key") {
            resolve(&keys, "commands", command)?;
        }
    }
    Ok(())
}

pub fn build_semantic_candidate(
    value: &Value,
    acceptance_ids: &[String],
    dependency_ids: &[String],
    dependency_files: &BTreeMap<String, BTreeMap<String, String>>,
) -> Result<Value, TaskIssue> {
    validate_semantic_candidate(value, true)?;
    if !value["acceptance_ids"]
        .as_array()
        .expect("validated responsibilities")
        .is_empty()
    {
        validate_known_acceptance(&value["acceptance_ids"], Some(acceptance_ids))?;
    }
    let mut validation_acceptance = value["acceptance_ids"]
        .as_array()
        .expect("validated responsibilities")
        .iter()
        .map(|id| id.as_str().expect("validated acceptance").to_owned())
        .collect::<Vec<_>>();
    validation_acceptance.extend(
        value["acceptance_criteria"]
            .as_array()
            .expect("validated technical criteria")
            .iter()
            .map(|row| {
                row["id"]
                    .as_str()
                    .expect("validated technical ID")
                    .to_owned()
            }),
    );
    let groups = [
        "inputs",
        "decisions",
        "files",
        "risks",
        "steps",
        "commands",
        "operations",
        "validations",
    ];
    let prefixes = [
        "INPUT",
        "TASK-DECISION",
        "FILE",
        "RISK",
        "STEP",
        "CMD",
        "OP",
        "VAL",
    ];
    let maps: BTreeMap<&str, BTreeMap<String, String>> = groups
        .iter()
        .zip(prefixes)
        .map(|(group, prefix)| {
            let rows = value[*group].as_array().into_iter().flatten();
            (
                *group,
                rows.enumerate()
                    .map(|(position, row)| {
                        (
                            row["key"].as_str().expect("validated key").to_owned(),
                            format!("{prefix}-{:03}", position + 1),
                        )
                    })
                    .collect(),
            )
        })
        .collect();
    let lookup = |group: &str, local: &Value| -> Result<String, TaskIssue> {
        let local = key(local, group)?;
        maps.get(group)
            .and_then(|entries| entries.get(&local))
            .cloned()
            .ok_or_else(|| {
                issue(
                    "invalid_semantic_reference",
                    "A local semantic reference is unknown.",
                    json!({"group": group, "key": local}),
                )
            })
    };
    let mut result = serde_json::Map::new();
    for group in groups {
        let Some(rows) = value.get(group).and_then(Value::as_array) else {
            continue;
        };
        let mut formal_rows = Vec::new();
        for original in rows {
            let mut row = original.as_object().expect("validated row").clone();
            row.remove("key");
            row.insert(
                "id".into(),
                json!(maps[group][original["key"].as_str().expect("validated key")]),
            );
            match group {
                "steps" => {
                    let references = original["references"]
                        .as_array()
                        .expect("validated references")
                        .iter()
                        .map(|reference| {
                            lookup(
                                reference["kind"].as_str().expect("validated kind"),
                                &reference["key"],
                            )
                            .map(|id| json!(id))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    row.insert("references".into(), Value::Array(references));
                }
                "validations" => {
                    if let Some(commands) = row.remove("command_keys") {
                        let ids = commands
                            .as_array()
                            .expect("validated command keys")
                            .iter()
                            .map(|local| lookup("commands", local).map(|id| json!(id)))
                            .collect::<Result<Vec<_>, _>>()?;
                        row.insert("command_ids".into(), Value::Array(ids));
                    }
                    if let Some(ids) = row.get("acceptance_ids") {
                        validate_known_acceptance(ids, Some(&validation_acceptance))?;
                    }
                }
                "operations" => {
                    let id = lookup(
                        "validations",
                        &row.remove("validation_key")
                            .expect("validated validation key"),
                    )?;
                    row.insert("validation_id".into(), json!(id));
                    if let Some(local) = row.remove("command_key") {
                        row.insert("command_id".into(), json!(lookup("commands", &local)?));
                    }
                }
                "inputs" if row["kind"] == "task_output" => {
                    let expected: HashSet<_> = [
                        "id",
                        "kind",
                        "precondition",
                        "dependency_position",
                        "file_key",
                    ]
                    .into_iter()
                    .collect();
                    if row.keys().map(String::as_str).collect::<HashSet<_>>() != expected {
                        return Err(issue(
                            "invalid_semantic_reference",
                            "Task-output input requires a dependency position and file key.",
                            json!({}),
                        ));
                    }
                    let position = row
                        .remove("dependency_position")
                        .and_then(|value| value.as_u64())
                        .filter(|position| *position > 0)
                        .ok_or_else(|| {
                            issue(
                                "invalid_semantic_position",
                                "Positions must uniquely identify existing one-based items.",
                                json!({"location":"dependency_position"}),
                            )
                        })? as usize;
                    let dependency = dependency_ids.get(position - 1).ok_or_else(|| {
                        issue(
                            "invalid_semantic_position",
                            "Positions must uniquely identify existing one-based items.",
                            json!({"location":"dependency_position"}),
                        )
                    })?;
                    let file_key = key(
                        &row.remove("file_key").expect("validated file key"),
                        "file_key",
                    )?;
                    let file = dependency_files
                        .get(dependency)
                        .and_then(|files| files.get(&file_key))
                        .ok_or_else(|| {
                            issue(
                                "invalid_semantic_reference",
                                "Dependency file key is unknown.",
                                json!({}),
                            )
                        })?;
                    row.insert("source".into(), json!(format!("{dependency}/{file}")));
                }
                "inputs"
                    if row.contains_key("dependency_position") || row.contains_key("file_key") =>
                {
                    return Err(issue(
                        "invalid_semantic_reference",
                        "Only task-output inputs use dependency file references.",
                        json!({}),
                    ));
                }
                _ => {}
            }
            formal_rows.push(Value::Object(row));
        }
        result.insert(group.into(), Value::Array(formal_rows));
    }
    Ok(Value::Object(result))
}

pub fn build_semantic_patch(
    current: &Value,
    replacements: &Value,
    acceptance_ids: &[String],
    dependency_ids: &[String],
    dependency_files: &BTreeMap<String, BTreeMap<String, String>>,
) -> Result<Value, TaskIssue> {
    let mut allowed_acceptance = acceptance_ids.to_vec();
    allowed_acceptance.extend(
        current["acceptance_criteria"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|row| row["id"].as_str().map(str::to_owned)),
    );
    let groups = [
        "inputs",
        "decisions",
        "files",
        "risks",
        "steps",
        "commands",
        "operations",
        "validations",
    ];
    let prefixes = [
        "INPUT",
        "TASK-DECISION",
        "FILE",
        "RISK",
        "STEP",
        "CMD",
        "OP",
        "VAL",
    ];
    fields(replacements, "semantic_after", &[], &groups)?;
    let mut aliases: BTreeMap<&str, BTreeMap<String, String>> = BTreeMap::new();
    for (group, prefix) in groups.into_iter().zip(prefixes) {
        let old_rows = current[group].as_array().cloned().unwrap_or_default();
        let mut known = old_rows
            .iter()
            .enumerate()
            .filter_map(|(position, row)| {
                row["id"]
                    .as_str()
                    .map(|id| (format!("existing-{}", position + 1), id.to_owned()))
            })
            .collect::<BTreeMap<_, _>>();
        if let Some(rows) = replacements.get(group) {
            let rows = rows.as_array().ok_or_else(|| {
                issue(
                    "invalid_semantic_items",
                    "Nested semantic edits require arrays.",
                    json!({}),
                )
            })?;
            known.clear();
            let mut used = HashSet::new();
            let mut next = old_rows
                .iter()
                .filter_map(|row| row["id"].as_str())
                .filter_map(|id| id.rsplit_once('-')?.1.parse::<usize>().ok())
                .max()
                .unwrap_or(0);
            for (position, row) in rows.iter().enumerate() {
                let (required, optional) = group_fields(group);
                let mut optional = optional.to_vec();
                optional.push("existing_position");
                fields(
                    row,
                    &format!("semantic_after.{group}[{}]", position + 1),
                    required,
                    &optional,
                )?;
                let local = key(&row["key"], &format!("{group}[{}].key", position + 1))?;
                if known.contains_key(&local) {
                    return Err(issue(
                        "duplicate_semantic_key",
                        "Local semantic keys must be unique.",
                        json!({}),
                    ));
                }
                let id = if let Some(old_position) = row.get("existing_position") {
                    let position = old_position.as_u64().ok_or_else(|| {
                        issue(
                            "invalid_semantic_position",
                            "An existing position must identify one retained record.",
                            json!({}),
                        )
                    })? as usize;
                    if position == 0 || position > old_rows.len() || !used.insert(position) {
                        return Err(issue(
                            "invalid_semantic_position",
                            "An existing position must identify one retained record.",
                            json!({}),
                        ));
                    }
                    let id = old_rows[position - 1]["id"]
                        .as_str()
                        .ok_or_else(|| {
                            issue(
                                "invalid_semantic_items",
                                "Nested source records need identifiable formal IDs.",
                                json!({}),
                            )
                        })?
                        .to_owned();
                    let alias = format!("existing-{position}");
                    if alias == local || known.insert(alias, id.clone()).is_some() {
                        return Err(issue(
                            "duplicate_semantic_key",
                            "Local semantic keys must be unique.",
                            json!({}),
                        ));
                    }
                    id
                } else {
                    next += 1;
                    format!("{prefix}-{next:03}")
                };
                known.insert(local, id);
            }
        }
        aliases.insert(group, known);
    }
    let lookup = |group: &str, value: &Value| -> Result<String, TaskIssue> {
        let local = key(value, group)?;
        aliases
            .get(group)
            .and_then(|rows| rows.get(&local))
            .cloned()
            .ok_or_else(|| {
                issue(
                    "invalid_semantic_reference",
                    "A local semantic reference is unknown.",
                    json!({"group":group,"key":local}),
                )
            })
    };
    let mut built = serde_json::Map::new();
    for group in groups {
        let Some(rows) = replacements.get(group).and_then(Value::as_array) else {
            continue;
        };
        let mut formal_rows = Vec::new();
        for row in rows {
            let mut formal = row.as_object().expect("validated row").clone();
            formal.remove("existing_position");
            let local = formal.remove("key").expect("validated key");
            formal.insert("id".into(), json!(lookup(group, &local)?));
            match group {
                "steps" => {
                    let refs = formal["references"]
                        .as_array()
                        .filter(|rows| !rows.is_empty())
                        .ok_or_else(|| {
                            issue(
                                "invalid_semantic_reference",
                                "Steps require semantic references.",
                                json!({}),
                            )
                        })?;
                    let resolved = refs
                        .iter()
                        .map(|reference| {
                            fields(reference, "step.reference", &["kind", "key"], &[])?;
                            lookup(reference["kind"].as_str().unwrap_or(""), &reference["key"])
                                .map(|id| json!(id))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    formal.insert("references".into(), Value::Array(resolved));
                }
                "validations" => {
                    if let Some(commands) = formal.remove("command_keys") {
                        let ids = commands
                            .as_array()
                            .ok_or_else(|| {
                                issue(
                                    "invalid_semantic_reference",
                                    "Automated validations require command keys.",
                                    json!({}),
                                )
                            })?
                            .iter()
                            .map(|local| lookup("commands", local).map(|id| json!(id)))
                            .collect::<Result<Vec<_>, _>>()?;
                        formal.insert("command_ids".into(), Value::Array(ids));
                    }
                    if let Some(ids) = formal.get("acceptance_ids") {
                        validate_known_acceptance(ids, Some(&allowed_acceptance))?;
                    }
                }
                "operations" => {
                    let key = formal
                        .remove("validation_key")
                        .expect("validated validation key");
                    formal.insert("validation_id".into(), json!(lookup("validations", &key)?));
                    if let Some(key) = formal.remove("command_key") {
                        formal.insert("command_id".into(), json!(lookup("commands", &key)?));
                    }
                }
                "inputs" if formal["kind"] == "task_output" => {
                    let position = formal
                        .remove("dependency_position")
                        .and_then(|value| value.as_u64())
                        .filter(|position| *position > 0)
                        .ok_or_else(|| {
                            issue(
                                "invalid_semantic_position",
                                "Positions must uniquely identify existing one-based items.",
                                json!({}),
                            )
                        })? as usize;
                    let dependency = dependency_ids.get(position - 1).ok_or_else(|| {
                        issue(
                            "invalid_semantic_position",
                            "Positions must uniquely identify existing one-based items.",
                            json!({}),
                        )
                    })?;
                    let file_key = key(
                        &formal.remove("file_key").unwrap_or(Value::Null),
                        "file_key",
                    )?;
                    let file = dependency_files
                        .get(dependency)
                        .and_then(|files| files.get(&file_key))
                        .ok_or_else(|| {
                            issue(
                                "invalid_semantic_reference",
                                "Dependency file key is unknown.",
                                json!({}),
                            )
                        })?;
                    formal.insert("source".into(), json!(format!("{dependency}/{file}")));
                }
                "inputs"
                    if formal.contains_key("dependency_position")
                        || formal.contains_key("file_key") =>
                {
                    return Err(issue(
                        "invalid_semantic_reference",
                        "Only task-output inputs use dependency file references.",
                        json!({}),
                    ));
                }
                _ => {}
            }
            formal_rows.push(Value::Object(formal));
        }
        built.insert(group.into(), Value::Array(formal_rows));
    }
    Ok(Value::Object(built))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_patch_preserves_retained_ids_and_allocates_new_ids() {
        let current = json!({"commands":[{"id":"CMD-001","mode":"argv","argv":["tool","old"]}],
            "validations":[{"id":"VAL-001","kind":"manual","confirmer":"user","criteria":"Old"}]});
        let replacements = json!({"commands":[
            {"key":"old","existing_position":1,"mode":"argv","argv":["tool","old"]},
            {"key":"rerun","mode":"argv","argv":["tool","new"]}],
            "validations":[{"key":"verify","kind":"automated","command_keys":["rerun"],
                "pass_condition":"Exit zero","acceptance_ids":["ACCEPTANCE-001"]}]});
        let built = build_semantic_patch(
            &current,
            &replacements,
            &["ACCEPTANCE-001".into()],
            &[],
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(built["commands"][0]["id"], "CMD-001");
        assert_eq!(built["commands"][1]["id"], "CMD-002");
        assert_eq!(built["validations"][0]["id"], "VAL-002");
        assert_eq!(built["validations"][0]["command_ids"], json!(["CMD-002"]));
        assert_eq!(
            built["validations"][0]["acceptance_ids"],
            json!(["ACCEPTANCE-001"])
        );
    }

    #[test]
    fn semantic_keys_reject_unknown_references() {
        let candidate = json!({"acceptance_ids":["ACCEPTANCE-001"],"acceptance_criteria":[{"id":"TASK-001-ACCEPTANCE-001","criterion":"Verified."}],"steps": [{"key": "start", "action": "Do it.", "references": [{"kind": "validations", "key": "verify"}]}], "validations": [{"key": "verify", "kind": "manual", "confirmer": "user", "criteria": "Done."}]});
        validate_semantic_candidate(&candidate, true).unwrap();
        let mut missing_steps = candidate.clone();
        missing_steps.as_object_mut().unwrap().remove("steps");
        let error = validate_semantic_candidate(&missing_steps, true).unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(error.details["location"], "task_candidate");
        assert_eq!(error.details["missing"], json!(["steps"]));
        let mut forged_goal = candidate.clone();
        forged_goal["goal"] = json!("Different goal");
        assert_eq!(
            validate_semantic_candidate(&forged_goal, true)
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
        let mut formal_id = candidate.clone();
        formal_id["validations"][0]["id"] = json!("VAL-001");
        assert_eq!(
            validate_semantic_candidate(&formal_id, true)
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
        let mut formal_key = candidate.clone();
        formal_key["validations"][0]["key"] = json!("VAL-001");
        assert_eq!(
            validate_semantic_candidate(&formal_key, true)
                .unwrap_err()
                .reason_code,
            "invalid_semantic_key"
        );
        let mut duplicate = candidate.clone();
        let row = duplicate["validations"][0].clone();
        duplicate["validations"].as_array_mut().unwrap().push(row);
        assert_eq!(
            validate_semantic_candidate(&duplicate, true)
                .unwrap_err()
                .reason_code,
            "duplicate_semantic_key"
        );
        let mut stale = candidate;
        stale["steps"][0]["references"][0]["key"] = json!("missing");
        assert_eq!(
            validate_semantic_candidate(&stale, true)
                .unwrap_err()
                .reason_code,
            "invalid_semantic_reference"
        );
    }

    #[test]
    fn candidate_build_resolves_local_keys_and_dependency_files() {
        let candidate = json!({
            "acceptance_ids":["ACCEPTANCE-001"],"acceptance_criteria":[{"id":"TASK-002-ACCEPTANCE-001","criterion":"Verified."}],
            "inputs":[{"key":"source","kind":"task_output","precondition":"Ready.","dependency_position":1,"file_key":"result"}],
            "commands":[{"key":"check","mode":"argv","argv":["cargo","test"]}],
            "validations":[{"key":"verify","kind":"automated","command_keys":["check"],"pass_condition":"Exit zero.","acceptance_ids":["ACCEPTANCE-001"]}],
            "steps":[{"key":"start","action":"Run check.","references":[{"kind":"inputs","key":"source"},{"kind":"validations","key":"verify"},{"kind":"commands","key":"check"}]}]
        });
        let files = BTreeMap::from([(
            "TASK-001".into(),
            BTreeMap::from([("result".into(), "FILE-002".into())]),
        )]);
        let formal = build_semantic_candidate(
            &candidate,
            &["ACCEPTANCE-001".into()],
            &["TASK-001".into()],
            &files,
        )
        .unwrap();
        assert_eq!(formal["inputs"][0]["source"], "TASK-001/FILE-002");
        assert_eq!(
            formal["steps"][0]["references"],
            json!(["INPUT-001", "VAL-001", "CMD-001"])
        );
        assert_eq!(
            formal["validations"][0]["acceptance_ids"],
            json!(["ACCEPTANCE-001"])
        );
        // Draft storage uses sorted keys; Python's generic renderer keeps insertion order.
        assert_eq!(
            crate::canonical::sha256_hex(&crate::canonical::canonical_json(&formal).unwrap()),
            "0c5d6dd7197651b224ab6b1c363e659c95d24ca8151020a9117af0af06631ee9"
        );
        let local = json!({
            "acceptance_ids":[],"acceptance_criteria":[{"id":"TASK-001-ACCEPTANCE-001","criterion":"Verified."}],
            "files":[{"key":"report","action":"create","path":"report.txt"}],
            "validations":[{"key":"result","kind":"manual","confirmer":"User","criteria":"Result is observable."}],
            "steps":[{"key":"review","action":"Review result.","references":[
                {"kind":"validations","key":"result"},{"kind":"files","key":"report"}]}]
        });
        let built = build_semantic_candidate(&local, &[], &[], &BTreeMap::new()).unwrap();
        assert_eq!(built["files"][0]["id"], "FILE-001");
        assert_eq!(built["steps"][0]["id"], "STEP-001");
        assert_eq!(
            built["steps"][0]["references"],
            json!(["VAL-001", "FILE-001"])
        );
    }
}
