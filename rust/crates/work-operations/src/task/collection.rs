//! Pure TASK collection projection and fingerprint.

use serde_json::json;
use work_model::schema::PublicSchema;
use work_model::task::index::{TaskIndex, TaskItemReference};
use work_model::task::item::TaskItem;
use work_model::task::projection::{
    TaskCollectionProjection, TaskProjectionChange, TaskProjectionChangeEdit, TaskProjectionItem,
};
use work_model::task::{TaskCollectionFingerprint, TaskFingerprintItem};

use crate::canonical::sha256_hex;
use crate::task::TaskIssue;

pub(crate) fn collection_fingerprint_sha256(
    index_sha256: &str,
    references: &[TaskItemReference],
) -> String {
    let fingerprint = TaskCollectionFingerprint {
        schema: PublicSchema::WorkTaskCollectionFingerprintV1,
        task_index_sha256: index_sha256.into(),
        items: references
            .iter()
            .map(|reference| TaskFingerprintItem {
                id: reference.id.clone(),
                task_item_sha256: reference.canonical_sha256.clone(),
            })
            .collect(),
    };
    let mut bytes = serde_json::to_vec_pretty(&fingerprint).expect("fingerprint serializes");
    bytes.push(b'\n');
    sha256_hex(&bytes)
}

pub fn require_item_set(
    items: &std::collections::BTreeMap<String, Vec<u8>>,
    expected_ids: &[String],
) -> Result<(), TaskIssue> {
    let expected: std::collections::BTreeSet<_> = expected_ids.iter().cloned().collect();
    let actual: std::collections::BTreeSet<_> = items.keys().cloned().collect();
    if actual != expected {
        return Err(TaskIssue {
            reason_code: "task_collection_item_set_mismatch",
            message: "The supplied TASK item set does not match the formal index.",
            details: json!({"missing": expected.difference(&actual).collect::<Vec<_>>(), "unknown": actual.difference(&expected).collect::<Vec<_>>()}),
        });
    }
    Ok(())
}

pub fn semantic_projection(
    index: TaskIndex,
    items: Vec<TaskItem>,
    task_path: &str,
    source_plan_sha256: &str,
) -> TaskCollectionProjection {
    let mut artifacts = index.artifacts;
    artifacts.task = task_path.into();
    let mut source_plan = index.source_plan;
    source_plan.canonical_sha256 = source_plan_sha256.into();
    TaskCollectionProjection {
        schema: PublicSchema::WorkTaskCollectionProjectionV1,
        requirement_id: index.requirement_id,
        spec_id: index.spec_id,
        status: index.status,
        title: index.title,
        summary: index.summary,
        artifacts,
        source_plan,
        instruction_selection: index.instruction_selection,
        execution_defaults: index.execution_defaults,
        decisions: index.decisions,
        tasks: items
            .into_iter()
            .map(|item| TaskProjectionItem {
                id: item.id,
                title: item.title,
                skill_id: item.skill_id,
                instruction_selection: item.instruction_selection,
                traceability: item.traceability,
                goal: item.goal,
                dependencies: item.dependencies,
                inputs: item.inputs,
                decisions: item.decisions,
                files: item.files,
                risks: item.risks,
                steps: item.steps,
                commands: item.commands,
                operations: item.operations,
                validations: item.validations,
            })
            .collect(),
        changes: index.changes.map(|changes| {
            changes
                .into_iter()
                .map(|change| TaskProjectionChange {
                    id: change.id,
                    spec_id: change.spec_id,
                    date: change.date,
                    reason: change.reason,
                    affected_ids: change.affected_ids,
                    edits: change
                        .edits
                        .into_iter()
                        .map(|edit| TaskProjectionChangeEdit {
                            operation: edit.operation,
                            path: edit.path,
                            before: edit.before,
                            after: edit.after,
                        })
                        .collect(),
                    plan_change_ids: change.plan_change_ids,
                })
                .collect()
        }),
        readiness: index.readiness,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_sensitive_to_reference_order() {
        let first = TaskItemReference {
            id: "TASK-001".into(),
            path: "tasks/TASK-001.json".into(),
            canonical_sha256: "a".repeat(64),
        };
        let second = TaskItemReference {
            id: "TASK-002".into(),
            path: "tasks/TASK-002.json".into(),
            canonical_sha256: "b".repeat(64),
        };
        let index_sha256 = "c".repeat(64);
        let item_sha256 = "a".repeat(64);
        let fingerprint = TaskCollectionFingerprint {
            schema: PublicSchema::WorkTaskCollectionFingerprintV1,
            task_index_sha256: index_sha256.clone(),
            items: vec![TaskFingerprintItem {
                id: "TASK-001".into(),
                task_item_sha256: item_sha256.clone(),
            }],
        };
        assert_eq!(
            serde_json::to_value(&fingerprint).unwrap(),
            json!({"schema":"work-task-collection-fingerprint/v1",
                "task_index_sha256":"c".repeat(64),
                "items":[{"id":"TASK-001","task_item_sha256":"a".repeat(64)}]})
        );
        assert_eq!(
            collection_fingerprint_sha256(&"c".repeat(64), &[first.clone(), second.clone()]),
            "87e0e7695438c2e22ca4ad7e1e23ece55be20dcc995bc76174ed4d479c7f0efc"
        );
        assert_ne!(
            collection_fingerprint_sha256(&"c".repeat(64), &[first.clone(), second.clone()]),
            collection_fingerprint_sha256(&"c".repeat(64), &[second, first])
        );
        assert!(
            require_item_set(&std::collections::BTreeMap::new(), &["TASK-001".into()]).is_err()
        );
    }
}
