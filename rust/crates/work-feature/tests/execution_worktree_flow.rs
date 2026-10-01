//! Execute worktree response preserves formal source fingerprints.

use serde_json::{Value, json};
use work_feature::error::WorkError;
use work_feature::execution::{ExecutionWorktreeRepository, inspect_worktree};
use work_operations::derivation::fingerprint::raw as sha256_hex;

struct FormalSources;

impl ExecutionWorktreeRepository for FormalSources {
    fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        Ok(if relative_path.ends_with("/index.json") {
            b"index\n".to_vec()
        } else {
            b"item\n".to_vec()
        })
    }

    fn git_status(&self) -> Result<Vec<Value>, WorkError> {
        Ok(vec![])
    }
}

#[test]
fn inspection_forwards_current_instruction_and_collection_fingerprints() {
    let preflight = json!({
        "requirement_id":"example", "task_spec_id":"TASK-SPEC-001", "task_id":"TASK-001",
        "skill_id":null, "task_collection_sha256":"a".repeat(64),
        "task_index_sha256":sha256_hex(b"index\n"),
        "task_item_sha256":sha256_hex(b"item\n"),
        "task_instructions_sha256":"b".repeat(64),
        "execute_instructions_sha256":"c".repeat(64),
        "task_status":"pending", "task_path":"outputs/work/tasks/example/index.json",
        "index_sha256":"d".repeat(64), "execution_dir":"outputs/work/executions/example",
        "dependencies":[],
    });
    let validation = json!({"task_collection_sha256":preflight["task_collection_sha256"],
        "task_index_sha256":preflight["task_index_sha256"],
        "task_item_sha256":{"TASK-001":preflight["task_item_sha256"]}});
    let collection = json!({"tasks":[{"id":"TASK-001","files":[]}]});
    let result = inspect_worktree(&FormalSources, &preflight, &validation, &collection).unwrap();
    for key in [
        "task_collection_sha256",
        "task_index_sha256",
        "task_item_sha256",
        "task_instructions_sha256",
        "execute_instructions_sha256",
    ] {
        assert_eq!(result[key], preflight[key]);
    }
    for legacy in ["task_sha256", "task_rules_sha256", "execute_rules_sha256"] {
        assert!(result.get(legacy).is_none());
    }
    let mut changed = preflight;
    changed["task_index_sha256"] = json!("f".repeat(64));
    assert_eq!(
        inspect_worktree(&FormalSources, &changed, &validation, &collection)
            .unwrap_err()
            .reason_code,
        "execute_worktree_task_changed"
    );
}
