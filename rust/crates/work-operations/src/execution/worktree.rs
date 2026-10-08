//! Pure Git worktree snapshot and formal TASK path classification.

use serde_json::{Value, json};

use crate::canonical::canonical_sha256;

fn within(path: &str, directory: &str) -> bool {
    path == directory || path.starts_with(&format!("{directory}/"))
}

/// Runtime bookkeeping is excluded only when every path in the Git record is managed runtime.
/// A rename crossing the boundary remains part of the reviewed business worktree.
pub fn runtime_worktree_records(records: &[Value]) -> Vec<Value> {
    records
        .iter()
        .filter(|record| {
            let path = record["path"].as_str().unwrap_or("");
            !std::iter::once(path)
                .chain(record["original_path"].as_str())
                .all(|path| within(path, "outputs/work/runtime"))
        })
        .cloned()
        .collect()
}

fn paths(task: &Value) -> Vec<&str> {
    task["files"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|file| {
            ["path", "source", "destination"]
                .into_iter()
                .filter_map(|field| file[field].as_str())
        })
        .collect()
}

pub fn inspect_records(
    records: &[Value],
    execution_dir: &str,
    task_id: &str,
    target: &Value,
    dependencies: &[Value],
) -> Value {
    let target_paths = paths(target);
    let dependency_paths: Vec<_> = dependencies
        .iter()
        .map(|task| (task["id"].as_str().unwrap_or(""), paths(task)))
        .collect();
    let mut included = Vec::new();
    let mut changes = Vec::new();
    let mut excluded = 0;
    let mut counts = json!({"staged":0,"unstaged":0,"untracked":0,
        "target_task":0,"completed_dependency":0,"unrelated":0});
    for record in records {
        let path = record["path"].as_str().unwrap_or("");
        let original = record["original_path"].as_str();
        let record_paths: Vec<_> = std::iter::once(path).chain(original).collect();
        if record_paths.iter().all(|path| within(path, execution_dir)) {
            excluded += 1;
            continue;
        }
        included.push(record.clone());
        let matches: Vec<_> = dependency_paths
            .iter()
            .filter(|(_, declared)| record_paths.iter().any(|path| declared.contains(path)))
            .map(|(id, _)| *id)
            .collect();
        let (classification, matched): (&str, Vec<&str>) =
            if record_paths.iter().any(|path| target_paths.contains(path)) {
                ("target_task", vec![task_id])
            } else if !matches.is_empty() {
                ("completed_dependency", matches)
            } else {
                ("unrelated", vec![])
            };
        counts[classification] = json!(counts[classification].as_u64().unwrap_or(0) + 1);
        let index_status = record["index_status"].as_str().unwrap_or(" ");
        let worktree_status = record["worktree_status"].as_str().unwrap_or(" ");
        if index_status != " " && index_status != "?" {
            counts["staged"] = json!(counts["staged"].as_u64().unwrap_or(0) + 1);
        }
        if worktree_status != " " && worktree_status != "?" {
            counts["unstaged"] = json!(counts["unstaged"].as_u64().unwrap_or(0) + 1);
        }
        if index_status == "?" && worktree_status == "?" {
            counts["untracked"] = json!(counts["untracked"].as_u64().unwrap_or(0) + 1);
        }
        let mut change = record.clone();
        change["path_classification"] = json!(classification);
        if !matched.is_empty() {
            change["matched_task_ids"] = json!(matched);
        }
        changes.push(change);
    }
    let snapshot = work_model::execution::response::verified::<
        work_model::execution::response::WorktreeSnapshot,
    >(json!({"schema":"work-execute-worktree-snapshot","records":included}));
    let encoded = serde_json::to_vec(&snapshot).expect("JSON snapshot serializes");
    let snapshot_sha256 = canonical_sha256(&encoded).expect("JSON snapshot is UTF-8");
    json!({"snapshot_sha256":snapshot_sha256,"excluded_execution_change_count":excluded,
        "review_status":if changes.is_empty() { "clean" } else { "required" },
        "counts":counts,"changes":changes})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_records_preserve_cross_boundary_renames_and_similar_paths() {
        let mut records = vec![
            json!({"path":"outputs/work/runtime/locks/example/execution.lock"}),
            json!({"path":"outputs/work/runtime/staging/new","original_path":"outputs/work/runtime/staging/old"}),
            json!({"path":"src.txt","original_path":"outputs/work/runtime/staging/old"}),
            json!({"path":"outputs/work/runtime/staging/new","original_path":"src.txt"}),
            json!({"path":"outputs/work/runtime-foreign/entry"}),
            json!({"path":""}),
        ];
        for record in &mut records {
            record["index_status"] = json!("?");
            record["worktree_status"] = json!("?");
        }
        let filtered = runtime_worktree_records(&records);
        assert_eq!(filtered, records[2..]);
        let before = inspect_records(&filtered, "execution", "TASK-001", &json!({}), &[]);
        let mut locked = filtered.clone();
        locked.push(records[0].clone());
        assert_eq!(
            inspect_records(
                &runtime_worktree_records(&locked),
                "execution",
                "TASK-001",
                &json!({}),
                &[]
            ),
            before
        );
        assert_ne!(
            inspect_records(&locked, "execution", "TASK-001", &json!({}), &[])["snapshot_sha256"],
            before["snapshot_sha256"]
        );
    }

    #[test]
    fn snapshot_excludes_only_changes_fully_inside_execution_dir() {
        let records = vec![
            json!({"index_status":"?","worktree_status":"?","path":"execution/index.json"}),
            json!({"index_status":"M","worktree_status":" ","path":"src/new.rs","original_path":"src/old.rs"}),
            json!({"index_status":" ","worktree_status":"M","path":"src/dep.rs"}),
            json!({"index_status":"?","worktree_status":"?","path":"other.txt"}),
        ];
        let target = json!({"files":[{"path":"src/old.rs"}]});
        let dependency = json!({"id":"TASK-002","files":[{"path":"src/dep.rs"}]});
        let result = inspect_records(&records, "execution", "TASK-003", &target, &[dependency]);
        assert_eq!(result["excluded_execution_change_count"], 1);
        assert_eq!(
            result["counts"],
            json!({"staged":1,"unstaged":1,"untracked":1,
            "target_task":1,"completed_dependency":1,"unrelated":1})
        );
        assert_eq!(result["changes"][0]["path_classification"], "target_task");
        assert_eq!(
            result["changes"][1]["matched_task_ids"],
            json!(["TASK-002"])
        );
        assert_eq!(result["review_status"], "required");
        assert_eq!(
            result["snapshot_sha256"],
            "726b1e33fa9ee9e186cace4a70bfe50b0de8e575b9948c43753725f4245afbd8"
        );
        let after_execution_change = vec![
            records[0].clone(),
            records[1].clone(),
            json!({"index_status":"?","worktree_status":"?","path":"execution/next.json"}),
            records[2].clone(),
            records[3].clone(),
        ];
        let same = inspect_records(
            &after_execution_change,
            "execution",
            "TASK-003",
            &target,
            &[json!({"id":"TASK-002","files":[{"path":"src/dep.rs"}]})],
        );
        assert_eq!(result["snapshot_sha256"], same["snapshot_sha256"]);
    }
}
