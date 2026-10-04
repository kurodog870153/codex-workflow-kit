//! Verified latest Attempt loading for workflow decisions.

use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use serde_json::{Value, json};
use work_feature::artifact_paths::default_artifact_paths;
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;
use work_feature::skill::SkillRoot;
use work_feature::task::load_collection_with_file_state;
use work_feature::workflow::WorkflowSnapshot;
#[cfg(test)]
use work_feature::workflow::{execution_state, pre_execution_state};
use work_operations::canonical::parse_json_contract;
use work_operations::execution::attempt::validate_attempt_bytes;
use work_operations::execution::index::validate_execution_index;
use work_operations::identifiers::RequirementId;

use crate::files::{LocalFiles, resolve_project_path};
use crate::hierarchy_catalog::LocalHierarchyCatalog;
#[cfg(test)]
use crate::routing_sources::RoutingSourceSession;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::source_snapshot_storage::LocalSourceSnapshotStorage;
use crate::specification::storage::storage_path;
use crate::task::draft_storage::LocalTaskDraftStorage;
use crate::task::storage::LocalTaskStorage;
use work_feature::ports::SourceSnapshotReader;
use work_feature::specification::reconciliation_input::validate_ledger_entries;

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

pub struct WorkflowStateRequest<'a> {
    pub project_root: &'a Path,
    pub skill_root: &'a Path,
    pub skill_configs: &'a [SkillRootConfig],
    pub requirement_id: &'a str,
    pub task_path: Option<&'a str>,
}

pub fn load_workflow_snapshot(
    request: &WorkflowStateRequest<'_>,
) -> Result<WorkflowSnapshot, WorkError> {
    let id: RequirementId = request.requirement_id.parse().map_err(
        |issue: work_operations::identifiers::IdentifierIssue| {
            WorkError::new(
                ExitCode::Contract,
                issue.reason_code(),
                "The requirement ID is invalid.",
                json!({}),
            )
        },
    )?;
    let mut artifacts = serde_json::to_value(default_artifact_paths(&id)).expect("paths serialize");
    let selected_task = request
        .task_path
        .unwrap_or_else(|| artifacts["task"].as_str().unwrap())
        .to_owned();
    let (_, task_file) = resolve_project_path(request.project_root, &selected_task)?;
    if request.task_path.is_some() && !task_file.exists() {
        return Err(fail(
            "workflow_task_missing",
            "The explicitly selected formal TASK does not exist.",
        ));
    }
    if task_file.exists() {
        let contract = parse_json_contract(&LocalFiles.read_raw(&task_file)?).map_err(|_| {
            fail(
                "invalid_json_contract",
                "The selected TASK JSON is invalid.",
            )
        })?;
        if contract["artifacts"]["task"] != selected_task {
            return Err(fail(
                "task_artifact_path_mismatch",
                "The selected TASK path differs from its declared path.",
            ));
        }
        artifacts = contract["artifacts"].clone();
    }
    let task_path = selected_task.as_str();
    let execution_dir = artifacts["execution"].as_str().ok_or_else(|| {
        fail(
            "workflow_artifact_path_missing",
            "The execution artifact path is missing.",
        )
    })?;
    let (_, execution_path) = resolve_project_path(request.project_root, execution_dir)?;
    let index_path = execution_path.join("index.json");
    let index = if index_path.is_file() {
        let raw = LocalFiles.read_raw(&index_path)?;
        let value = parse_json_contract(&raw).map_err(|_| {
            fail(
                "invalid_json_contract",
                "The execution index JSON is invalid.",
            )
        })?;
        validate_execution_index(&value, &raw).map_err(|issue| {
            WorkError::new(
                ExitCode::Contract,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
        Some(value)
    } else {
        None
    };
    let instructions = LocalHierarchyCatalog {
        skill_root: request.skill_root.to_path_buf(),
    };
    let skills = LocalSkillCatalog {
        roots: request.skill_configs.to_vec(),
    };
    let skill_roots: Vec<SkillRoot> = request
        .skill_configs
        .iter()
        .map(|config| SkillRoot {
            scope: config.scope.clone(),
            locator: config.locator.clone(),
        })
        .collect();
    let task = if task_file.exists() {
        Some(load_collection_with_file_state(
            &instructions,
            &skills,
            &crate::artifact_paths::LocalArtifactPaths {
                project_root: request.project_root.to_path_buf(),
            },
            &LocalTaskStorage {
                project_root: request.project_root.to_path_buf(),
            },
            &skill_roots,
            task_path,
            true,
        )?)
    } else {
        None
    };
    if task
        .as_ref()
        .is_some_and(|value| value["requirement_id"] != request.requirement_id)
    {
        return Err(fail(
            "workflow_task_requirement_mismatch",
            "The selected TASK does not match the requested requirement.",
        ));
    }
    let draft = if task.is_none() {
        Some(
            LocalTaskDraftStorage {
                project_root: request.project_root.to_path_buf(),
            }
            .status(request.requirement_id, None)?,
        )
    } else {
        None
    };
    let source = if let Some(task) = &task {
        Some(task["collection_contract"]["source"].clone())
    } else if draft
        .as_ref()
        .is_some_and(|value| value["status"] != "not_initialized")
    {
        let saved_index = LocalTaskDraftStorage {
            project_root: request.project_root.to_path_buf(),
        }
        .read_planning_index(request.requirement_id)?;
        let saved = &saved_index["source"];
        work_feature::task::source::validate_context(
            &LocalSourceSnapshotStorage {
                project_root: request.project_root.to_path_buf(),
            },
            &instructions,
            &skills,
            &crate::artifact_paths::LocalArtifactPaths {
                project_root: request.project_root.to_path_buf(),
            },
            &skill_roots,
            request.requirement_id,
            saved,
        )?;
        artifacts = saved["artifacts"].clone();
        Some(saved.clone())
    } else {
        let relative = artifacts["source"].as_str().ok_or_else(|| {
            fail(
                "workflow_artifact_path_missing",
                "The Source artifact path is missing.",
            )
        })?;
        let (_, directory) = resolve_project_path(request.project_root, relative)?;
        let mut snapshots = Vec::new();
        if directory.exists() {
            for entry in std::fs::read_dir(&directory).map_err(|_| {
                fail(
                    "workflow_source_read_failed",
                    "Source versions could not be inspected.",
                )
            })? {
                let entry = entry.map_err(|_| {
                    fail(
                        "workflow_source_read_failed",
                        "Source versions could not be inspected.",
                    )
                })?;
                let name = entry.file_name();
                let name = name
                    .to_str()
                    .ok_or_else(|| fail("invalid_source_namespace", "A Source name is invalid."))?;
                if name.starts_with(".capture-") {
                    return Err(fail(
                        "source_snapshot_incomplete",
                        "A Source capture requires recovery.",
                    ));
                }
                let source_id = name
                    .parse()
                    .map_err(|_| fail("invalid_source_namespace", "A Source ID is invalid."))?;
                let snapshot = LocalSourceSnapshotStorage {
                    project_root: request.project_root.to_path_buf(),
                }
                .read_snapshot_at(&id, &source_id, relative)?;
                snapshots
                    .push(serde_json::to_value(snapshot.manifest).expect("Snapshot serializes"));
            }
        }
        snapshots
            .sort_by(|left, right| left["source_id"].as_str().cmp(&right["source_id"].as_str()));
        if snapshots.is_empty() {
            None
        } else {
            Some(json!({"snapshots":snapshots}))
        }
    };
    let latest_attempts = if let Some(ref index) = index {
        load_latest_attempts(request.project_root, &artifacts, index)?
    } else {
        json!({})
    };
    Ok(WorkflowSnapshot {
        artifacts,
        source,
        draft,
        task,
        index,
        latest_attempts,
    })
}

#[cfg(test)]
fn workflow_state(request: &WorkflowStateRequest<'_>) -> Result<Value, WorkError> {
    let snapshot = load_workflow_snapshot(request)?;
    let WorkflowSnapshot {
        artifacts,
        source,
        draft,
        task,
        index,
        latest_attempts,
    } = snapshot;
    let mut routing = RoutingSourceSession::new(PathBuf::from(request.skill_root));
    if let Some(state) = pre_execution_state(
        &mut routing,
        request.requirement_id,
        &artifacts,
        source.as_ref(),
        draft.as_ref(),
        task.as_ref(),
        index.is_some(),
    )? {
        return Ok(state);
    }
    execution_state(
        &mut routing,
        request.requirement_id,
        &artifacts,
        index
            .as_ref()
            .expect("pre-execution state handled missing index"),
        &latest_attempts,
    )
}

pub fn load_latest_attempts(
    root: &Path,
    artifacts: &Value,
    index: &Value,
) -> Result<Value, WorkError> {
    let execution = artifacts["execution"].as_str().ok_or_else(|| {
        fail(
            "workflow_artifact_path_missing",
            "The execution artifact path is missing.",
        )
    })?;
    let mut attempts = serde_json::Map::new();
    for row in index["tasks"]
        .as_array()
        .ok_or_else(|| fail("execution_index_tasks", "Execution TASK rows are missing."))?
    {
        let Some(attempt_id) = row["latest_attempt"].as_str() else {
            continue;
        };
        let task_id = row["id"]
            .as_str()
            .ok_or_else(|| fail("execution_index_tasks", "An execution TASK ID is missing."))?;
        let relative = format!("{execution}/{task_id}/{attempt_id}/attempt.json");
        let path = storage_path(root, &relative)?;
        if !path.is_file() {
            continue;
        }
        let raw = LocalFiles.read_raw(&path)?;
        let mut attempt = parse_json_contract(&raw).map_err(|_| {
            fail(
                "invalid_json_contract",
                "The latest Attempt is invalid JSON.",
            )
        })?;
        validate_attempt_bytes(&attempt, &raw).map_err(|issue| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
        if attempt["task_id"] != task_id || attempt["attempt_id"] != attempt_id {
            return Err(fail(
                "attempt_path_identity",
                "The latest Attempt path does not match its identity.",
            ));
        }
        let ledger_path = storage_path(
            root,
            &format!("{execution}/{task_id}/{attempt_id}/reconciliation.json"),
        )?;
        if ledger_path.is_file() {
            let ledger =
                parse_json_contract(&LocalFiles.read_raw(&ledger_path)?).map_err(|_| {
                    fail(
                        "reconciliation_ledger",
                        "The reconciliation ledger is invalid.",
                    )
                })?;
            if ledger["schema"] != "work-spec-reconciliation-ledger"
                || ledger["attempt_path"] != relative
            {
                return Err(fail(
                    "reconciliation_ledger",
                    "The reconciliation ledger does not reference its Attempt.",
                ));
            }
            validate_ledger_entries(&ledger)?;
            let ids = ledger["entries"]
                .as_array()
                .ok_or_else(|| {
                    fail(
                        "reconciliation_ledger",
                        "The reconciliation ledger entries are invalid.",
                    )
                })?
                .iter()
                .map(|row| row["deviation_id"].clone())
                .collect::<Vec<_>>();
            attempt["_reconciliation_resolved_ids"] = json!(ids);
        }
        attempts.insert(task_id.to_owned(), attempt);
    }
    Ok(Value::Object(attempts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "work-workflow-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn workflow_state_reads_missing_and_source_only_project_without_writes() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let root = temporary_root("state");
        let skill_root = repo.join("../skills/work");
        let request = WorkflowStateRequest {
            project_root: &root,
            skill_root: &skill_root,
            skill_configs: &[],
            requirement_id: "example",
            task_path: None,
        };
        let missing = workflow_state(&request).unwrap();
        assert_eq!(missing["status"], "source_required");
        assert_eq!(missing["next_action"], "capture_source");
        assert_eq!(missing["requires_user_confirmation"], true);
        assert_eq!(missing["request_contract_id"], Value::Null);
        assert_eq!(missing["command"], "source capture");
        assert_eq!(
            missing["arguments"]["input_file"],
            "<capture-metadata-file>"
        );
        assert_eq!(
            missing["semantic_input_contract"],
            missing["request_contract_id"]
        );
        assert_eq!(missing["routing_status"], "VALID");
        assert_eq!(
            missing["source_order"],
            missing["required_instruction_sources"]
        );
        assert_eq!(
            missing["selection_manifest"]["selection_sha256"],
            missing["selection_sha256"]
        );
        assert!(work_operations::protocol::valid_sha256(
            missing["selection_sha256"].as_str().unwrap()
        ));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);

        let fixture = repo.join("crates/work-infrastructure/fixtures/task-diagnostics");
        crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        let source_only = workflow_state(&request).unwrap();
        assert_eq!(source_only["status"], "task_not_initialized");
        assert_eq!(source_only["next_action"], "confirm_task_list");
        assert_eq!(
            source_only["details"]["source"]["snapshots"][0]["source_id"],
            "SRC-001"
        );
        assert_eq!(
            source_only["selection_sha256"],
            source_only["selection_manifest"]["selection_sha256"]
        );
        assert_ne!(missing["selection_sha256"], source_only["selection_sha256"]);
        assert!(!root.join("outputs/work/plans").exists());
    }

    #[test]
    fn selected_task_path_must_match_declared_task_path() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let root = temporary_root("selected-task");
        let relative = "custom/tasks/example/index.json";
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_vec(&json!({"artifacts":{"task":"other/tasks/example/index.json"}}))
                .unwrap(),
        )
        .unwrap();
        let result = workflow_state(&WorkflowStateRequest {
            project_root: &root,
            skill_root: &repo.join("../skills/work"),
            skill_configs: &[],
            requirement_id: "example",
            task_path: Some(relative),
        });
        assert_eq!(
            result.unwrap_err().reason_code,
            "task_artifact_path_mismatch"
        );
    }

    #[test]
    fn selected_task_drives_custom_execution_paths() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let root = temporary_root("custom-artifacts");
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-diagnostics");
        let relative = "custom/tasks/example/index.json";
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap().join("tasks")).unwrap();
        let mut index: Value = serde_json::from_slice(
            &fs::read(fixture.join("outputs/work/tasks/example/index.json")).unwrap(),
        )
        .unwrap();
        index["artifacts"]["task"] = json!(relative);
        index["artifacts"]["execution"] = json!("custom/executions/example");
        fs::write(
            &path,
            crate::fixture_support::render_task_index(&index).unwrap(),
        )
        .unwrap();
        fs::copy(
            fixture.join("outputs/work/tasks/example/tasks/TASK-001.json"),
            path.parent().unwrap().join("tasks/TASK-001.json"),
        )
        .unwrap();
        crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        fs::write(root.join("src.txt"), b"original\n").unwrap();
        for decoy in [
            "outputs/work/tasks/example/index.json",
            "outputs/work/executions/example/index.json",
        ] {
            let path = root.join(decoy);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"invalid json").unwrap();
        }
        let request = WorkflowStateRequest {
            project_root: &root,
            skill_root: &repo.join("../skills/work"),
            skill_configs: &[],
            requirement_id: "example",
            task_path: Some(relative),
        };
        let state = workflow_state(&request).unwrap();
        assert_eq!(state["status"], "execution_recovery_required");
        assert_eq!(state["artifacts"]["execution"], "custom/executions/example");
        let custom = root.join("custom/executions/example/index.json");
        fs::create_dir_all(custom.parent().unwrap()).unwrap();
        fs::write(custom, b"invalid json").unwrap();
        assert_eq!(
            workflow_state(&request).unwrap_err().reason_code,
            "invalid_json_contract"
        );
    }

    #[test]
    fn workflow_state_reads_formal_execution_and_reconciliation() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-reconciliation/real-flow/with-migration");
        let root = temporary_root("complete");
        for relative in [
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
            "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
            "outputs/work/executions/example/TASK-001/ATTEMPT-001/reconciliation.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
            crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        }
        fs::write(root.join("src.txt"), b"original\n").unwrap();
        let skill_root = repo.join("../skills/work");
        let request = WorkflowStateRequest {
            project_root: &root,
            skill_root: &skill_root,
            skill_configs: &[],
            requirement_id: "example",
            task_path: None,
        };
        let state = workflow_state(&request).unwrap();
        assert_eq!(state["status"], "execution_completed");
        assert_eq!(state["next_action"], "review_completion");
        assert_eq!(state["details"], json!({"overall_status":"completed"}));
        assert_eq!(
            state["selection_sha256"],
            state["selection_manifest"]["selection_sha256"]
        );
    }

    #[test]
    fn workflow_state_reads_pending_execution_and_rejects_wrong_selected_task() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-diagnostics");
        let root = temporary_root("pending");
        for relative in [
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
            crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        }
        fs::write(root.join("src.txt"), b"original\n").unwrap();
        let skill_root = repo.join("../skills/work");
        let request = WorkflowStateRequest {
            project_root: &root,
            skill_root: &skill_root,
            skill_configs: &[],
            requirement_id: "example",
            task_path: None,
        };
        let state = workflow_state(&request).unwrap();
        assert_eq!(state["status"], "execution_pending");
        assert_eq!(state["next_action"], "select_task_for_execution");
        assert_eq!(state["details"], json!({"overall_status":"pending"}));
        assert_eq!(
            state["selection_sha256"],
            state["selection_manifest"]["selection_sha256"]
        );
        let wrong_plan = root.join("other/example.json");
        fs::create_dir_all(wrong_plan.parent().unwrap()).unwrap();
        fs::copy(
            fixture.join("outputs/work/tasks/example/index.json"),
            &wrong_plan,
        )
        .unwrap();
        assert_eq!(
            workflow_state(&WorkflowStateRequest {
                task_path: Some("other/example.json"),
                ..request
            })
            .unwrap_err()
            .reason_code,
            "task_artifact_path_mismatch"
        );
    }

    #[test]
    fn latest_attempt_and_ledger_match_current_contract_workflow_view() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-reconciliation/real-flow");
        let execution = "outputs/work/executions/example";
        let index: Value =
            serde_json::from_slice(&fs::read(fixture.join(execution).join("index.json")).unwrap())
                .unwrap();
        let artifacts = json!({"execution":execution});
        let actual = load_latest_attempts(&fixture, &artifacts, &index).unwrap();
        let expected: Value = serde_json::from_slice(
            &fs::read(
                repo.join("crates/work-infrastructure/fixtures/workflow-execution/latest-attempts-expected.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn malformed_ledger_is_rejected_before_workflow_selection() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-reconciliation/real-flow");
        let execution = "outputs/work/executions/example";
        let index: Value =
            serde_json::from_slice(&fs::read(fixture.join(execution).join("index.json")).unwrap())
                .unwrap();
        let root = std::env::temp_dir().join(format!(
            "work-ledger-invalid-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let attempt_dir = format!("{execution}/TASK-001/ATTEMPT-001");
        for name in ["attempt.json", "reconciliation.json"] {
            let relative = format!("{attempt_dir}/{name}");
            let destination = root.join(&relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
            crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        }
        let ledger_path = root.join(attempt_dir).join("reconciliation.json");
        let mut ledger: Value = serde_json::from_slice(&fs::read(&ledger_path).unwrap()).unwrap();
        ledger["entries"][0]["reconciliation_fingerprint"] = json!("invalid");
        fs::write(&ledger_path, serde_json::to_vec(&ledger).unwrap()).unwrap();
        let error =
            load_latest_attempts(&root, &json!({"execution":execution}), &index).unwrap_err();
        assert_eq!(error.exit_code, ExitCode::Contract);
        assert_eq!(error.reason_code, "invalid_contract_value");
        assert_eq!(
            error.details["location"],
            "entries[0].reconciliation_fingerprint"
        );
    }
}
