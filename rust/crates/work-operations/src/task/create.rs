//! Exact formal TASK index, item, and approval byte construction.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::task::TaskIssue;
use crate::task::item::validate_task_item;
use crate::task::ordering::{TaskDocumentKind, render_task};

pub struct PreparedCollection {
    pub index: Value,
    pub index_raw: Vec<u8>,
    pub items: BTreeMap<String, Vec<u8>>,
    pub approval_bytes: Vec<u8>,
}

fn issue(reason_code: &'static str, message: &'static str) -> TaskIssue {
    TaskIssue {
        reason_code,
        message,
        details: json!({}),
    }
}

pub fn prepare_collection(
    projection: &Value,
    index_path: &str,
    plan_path: &str,
) -> Result<PreparedCollection, TaskIssue> {
    if projection["schema"] != "work-task-collection-projection/v1" {
        return Err(issue(
            "task_create_schema",
            "TASK create input must be a complete work-task-collection-projection/v1 contract.",
        ));
    }
    if projection["artifacts"]["task"] != index_path || projection["artifacts"]["plan"] != plan_path
    {
        return Err(issue(
            "task_create_path_mismatch",
            "The explicit create paths must match the TASK artifact paths.",
        ));
    }
    let tasks = projection["tasks"]
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| {
            issue(
                "invalid_item_array",
                "TASK create requires non-empty tasks.",
            )
        })?;
    let mut items = BTreeMap::new();
    let mut references = Vec::new();
    for task in tasks {
        let task_id = task["id"].as_str().ok_or_else(|| {
            issue(
                "invalid_task_item",
                "Each TASK create row must be an object with an ID.",
            )
        })?;
        let Some(task_object) = task.as_object() else {
            return Err(issue(
                "invalid_task_item",
                "Each TASK create row must be an object with an ID.",
            ));
        };
        let mut item = task_object.clone();
        item.insert("schema".into(), json!("work-task-item/v1"));
        let item = Value::Object(item);
        let raw = render_task(&item, TaskDocumentKind::Item).map_err(|_| {
            issue(
                "invalid_contract_value",
                "The TASK item cannot be rendered.",
            )
        })?;
        let validated = validate_task_item(&item, &raw, task_id)?;
        let digest = validated["task_item_sha256"].as_str().unwrap_or("");
        references.push(json!({"id":task_id,"path":format!("tasks/{task_id}.json"),
            "canonical_sha256":digest}));
        items.insert(task_id.to_owned(), raw);
    }
    let mut index = projection
        .as_object()
        .ok_or_else(|| issue("task_create_schema", "TASK create input must be an object."))?
        .clone();
    index.insert("schema".into(), json!("work-task-index/v1"));
    index.insert("tasks".into(), Value::Array(references));
    let index = Value::Object(index);
    let index_raw = render_task(&index, TaskDocumentKind::Index).map_err(|_| {
        issue(
            "invalid_contract_value",
            "The TASK index cannot be rendered.",
        )
    })?;
    let mut approval_bytes = b"WORK-TASK-COLLECTION-CREATE-V1\n".to_vec();
    for (path, raw) in std::iter::once((index_path.to_owned(), &index_raw)).chain(items.iter().map(
        |(task_id, raw)| {
            (
                format!(
                    "{}/tasks/{task_id}.json",
                    index_path.rsplit_once('/').map_or("", |(parent, _)| parent)
                ),
                raw,
            )
        },
    )) {
        approval_bytes.extend_from_slice(format!("{}:", path.len()).as_bytes());
        approval_bytes.extend_from_slice(path.as_bytes());
        approval_bytes.extend_from_slice(format!("{}:", raw.len()).as_bytes());
        approval_bytes.extend_from_slice(raw);
    }
    Ok(PreparedCollection {
        index,
        index_raw,
        items,
        approval_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::sha256_hex;

    #[test]
    fn legacy_python_fixture_round_trips_formal_index_and_item() {
        let index_raw = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/fixtures/instruction-migration/outputs/work/tasks/example/index.json"
        ));
        let item_raw = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/fixtures/instruction-migration/outputs/work/tasks/example/tasks/TASK-001.json"
        ));
        let mut index: Value = serde_json::from_slice(index_raw).unwrap();
        let mut item: Value = serde_json::from_slice(item_raw).unwrap();
        item.as_object_mut().unwrap().remove("schema");
        index["schema"] = json!("work-task-collection-projection/v1");
        index["tasks"] = json!([item]);
        let result = prepare_collection(
            &index,
            "outputs/work/tasks/example/index.json",
            "outputs/work/plans/example.json",
        )
        .unwrap();
        assert_eq!(result.index_raw, index_raw);
        assert_eq!(result.items["TASK-001"], item_raw);
        assert_eq!(
            sha256_hex(&result.approval_bytes),
            "03b228812fc0479b9da340b1d3ed2a3ec14b936b5e03e0693f437f8befb8219f"
        );
    }
}
