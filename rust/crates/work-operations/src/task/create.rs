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

fn append_target(bytes: &mut Vec<u8>, path: &str, raw: &[u8]) {
    bytes.extend_from_slice(format!("{}:", path.len()).as_bytes());
    bytes.extend_from_slice(path.as_bytes());
    bytes.extend_from_slice(format!("{}:", raw.len()).as_bytes());
    bytes.extend_from_slice(raw);
}

pub fn approval_with_execution(
    collection: &[u8],
    execution_path: &str,
    execution_raw: &[u8],
) -> Vec<u8> {
    let mut bytes = collection.to_vec();
    append_target(&mut bytes, execution_path, execution_raw);
    bytes
}

pub fn prepare_collection(
    projection: &Value,
    index_path: &str,
    source_root: &str,
) -> Result<PreparedCollection, TaskIssue> {
    if projection["schema"] != "work-task-collection-projection" {
        return Err(issue(
            "task_create_schema",
            "TASK create input must be a complete work-task-collection-projection contract.",
        ));
    }
    if projection["artifacts"]["task"] != index_path
        || projection["artifacts"]["source"] != source_root
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
        item.insert("schema".into(), json!("work-task-item"));
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
    index.insert("schema".into(), json!("work-task-index"));
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
        append_target(&mut approval_bytes, &path, raw);
    }
    Ok(PreparedCollection {
        index,
        index_raw,
        items,
        approval_bytes,
    })
}

#[cfg(test)]
mod approval_tests {
    use super::*;
    #[test]
    fn approval_binds_exact_execution_path_and_bytes_to_collection() {
        let actual = approval_with_execution(b"collection", "execution/index.json", b"pending\n");
        assert_eq!(actual, b"collection20:execution/index.json8:pending\n");
        assert_ne!(
            actual,
            approval_with_execution(b"collection", "other/index.json", b"pending\n")
        );
        assert_ne!(
            actual,
            approval_with_execution(b"collection", "execution/index.json", b"pending \n")
        );
        assert_ne!(
            actual,
            approval_with_execution(b"other", "execution/index.json", b"pending\n")
        );
    }
}
