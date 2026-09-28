//! Verified latest Attempt loading for workflow decisions.

use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::plan::{PlanPathRepository, validate_plan_file};
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
use crate::plan_storage::LocalPlanStorage;
#[cfg(test)]
use crate::routing_sources::RoutingSourceSession;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::specification::storage::storage_path;
use crate::task::draft_storage::LocalTaskDraftStorage;
use crate::task::storage::LocalTaskStorage;
use work_feature::plan::default_artifact_paths;
use work_feature::specification::reconciliation_input::validate_ledger_entries;

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

pub struct WorkflowStateRequest<'a> {
    pub project_root: &'a Path,
    pub skill_root: &'a Path,
    pub skill_configs: &'a [SkillRootConfig],
    pub requirement_id: &'a str,
    pub plan_path: Option<&'a str>,
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
    let paths = LocalPlanStorage {
        project_root: request.project_root.to_path_buf(),
    };
    let mut artifacts = serde_json::Map::new();
    for (field, relative) in default_artifact_paths(&id) {
        resolve_project_path(request.project_root, &relative)?;
        artifacts.insert(field.into(), json!(relative));
    }
    if let Some(selected) = request.plan_path {
        let (normalized, path) = resolve_project_path(request.project_root, selected)?;
        artifacts.insert("plan".into(), json!(normalized));
        if path.is_file() {
            let raw = LocalFiles.read_raw(&path)?;
            let contract = parse_json_contract(&raw).map_err(|_| {
                fail(
                    "invalid_json_contract",
                    "The selected Plan JSON is invalid.",
                )
            })?;
            let selected_artifacts = &contract["artifacts"];
            paths.validate_paths(
                &id,
                selected_artifacts,
                &normalized,
                selected_artifacts["task"]
                    .as_str()
                    .is_some_and(|path| path.ends_with("/index.json")),
            )?;
            artifacts = selected_artifacts
                .as_object()
                .expect("validated Plan artifact paths")
                .clone();
        }
    }
    let artifacts = Value::Object(artifacts);
    let plan_path = artifacts["plan"].as_str().expect("artifact Plan path");
    let task_path = artifacts["task"].as_str().expect("artifact TASK path");
    let execution_dir = artifacts["execution"]
        .as_str()
        .expect("artifact execution path");
    let (_, plan_file) = resolve_project_path(request.project_root, plan_path)?;
    let (_, task_file) = resolve_project_path(request.project_root, task_path)?;
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
    let plan = if plan_file.exists() {
        let validated =
            validate_plan_file(&instructions, &skills, &paths, &skill_roots, plan_path)?;
        if validated["requirement_id"] != request.requirement_id {
            return Err(fail(
                "workflow_plan_requirement_mismatch",
                "The selected Plan does not match the requested requirement.",
            ));
        }
        Some(validated)
    } else {
        None
    };
    let task = if plan.is_some() && task_file.exists() {
        Some(load_collection_with_file_state(
            &instructions,
            &skills,
            &paths,
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
    let draft = if plan.is_some() && task.is_none() {
        Some(
            LocalTaskDraftStorage {
                project_root: request.project_root.to_path_buf(),
            }
            .status(request.requirement_id, None)?,
        )
    } else {
        None
    };
    let latest_attempts = if let Some(ref index) = index {
        load_latest_attempts(request.project_root, &artifacts, index)?
    } else {
        json!({})
    };
    Ok(WorkflowSnapshot {
        artifacts,
        plan,
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
        plan,
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
        plan.as_ref(),
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
            if ledger["schema"] != "work-spec-reconciliation-ledger/v1"
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
    fn workflow_state_reads_missing_and_plan_only_project_without_writes() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let root = temporary_root("state");
        let skill_root = repo.join("../skills/work");
        let request = WorkflowStateRequest {
            project_root: &root,
            skill_root: &skill_root,
            skill_configs: &[],
            requirement_id: "example",
            plan_path: None,
        };
        let missing = workflow_state(&request).unwrap();
        assert_eq!(missing["status"], "plan_required");
        assert_eq!(missing["next_action"], "prepare_plan");
        assert_eq!(missing["requires_user_confirmation"], true);
        assert_eq!(
            missing["request_contract_id"],
            "work-plan-semantic-request/v1"
        );
        assert_eq!(missing["command"], "plan semantic-prepare");
        assert_eq!(missing["arguments"]["input_file"], "<semantic-input-file>");
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
        assert_eq!(
            missing["selection_sha256"],
            "3509cb2eac17b1286558bdbab8e0e44f7775a2d0c2ea9b671eff2b716236eaf1"
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);

        let path = root.join("outputs/work/plans/example.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::copy(
            repo.join("crates/work-infrastructure/fixtures/task-assembly/plan.json"),
            &path,
        )
        .unwrap();
        let plan_only = workflow_state(&request).unwrap();
        assert_eq!(plan_only["status"], "task_not_initialized");
        assert_eq!(plan_only["next_action"], "confirm_task_list");
        assert_eq!(
            plan_only["details"]["plan_sha256"],
            "ff2063a3da86ffda334b109b7e6afecd4a1ea5dfd172e977ff0742154f5c1c2d"
        );
        assert_eq!(
            plan_only["selection_sha256"],
            "553ec62d303a63a1956ea7f6d0cd30b128d2a892c85d65684863cfb480d17620"
        );
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn selected_plan_path_must_match_declared_plan_path() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let root = temporary_root("selected-plan");
        let relative = "custom/plans/example.json";
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema":"work-plan/v1",
                "requirement_id":"example",
                "artifacts":{
                    "plan":"other/plans/example.json",
                    "task":"custom/tasks/example/index.json",
                    "execution":"custom/executions/example"
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let result = workflow_state(&WorkflowStateRequest {
            project_root: &root,
            skill_root: &repo.join("../skills/work"),
            skill_configs: &[],
            requirement_id: "example",
            plan_path: Some(relative),
        });
        assert_eq!(
            result.unwrap_err().reason_code,
            "plan_artifact_path_mismatch"
        );
    }

    #[test]
    fn selected_plan_drives_custom_task_and_execution_paths() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let root = temporary_root("custom-artifacts");
        let relative = "custom/plans/example.json";
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut plan: Value = serde_json::from_slice(
            &fs::read(repo.join("crates/work-infrastructure/fixtures/task-assembly/plan.json"))
                .unwrap(),
        )
        .unwrap();
        plan["artifacts"] = json!({
            "plan": relative,
            "task": "custom/tasks/example/index.json",
            "execution": "custom/executions/example"
        });
        fs::write(
            &path,
            work_operations::plan::render_plan_value(&plan).unwrap(),
        )
        .unwrap();
        let default_task = root.join("outputs/work/tasks/example/index.json");
        let default_index = root.join("outputs/work/executions/example/index.json");
        for decoy in [&default_task, &default_index] {
            fs::create_dir_all(decoy.parent().unwrap()).unwrap();
            fs::write(decoy, b"invalid json").unwrap();
        }
        let request = WorkflowStateRequest {
            project_root: &root,
            skill_root: &repo.join("../skills/work"),
            skill_configs: &[],
            requirement_id: "example",
            plan_path: Some(relative),
        };
        let state = workflow_state(&request).unwrap();
        assert_eq!(state["status"], "task_not_initialized");

        let custom_index = root.join("custom/executions/example/index.json");
        fs::create_dir_all(custom_index.parent().unwrap()).unwrap();
        fs::write(custom_index, b"invalid json").unwrap();
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
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
            "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
            "outputs/work/executions/example/TASK-001/ATTEMPT-001/reconciliation.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        fs::write(root.join("src.txt"), b"original\n").unwrap();
        let skill_root = repo.join("../skills/work");
        let request = WorkflowStateRequest {
            project_root: &root,
            skill_root: &skill_root,
            skill_configs: &[],
            requirement_id: "example",
            plan_path: None,
        };
        let state = workflow_state(&request).unwrap();
        assert_eq!(state["status"], "execution_completed");
        assert_eq!(state["next_action"], "review_completion");
        assert_eq!(state["details"], json!({"overall_status":"completed"}));
        assert_eq!(
            state["selection_sha256"],
            "727816a5b49c2757e2ab602bb94ee484ada530c4bc72e54432005cf9c036d7f3"
        );
    }

    #[test]
    fn workflow_state_reads_pending_execution_and_rejects_wrong_selected_plan() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-diagnostics");
        let root = temporary_root("pending");
        for relative in [
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        fs::write(root.join("src.txt"), b"original\n").unwrap();
        let skill_root = repo.join("../skills/work");
        let request = WorkflowStateRequest {
            project_root: &root,
            skill_root: &skill_root,
            skill_configs: &[],
            requirement_id: "example",
            plan_path: None,
        };
        let state = workflow_state(&request).unwrap();
        assert_eq!(state["status"], "execution_pending");
        assert_eq!(state["next_action"], "select_task_for_execution");
        assert_eq!(state["details"], json!({"overall_status":"pending"}));
        assert_eq!(
            state["selection_sha256"],
            "38297cc5bd30a4f2d137b1ea1725c423385bad8d30f9ee9b4d361447e01f5413"
        );
        let wrong_plan = root.join("other/example.json");
        fs::create_dir_all(wrong_plan.parent().unwrap()).unwrap();
        fs::copy(fixture.join("outputs/work/plans/example.json"), &wrong_plan).unwrap();
        assert_eq!(
            workflow_state(&WorkflowStateRequest {
                plan_path: Some("other/example.json"),
                ..request
            })
            .unwrap_err()
            .reason_code,
            "plan_artifact_path_mismatch"
        );
    }

    #[test]
    fn latest_attempt_and_ledger_match_python_workflow_view() {
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
