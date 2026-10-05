//! Read-only Handoff construction from validated Task and Execution artifacts.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::handoff::{
    ClosedReturnInput, build_closed_return, build_preflight_return, build_task_to_execute,
    validate_preflight_return_request, validate_task_to_execute_request,
    verify_return_against_expected, verify_task_to_execute,
};
use work_feature::handoff::{HandoffCommandRepository, HandoffStorageAction};
use work_feature::instruction::select;
use work_feature::ports::ArtifactStore;
use work_feature::skill::SkillRoot;
use work_feature::task::{TaskCollectionRepository, load_collection};
use work_operations::canonical::parse_json_contract;
use work_operations::execution::attempt::validate_attempt_bytes;
use work_operations::execution::index::validate_execution_index;

use crate::files::{LocalFiles, resolve_project_path};
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::specification::storage::require_no_spec_update;
use crate::task::storage::LocalTaskStorage;

type TaskSourceSnapshot = BTreeMap<String, (PathBuf, Vec<u8>)>;

#[derive(Debug, Clone)]
pub struct LocalHandoffStorage {
    pub project_root: PathBuf,
    pub skill_root: PathBuf,
    pub skill_configs: Vec<SkillRootConfig>,
}

impl HandoffCommandRepository for LocalHandoffStorage {
    fn execute(
        &self,
        action: HandoffStorageAction<'_>,
        request: &Value,
    ) -> Result<Value, WorkError> {
        match action {
            HandoffStorageAction::TaskToExecute {
                verify,
                task_path,
                task_id,
            } => {
                if verify {
                    self.verify_task_to_execute(task_path, task_id, request)
                } else {
                    self.build_task_to_execute(task_path, task_id, request)
                }
            }
            HandoffStorageAction::ExecuteReturn {
                verify,
                preflight,
                direction,
                task_path,
                task_id,
                attempt_id,
            } => {
                if verify {
                    if preflight {
                        self.verify_preflight_return(direction, task_path, task_id, request)
                    } else {
                        self.verify_closed_return(
                            direction,
                            task_path,
                            task_id,
                            attempt_id.expect("closed return requires attempt"),
                            request,
                        )
                    }
                } else if preflight {
                    self.build_preflight_return(direction, task_path, task_id, request)
                } else {
                    self.build_closed_return(
                        direction,
                        task_path,
                        task_id,
                        attempt_id.expect("closed return requires attempt"),
                        request,
                    )
                }
            }
        }
    }
}

impl LocalHandoffStorage {
    fn require_no_execution_transaction(&self, execution_dir: &str) -> Result<(), WorkError> {
        let (_, path) = resolve_project_path(&self.project_root, execution_dir)?;
        if path.is_dir() {
            let entries = fs::read_dir(path).map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "file_read_failed",
                    "The execution directory could not be read.",
                    json!({}),
                )
            })?;
            for entry in entries {
                let entry = entry.map_err(|_| {
                    WorkError::new(
                        ExitCode::IoFailure,
                        "file_read_failed",
                        "The execution directory could not be read.",
                        json!({}),
                    )
                })?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(".work-") && name.ends_with(".tmp") {
                    return Err(WorkError::new(
                        ExitCode::WorkflowState,
                        "handoff_execution_recovery_required",
                        "An execution transaction requires recovery.",
                        json!({}),
                    ));
                }
            }
        }
        Ok(())
    }

    fn require_unstarted_task_directory(
        &self,
        execution_dir: &str,
        task_id: &str,
    ) -> Result<(), WorkError> {
        let (_, directory) =
            resolve_project_path(&self.project_root, &format!("{execution_dir}/{task_id}"))?;
        if directory.exists()
            && (!directory.is_dir()
                || fs::read_dir(&directory)
                    .map_err(|_| {
                        WorkError::new(
                            ExitCode::IoFailure,
                            "file_read_failed",
                            "The TASK execution directory could not be read.",
                            json!({}),
                        )
                    })?
                    .next()
                    .is_some())
        {
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "handoff_attempt_artifacts_present",
                "The target TASK execution directory must be absent or empty.",
                json!({}),
            ));
        }
        Ok(())
    }

    fn task_snapshot(&self, task_path: &str) -> Result<(Value, TaskSourceSnapshot), WorkError> {
        let (normalized, _) = resolve_project_path(&self.project_root, task_path)?;
        let instructions = LocalHierarchyCatalog {
            skill_root: self.skill_root.clone(),
        };
        let skills = LocalSkillCatalog {
            roots: self.skill_configs.clone(),
        };
        let roots: Vec<SkillRoot> = self
            .skill_configs
            .iter()
            .map(|config| SkillRoot {
                scope: config.scope.clone(),
                locator: config.locator.clone(),
            })
            .collect();
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: self.project_root.clone(),
        };
        let repository = LocalTaskStorage {
            project_root: self.project_root.clone(),
        };
        let validation = load_collection(
            &instructions,
            &skills,
            &paths,
            &repository,
            &roots,
            &normalized,
        )?;
        let contract = &validation["collection_contract"];
        require_no_spec_update(
            &self.project_root,
            contract["artifacts"]["execution"]
                .as_str()
                .expect("validated execution path"),
            None,
        )?;
        let mut snapshot = BTreeMap::new();
        let index_raw = repository.read_task_file(&normalized)?;
        if work_operations::derivation::fingerprint::raw(&index_raw)
            != validation["task_index_sha256"]
        {
            return Err(Self::source_changed("task_path"));
        }
        snapshot.insert(
            normalized.clone(),
            (
                resolve_project_path(&self.project_root, &normalized)?
                    .1
                    .canonicalize()
                    .map_err(|_| Self::source_changed("task_path"))?,
                index_raw,
            ),
        );
        let directory = normalized.rsplit_once('/').map_or("", |(parent, _)| parent);
        for reference in contract["tasks"].as_array().expect("validated TASK items") {
            let id = reference["id"].as_str().expect("validated ID");
            let relative = format!("{directory}/tasks/{id}.json");
            let raw = repository.read_task_file(&relative)?;
            if work_operations::derivation::fingerprint::raw(&raw)
                != validation["task_item_sha256"][id]
            {
                return Err(Self::source_changed("task_path"));
            }
            snapshot.insert(
                relative.clone(),
                (
                    resolve_project_path(&self.project_root, &relative)?
                        .1
                        .canonicalize()
                        .map_err(|_| Self::source_changed("task_path"))?,
                    raw,
                ),
            );
        }
        for relative in work_feature::task::source::evidence_paths(contract)? {
            let (_, path) = resolve_project_path(&self.project_root, &relative)?;
            snapshot.insert(
                relative,
                (
                    path.canonicalize()
                        .map_err(|_| Self::source_changed("source"))?,
                    LocalFiles.read_raw(&path)?,
                ),
            );
        }
        let current = load_collection(
            &instructions,
            &skills,
            &paths,
            &repository,
            &roots,
            &normalized,
        )?;
        if current != validation {
            return Err(Self::source_changed("source"));
        }
        Ok((validation, snapshot))
    }

    fn source_changed(field: &str) -> WorkError {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            "handoff_source_changed",
            "A source artifact changed during handoff construction.",
            json!({"field":field}),
        )
    }

    fn recheck_file(path: &std::path::Path, expected: &[u8], field: &str) -> Result<(), WorkError> {
        if LocalFiles.read_raw(path).ok().as_deref() == Some(expected) {
            Ok(())
        } else {
            Err(Self::source_changed(field))
        }
    }

    fn recheck_task(
        &self,
        validation: &Value,
        snapshot: &TaskSourceSnapshot,
    ) -> Result<(), WorkError> {
        let repository = LocalTaskStorage {
            project_root: self.project_root.clone(),
        };
        for (path, (canonical, raw)) in snapshot {
            let current = resolve_project_path(&self.project_root, path)
                .ok()
                .and_then(|(_, path)| path.canonicalize().ok());
            if current.as_ref() != Some(canonical)
                || repository.read_task_file(path).ok().as_deref() != Some(raw.as_slice())
            {
                let field = if work_feature::task::source::evidence_paths(
                    &validation["collection_contract"],
                )?
                .iter()
                .any(|source| source == path)
                {
                    "source"
                } else {
                    "task_path"
                };
                return Err(Self::source_changed(field));
            }
        }
        require_no_spec_update(
            &self.project_root,
            validation["collection_contract"]["artifacts"]["execution"]
                .as_str()
                .expect("validated execution path"),
            None,
        )
    }

    pub fn build_task_to_execute(
        &self,
        task_path: &str,
        task_id: &str,
        request: &Value,
    ) -> Result<Value, WorkError> {
        validate_task_to_execute_request(request)?;
        let (validation, snapshot) = self.task_snapshot(task_path)?;
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: self.project_root.clone(),
        };
        let result = build_task_to_execute(&paths, &validation, task_id, request)?;
        self.recheck_task(&validation, &snapshot)?;
        Ok(result)
    }

    pub fn verify_task_to_execute(
        &self,
        task_path: &str,
        task_id: &str,
        incoming: &Value,
    ) -> Result<Value, WorkError> {
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: self.project_root.clone(),
        };
        work_operations::handoff::validate_handoff_structure(incoming)
            .map_err(|e| WorkError::new(ExitCode::Contract, e.reason_code, e.message, e.details))?;
        work_feature::handoff::validate_return_direction(incoming, "task_to_execute", false)?;
        work_feature::handoff::validate_task_handoff(&paths, incoming)?;
        let (validation, snapshot) = self.task_snapshot(task_path)?;
        let result = verify_task_to_execute(&paths, &validation, task_id, incoming)?;
        self.recheck_task(&validation, &snapshot)?;
        Ok(result)
    }

    pub fn build_preflight_return(
        &self,
        direction: &str,
        task_path: &str,
        task_id: &str,
        request: &Value,
    ) -> Result<Value, WorkError> {
        validate_preflight_return_request(request, direction)?;
        let (validation, snapshot) = self.task_snapshot(task_path)?;
        let execution = validation["collection_contract"]["artifacts"]["execution"]
            .as_str()
            .expect("validated execution path");
        self.require_no_execution_transaction(execution)?;
        let index_path = format!("{execution}/index.json");
        let (_, index_absolute) = resolve_project_path(&self.project_root, &index_path)?;
        let index_raw = LocalFiles.read_raw(&index_absolute)?;
        let index = parse_json_contract(&index_raw).map_err(|_| {
            WorkError::new(
                ExitCode::Contract,
                "invalid_json_contract",
                "The execution index JSON is invalid.",
                json!({}),
            )
        })?;
        validate_execution_index(&index, &index_raw).map_err(|issue| {
            WorkError::new(
                ExitCode::Contract,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: self.project_root.clone(),
        };
        let result =
            build_preflight_return(&paths, &validation, &index, direction, task_id, request)?;
        self.require_unstarted_task_directory(execution, task_id)?;
        self.recheck_task(&validation, &snapshot)?;
        Self::recheck_file(&index_absolute, &index_raw, "execution_index")?;
        self.require_no_execution_transaction(execution)?;
        self.require_unstarted_task_directory(execution, task_id)?;
        Ok(result)
    }

    pub fn verify_preflight_return(
        &self,
        direction: &str,
        task_path: &str,
        task_id: &str,
        incoming: &Value,
    ) -> Result<Value, WorkError> {
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: self.project_root.clone(),
        };
        work_feature::handoff::validate_task_handoff(&paths, incoming)?;
        work_feature::handoff::validate_return_direction(incoming, direction, true)?;
        let normalized_task = resolve_project_path(&self.project_root, task_path)?.0;
        if incoming["artifacts"]["task"] != normalized_task {
            return Err(Self::source_changed("task_path"));
        }
        let request = work_feature::handoff::return_request(incoming);
        let expected =
            self.build_preflight_return(direction, &normalized_task, task_id, &request)?;
        verify_return_against_expected(&paths, incoming, &expected, direction)
    }

    pub fn build_closed_return(
        &self,
        direction: &str,
        task_path: &str,
        task_id: &str,
        attempt_id: &str,
        request: &Value,
    ) -> Result<Value, WorkError> {
        validate_preflight_return_request(request, direction)?;
        let (validation, snapshot) = self.task_snapshot(task_path)?;
        let execution = validation["collection_contract"]["artifacts"]["execution"]
            .as_str()
            .expect("validated execution path");
        self.require_no_execution_transaction(execution)?;
        let index_path = format!("{execution}/index.json");
        let (_, index_absolute) = resolve_project_path(&self.project_root, &index_path)?;
        let index_raw = LocalFiles.read_raw(&index_absolute)?;
        let index = parse_json_contract(&index_raw).map_err(|_| {
            WorkError::new(
                ExitCode::Contract,
                "invalid_json_contract",
                "The execution index JSON is invalid.",
                json!({}),
            )
        })?;
        validate_execution_index(&index, &index_raw).map_err(|issue| {
            WorkError::new(
                ExitCode::Contract,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
        work_feature::handoff::validate_closed_return_index(
            &validation,
            &index,
            task_id,
            attempt_id,
        )?;
        let attempt_path = format!("{execution}/{task_id}/{attempt_id}/attempt.json");
        let (_, attempt_absolute) = resolve_project_path(&self.project_root, &attempt_path)?;
        let attempt_raw = LocalFiles.read_raw(&attempt_absolute)?;
        let attempt = parse_json_contract(&attempt_raw).map_err(|_| {
            WorkError::new(
                ExitCode::Contract,
                "invalid_json_contract",
                "The Attempt JSON is invalid.",
                json!({}),
            )
        })?;
        validate_attempt_bytes(&attempt, &attempt_raw).map_err(|issue| {
            WorkError::new(
                ExitCode::Contract,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
        work_feature::handoff::validate_closed_return_attempt(&attempt, task_id, attempt_id)?;
        let (selected_paths, references) =
            work_feature::handoff::closed_return_instruction_inputs(&validation, &attempt, task_id);
        let current = serde_json::to_value(select(
            &LocalHierarchyCatalog {
                skill_root: self.skill_root.clone(),
            },
            "execute",
            &selected_paths,
            &references,
        )?)
        .expect("instruction selection serializes");
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: self.project_root.clone(),
        };
        let result = build_closed_return(
            &paths,
            &ClosedReturnInput {
                validation: &validation,
                index: &index,
                attempt: &attempt,
                attempt_raw: &attempt_raw,
                current_execute_selection: &current,
                direction,
                task_id,
                attempt_id,
                request,
            },
        )?;
        self.recheck_task(&validation, &snapshot)?;
        for (field, path, expected) in [
            ("execution_index", &index_absolute, &index_raw),
            ("attempt_path", &attempt_absolute, &attempt_raw),
        ] {
            Self::recheck_file(path, expected, field)?;
        }
        self.require_no_execution_transaction(execution)?;
        Ok(result)
    }

    pub fn verify_closed_return(
        &self,
        direction: &str,
        task_path: &str,
        task_id: &str,
        attempt_id: &str,
        incoming: &Value,
    ) -> Result<Value, WorkError> {
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: self.project_root.clone(),
        };
        work_feature::handoff::validate_task_handoff(&paths, incoming)?;
        work_feature::handoff::validate_return_direction(incoming, direction, true)?;
        let normalized_task = resolve_project_path(&self.project_root, task_path)?.0;
        if incoming["artifacts"]["task"] != normalized_task {
            return Err(Self::source_changed("task_path"));
        }
        let request = work_feature::handoff::return_request(incoming);
        let expected =
            self.build_closed_return(direction, &normalized_task, task_id, attempt_id, &request)?;
        verify_return_against_expected(&paths, incoming, &expected, direction)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn task_handoff_ignores_legacy_plan_and_rejects_source_task_drift_and_aliases() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-diagnostics");
        let root = std::env::temp_dir().join(format!(
            "work-handoff-task-drift-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let plan_path = "outputs/work/plans/example.json";
        let task_path = "outputs/work/tasks/example/index.json";
        for relative in [task_path, "outputs/work/tasks/example/tasks/TASK-001.json"] {
            let target = root.join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), target).unwrap();
            crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        }
        let storage = LocalHandoffStorage {
            project_root: root.clone(),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![],
        };
        let request = json!({"summary":"Start execution."});
        let handoff = storage
            .build_task_to_execute(task_path, "TASK-001", &request)
            .unwrap();
        let (validation, snapshot) = storage.task_snapshot(task_path).unwrap();
        storage.recheck_task(&validation, &snapshot).unwrap();
        let index_raw = fs::read(root.join(task_path)).unwrap();
        let mut index = parse_json_contract(&index_raw).unwrap();
        index["summary"] = json!("Revised summary");
        fs::write(
            root.join(task_path),
            work_operations::task::ordering::render_task(
                &index,
                work_operations::task::ordering::TaskDocumentKind::Index,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            storage
                .recheck_task(&validation, &snapshot)
                .unwrap_err()
                .reason_code,
            "handoff_source_changed"
        );
        assert_eq!(
            storage
                .verify_task_to_execute(task_path, "TASK-001", &handoff)
                .unwrap_err()
                .reason_code,
            "handoff_source_mismatch"
        );
        fs::write(root.join(task_path), &index_raw).unwrap();
        fs::create_dir_all(root.join("outputs/work/plans")).unwrap();
        let plan_raw =
            serde_json::to_vec(&json!({"schema":"work-plan/v1","summary":"Original Plan"}))
                .unwrap();
        let plan = json!({"schema":"work-plan/v1","summary":"Revised Plan"});
        fs::write(root.join(plan_path), serde_json::to_vec(&plan).unwrap()).unwrap();
        storage.recheck_task(&validation, &snapshot).unwrap();
        storage
            .build_task_to_execute(task_path, "TASK-001", &request)
            .unwrap();
        storage
            .verify_task_to_execute(task_path, "TASK-001", &handoff)
            .unwrap();
        let source_path = root.join("outputs/work/sources/example/SRC-001/source.txt");
        let source_raw = fs::read(&source_path).unwrap();
        let mut changed = source_raw.clone();
        changed[0] ^= 1;
        fs::write(&source_path, &changed).unwrap();
        assert_eq!(
            storage
                .recheck_task(&validation, &snapshot)
                .unwrap_err()
                .reason_code,
            "handoff_source_changed"
        );
        assert_eq!(
            storage
                .build_task_to_execute(task_path, "TASK-001", &request)
                .unwrap_err()
                .reason_code,
            "source_hash_mismatch"
        );
        assert!(
            storage
                .verify_task_to_execute(task_path, "TASK-001", &handoff)
                .is_err()
        );
        fs::write(&source_path, &source_raw).unwrap();
        fs::write(root.join(plan_path), &plan_raw).unwrap();
        let mut draft = parse_json_contract(&index_raw).unwrap();
        draft["status"] = json!("draft");
        fs::write(
            root.join(task_path),
            work_operations::task::ordering::render_task(
                &draft,
                work_operations::task::ordering::TaskDocumentKind::Index,
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            storage
                .build_task_to_execute(task_path, "TASK-001", &request)
                .is_err()
        );
        fs::write(
            root.join(task_path),
            serde_json::to_vec(&parse_json_contract(&index_raw).unwrap()).unwrap(),
        )
        .unwrap();
        assert!(
            storage
                .build_task_to_execute(task_path, "TASK-001", &request)
                .is_err()
        );
        fs::write(root.join(task_path), &index_raw).unwrap();
        #[cfg(unix)]
        {
            let original_source = root.join("outputs/work/sources/example/SRC-001/source.txt");
            let redirected_source = root.join("alternate-source.txt");
            fs::rename(&original_source, &redirected_source).unwrap();
            std::os::unix::fs::symlink(&redirected_source, &original_source).unwrap();
            assert_eq!(
                storage
                    .recheck_task(&validation, &snapshot)
                    .unwrap_err()
                    .reason_code,
                "handoff_source_changed"
            );
            fs::remove_file(&original_source).unwrap();
            fs::rename(&redirected_source, &original_source).unwrap();
            let original = root.join(task_path);
            let redirected = root.join("alternate-task-index.json");
            fs::rename(&original, &redirected).unwrap();
            std::os::unix::fs::symlink(&redirected, &original).unwrap();
            assert_eq!(
                storage
                    .recheck_task(&validation, &snapshot)
                    .unwrap_err()
                    .reason_code,
                "handoff_source_changed"
            );
        }
    }

    #[test]
    fn closed_attempt_returns_match_current_contract_stopped_and_blocked() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let historical_skill =
            crate::fixture_support::historical_execute_skill_root(&repo.join("../skills/work"))
                .unwrap();
        let request = json!({"summary":"調整已確認範圍",
            "confirmed_approach":"保留既有介面", "requested_changes":["新增驗收條件"],
            "preserve":["既有功能"],"affected_ids":["ACCEPTANCE-001","TASK-001"],
            "validation_requirements":["重新確認驗收條件"],"reason":"Clarify specification"});
        let task_path = "outputs/work/tasks/example/index.json";
        for status in ["stopped", "blocked"] {
            let fixture = repo
                .join("crates/work-infrastructure/fixtures/handoff-closed")
                .join(status);
            let storage = LocalHandoffStorage {
                project_root: fixture.clone(),
                skill_root: historical_skill.clone(),
                skill_configs: vec![],
            };
            let current = LocalHandoffStorage {
                project_root: fixture.clone(),
                skill_root: repo.join("../skills/work"),
                skill_configs: vec![],
            };
            let attempt_path =
                fixture.join("outputs/work/executions/example/TASK-001/ATTEMPT-004/attempt.json");
            let historical_bytes = fs::read(&attempt_path).unwrap();
            assert_eq!(
                current
                    .build_closed_return(
                        "execute_to_task",
                        task_path,
                        "TASK-001",
                        "ATTEMPT-004",
                        &request
                    )
                    .unwrap_err()
                    .reason_code,
                "handoff_execute_instructions_changed"
            );
            assert_eq!(fs::read(&attempt_path).unwrap(), historical_bytes);
            if status == "stopped" {
                let semantic = json!({"summary":"Start selected TASK."});
                let first = storage
                    .build_task_to_execute(task_path, "TASK-001", &semantic)
                    .unwrap();
                let second = storage
                    .build_task_to_execute(task_path, "TASK-002", &semantic)
                    .unwrap();
                assert_ne!(
                    first["source"]["task_item_sha256"],
                    second["source"]["task_item_sha256"]
                );
                assert_eq!(
                    storage
                        .verify_task_to_execute(task_path, "TASK-002", &second)
                        .unwrap()["source"],
                    second["source"]
                );
                assert_eq!(
                    storage
                        .verify_task_to_execute(task_path, "TASK-001", &second)
                        .unwrap_err()
                        .reason_code,
                    "handoff_source_mismatch"
                );
                assert_eq!(
                    storage
                        .verify_task_to_execute(task_path, "TASK-999", &first)
                        .unwrap_err()
                        .reason_code,
                    "handoff_task_not_found"
                );
            }
            assert_eq!(
                storage
                    .build_preflight_return("execute_to_task", task_path, "TASK-001", &request)
                    .unwrap_err()
                    .reason_code,
                "handoff_task_already_started"
            );
            let legacy: Value = serde_json::from_slice(
                &fs::read(fixture.join("execute_to_plan-expected.json")).unwrap(),
            )
            .unwrap();
            let paths = crate::artifact_paths::LocalArtifactPaths {
                project_root: fixture.clone(),
            };
            assert_eq!(
                work_feature::handoff::validate_task_handoff(&paths, &legacy)
                    .unwrap_err()
                    .reason_code,
                "invalid_handoff_direction"
            );
            {
                let direction = "execute_to_task";
                let expected: Value = serde_json::from_slice(
                    &fs::read(fixture.join(format!("{direction}-expected.json"))).unwrap(),
                )
                .unwrap();
                let validation: Value = serde_json::from_slice(
                    &fs::read(fixture.join(format!("{direction}-validation.json"))).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    storage
                        .build_closed_return(
                            direction,
                            task_path,
                            "TASK-001",
                            "ATTEMPT-004",
                            &request
                        )
                        .unwrap(),
                    expected,
                    "{status} {direction}"
                );
                assert_eq!(
                    storage
                        .verify_closed_return(
                            direction,
                            task_path,
                            "TASK-001",
                            "ATTEMPT-004",
                            &expected
                        )
                        .unwrap(),
                    validation,
                    "{status} {direction}"
                );
                let mut missing_hash = expected.clone();
                missing_hash["source"]
                    .as_object_mut()
                    .unwrap()
                    .remove("attempt_sha256");
                assert!(
                    storage
                        .verify_closed_return(
                            direction,
                            task_path,
                            "TASK-001",
                            "ATTEMPT-004",
                            &missing_hash
                        )
                        .is_err()
                );
                assert_eq!(
                    storage
                        .build_closed_return(
                            direction,
                            task_path,
                            "TASK-001",
                            "ATTEMPT-001",
                            &request
                        )
                        .unwrap_err()
                        .reason_code,
                    "handoff_attempt_not_current"
                );
            }
        }
    }

    #[test]
    fn legacy_handoff_directions_are_rejected_before_source_lookup() {
        let storage = LocalHandoffStorage {
            project_root: std::env::temp_dir(),
            skill_root: PathBuf::new(),
            skill_configs: vec![],
        };
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: storage.project_root.clone(),
        };
        for direction in ["plan_to_task", "task_to_plan", "execute_to_plan"] {
            let legacy = json!({"schema":"work-handoff", "direction":direction});
            assert_eq!(
                work_feature::handoff::validate_task_handoff(&paths, &legacy)
                    .unwrap_err()
                    .reason_code,
                "invalid_handoff_direction"
            );
            assert!(serde_json::from_value::<work_model::handoff::FormalHandoff>(legacy).is_err());
            assert_eq!(
                storage
                    .build_preflight_return(direction, "missing/index.json", "TASK-001", &json!({}))
                    .unwrap_err()
                    .reason_code,
                "invalid_handoff_direction"
            );
        }
    }

    #[test]
    fn task_to_execute_matches_current_contract_reference_from_formal_collection() {
        let storage = LocalHandoffStorage {
            project_root: PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/work-infrastructure/fixtures/task-diagnostics"
            )),
            skill_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work")),
            skill_configs: vec![],
        };
        let task_path = "outputs/work/tasks/example/index.json";
        let request = json!({"summary":"Start execution."});
        let source = json!({"stage":"task","task_spec_id":"TASK-SPEC-001","task_id":"TASK-001",
            "task_collection_sha256":"0a89ab8518e109f74a3b515b15cd9879d53e3475c8232cced6a9d141a77d4716",
            "task_index_sha256":"5a6561e945ac751d2afe854caea5657dda95b6492416fe834f1f435e171deb1b",
            "task_item_sha256":"f1071ad659955de4e360da6b0e53b6f99870d9bede067bb26e0341749c594abb",
            "task_instructions_sha256":"bbb1285eeb68a50a9c91c7ce8ebadce33aba447624a33ab31f2708f751fee875",
            "skill_id":null,"skill_selection_sha256":"a09357ef9f22c43dca16b489da61b287838bf64963a5956bf52ae0327e04a959"});
        let expected = json!({"schema":"work-handoff","marker":"WORK-HANDOFF",
            "direction":"task_to_execute","requirement_id":"example",
            "artifacts":{"source":"outputs/work/sources/example","task":task_path,
                "execution":"outputs/work/executions/example"},
            "source":source,"target":{"stage":"execute"},"summary":"Start execution."});
        assert_eq!(
            storage
                .build_task_to_execute(task_path, "TASK-001", &request)
                .unwrap(),
            expected
        );
        assert_eq!(
            storage
                .verify_task_to_execute(task_path, "TASK-001", &expected)
                .unwrap(),
            json!({"schema":"work-handoff-source-validation","status":"valid",
                "direction":"task_to_execute","marker":"WORK-HANDOFF","requirement_id":"example",
                "source_stage":"task","target_stage":"execute","task_path":task_path,"source":source})
        );
        for field in [
            "task_spec_id",
            "skill_id",
            "task_collection_sha256",
            "task_index_sha256",
            "task_item_sha256",
            "task_instructions_sha256",
            "skill_selection_sha256",
        ] {
            let mut forged = expected.clone();
            forged["source"][field] = if field == "skill_id" {
                json!("unknown")
            } else if field == "task_spec_id" {
                json!("TASK-SPEC-999")
            } else {
                json!("0".repeat(64))
            };
            assert_eq!(
                storage
                    .verify_task_to_execute(task_path, "TASK-001", &forged)
                    .unwrap_err()
                    .reason_code,
                "handoff_source_mismatch",
                "{field}"
            );
        }
        for (field, path) in [
            ("source", "custom/sources/example"),
            ("task", "custom/tasks/example/index.json"),
            ("execution", "custom/executions/example"),
        ] {
            let mut forged = expected.clone();
            forged["artifacts"][field] = json!(path);
            assert_eq!(
                storage
                    .verify_task_to_execute(task_path, "TASK-001", &forged)
                    .unwrap_err()
                    .reason_code,
                "handoff_source_mismatch",
                "{field}"
            );
        }
        for field in ["source", "task_id", "artifacts", "skill_id", "affected_ids"] {
            let mut forged = request.clone();
            forged[field] = json!("override");
            assert_eq!(
                storage
                    .build_task_to_execute(task_path, "TASK-001", &forged)
                    .unwrap_err()
                    .reason_code,
                "invalid_object_fields",
                "{field}"
            );
        }
        assert_eq!(
            storage
                .verify_preflight_return("execute_to_task", task_path, "TASK-001", &expected)
                .unwrap_err()
                .reason_code,
            "handoff_direction_mismatch"
        );
        assert_eq!(
            storage
                .build_task_to_execute(task_path, "", &request)
                .unwrap_err()
                .reason_code,
            "handoff_task_not_found"
        );
        assert_eq!(
            storage
                .build_task_to_execute(task_path, "TASK-999", &request)
                .unwrap_err()
                .reason_code,
            "handoff_task_not_found"
        );
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: storage.project_root.clone(),
        };
        let mut validation = load_collection(
            &LocalHierarchyCatalog {
                skill_root: storage.skill_root.clone(),
            },
            &LocalSkillCatalog { roots: vec![] },
            &paths,
            &LocalTaskStorage {
                project_root: storage.project_root.clone(),
            },
            &[],
            task_path,
        )
        .unwrap();
        validation["task_skill_ids"]["TASK-001"] = json!("repo:confirmed-skill");
        let bound =
            work_feature::handoff::build_task_to_execute(&paths, &validation, "TASK-001", &request)
                .unwrap();
        assert_eq!(bound["source"]["skill_id"], "repo:confirmed-skill");
    }

    #[test]
    fn execute_return_requires_selected_task_and_complete_semantic_request() {
        let storage = LocalHandoffStorage {
            project_root: PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/work-infrastructure/fixtures/task-diagnostics"
            )),
            skill_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work")),
            skill_configs: vec![],
        };
        let task_path = "outputs/work/tasks/example/index.json";
        let request = json!({"summary":"Review specification.","reason":"Specification defect.","confirmed_approach":"Retain the interface.","requested_changes":["Clarify scope."],"preserve":["Current behavior."],"affected_ids":["TASK-001"],"validation_requirements":["Review criteria."]});
        assert_eq!(
            storage
                .build_preflight_return("execute_to_task", task_path, "TASK-999", &request)
                .unwrap_err()
                .reason_code,
            "handoff_task_not_found"
        );
        for (affected, reason) in [
            (json!(["GOAL-999"]), "handoff_unknown_affected_ids"),
            (json!(["TASK-001", "TASK-001"]), "duplicate_array_value"),
        ] {
            let mut invalid = request.clone();
            invalid["affected_ids"] = affected;
            assert_eq!(
                storage
                    .build_preflight_return("execute_to_task", task_path, "TASK-001", &invalid)
                    .unwrap_err()
                    .reason_code,
                reason
            );
        }
        for field in [
            "summary",
            "confirmed_approach",
            "requested_changes",
            "preserve",
            "affected_ids",
            "validation_requirements",
        ] {
            let mut invalid = request.clone();
            invalid.as_object_mut().unwrap().remove(field);
            assert_eq!(
                storage
                    .build_preflight_return("execute_to_task", task_path, "TASK-001", &invalid)
                    .unwrap_err()
                    .reason_code,
                "invalid_object_fields",
                "{field}"
            );
        }
        for field in ["source", "artifacts", "task_id", "skill_id", "direction"] {
            let mut invalid = request.clone();
            invalid[field] = json!("override");
            assert_eq!(
                storage
                    .build_preflight_return("execute_to_task", task_path, "TASK-001", &invalid)
                    .unwrap_err()
                    .reason_code,
                "invalid_object_fields",
                "{field}"
            );
        }
        let mut empty_changes = request.clone();
        empty_changes["requested_changes"] = json!([]);
        assert_eq!(
            storage
                .build_preflight_return("execute_to_task", task_path, "TASK-001", &empty_changes)
                .unwrap_err()
                .reason_code,
            "invalid_string_array"
        );
    }

    #[test]
    fn execute_preflight_returns_match_current_contract_both_directions() {
        let storage = LocalHandoffStorage {
            project_root: PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/work-infrastructure/fixtures/task-diagnostics"
            )),
            skill_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work")),
            skill_configs: vec![],
        };
        let task_path = "outputs/work/tasks/example/index.json";
        let request = json!({"summary":"Review specification.","reason":"Specification defect.",
            "confirmed_approach":"Retain the interface.",
            "requested_changes":["Clarify scope."],"preserve":["Current behavior."],
            "affected_ids":["TASK-001"],"validation_requirements":["Review criteria."]});
        let source = json!({"stage":"execute","task_spec_id":"TASK-SPEC-001","task_id":"TASK-001",
            "task_collection_sha256":"0a89ab8518e109f74a3b515b15cd9879d53e3475c8232cced6a9d141a77d4716",
            "task_index_sha256":"5a6561e945ac751d2afe854caea5657dda95b6492416fe834f1f435e171deb1b",
            "task_item_sha256":"f1071ad659955de4e360da6b0e53b6f99870d9bede067bb26e0341749c594abb",
            "task_instructions_sha256":"bbb1285eeb68a50a9c91c7ce8ebadce33aba447624a33ab31f2708f751fee875",
            "skill_id":null,"execute_skill_selection_sha256":"a09357ef9f22c43dca16b489da61b287838bf64963a5956bf52ae0327e04a959",
            "execution_context":{"attempt":{"status":"not_created"},"phase":"preflight",
                "issue_type":"specification_defect","reason":"Specification defect."}});
        {
            let (direction, target) = ("execute_to_task", "task");
            let result = storage
                .build_preflight_return(direction, task_path, "TASK-001", &request)
                .unwrap();
            assert_eq!(
                result,
                json!({"schema":"work-handoff","marker":"WORK-HANDOFF",
                "direction":direction,"requirement_id":"example",
                "artifacts":{"source":"outputs/work/sources/example","task":task_path,
                    "execution":"outputs/work/executions/example"},
                "source":source,"target":{"stage":target},
                "summary":request["summary"],"confirmed_approach":request["confirmed_approach"],
                "requested_changes":request["requested_changes"],"preserve":request["preserve"],
                "affected_ids":request["affected_ids"],
                "validation_requirements":request["validation_requirements"]})
            );
            assert_eq!(
                storage
                    .verify_preflight_return(direction, task_path, "TASK-001", &result,)
                    .unwrap(),
                json!({"schema":"work-handoff-source-validation","status":"valid",
                "direction":direction,"marker":"WORK-HANDOFF","requirement_id":"example",
                "source_stage":"execute","target_stage":target,
                "task_path":task_path,"source":source})
            );
        }
        for invalid in [
            json!({"summary":"Review specification.","reason":"Specification defect.",
                "confirmed_approach":"Retain the interface.","requested_changes":["Clarify scope."],
                "preserve":["Current behavior."],"affected_ids":["TASK-001"],
                "validation_requirements":["Review criteria."],"source":{}}),
            json!({"summary":"Review specification.","reason":"Specification defect.",
                "confirmed_approach":"Retain the interface.","requested_changes":["Clarify scope."],
                "preserve":["Current behavior."],"affected_ids":["TASK-001"],
                "validation_requirements":["Review criteria."],"attempt":{"status":"not_created"}}),
            json!({"summary":"Review specification.","reason":"",
                "confirmed_approach":"Retain the interface.","requested_changes":["Clarify scope."],
                "preserve":["Current behavior."],"affected_ids":["TASK-001"],
                "validation_requirements":["Review criteria."]}),
            json!({"summary":"Review specification.","reason":"Specification defect.",
                "confirmed_approach":"Retain the interface.","requested_changes":["Clarify scope."],
                "preserve":["Current behavior."],"affected_ids":["INPUT-999"],
                "validation_requirements":["Review criteria."]}),
            json!({"summary":"Review specification.",
                "confirmed_approach":"Retain the interface.","requested_changes":["Clarify scope."],
                "preserve":["Current behavior."],"affected_ids":["TASK-001"],
                "validation_requirements":["Review criteria."]}),
        ] {
            assert!(
                storage
                    .build_preflight_return("execute_to_task", task_path, "TASK-001", &invalid)
                    .is_err(),
                "{invalid}"
            );
        }
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: storage.project_root.clone(),
        };
        let mut validation = load_collection(
            &LocalHierarchyCatalog {
                skill_root: storage.skill_root.clone(),
            },
            &LocalSkillCatalog { roots: vec![] },
            &paths,
            &LocalTaskStorage {
                project_root: storage.project_root.clone(),
            },
            &[],
            task_path,
        )
        .unwrap();
        validation["collection_contract"]["tasks"][0]["inputs"] = json!([{
            "id":"INPUT-001","kind":"user_provided","source":"User specification",
            "precondition":"User confirms detail"}]);
        let index = parse_json_contract(
            &fs::read(
                storage
                    .project_root
                    .join("outputs/work/executions/example/index.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let returned = work_feature::handoff::build_preflight_return(
            &crate::artifact_paths::LocalArtifactPaths {
                project_root: storage.project_root.clone(),
            },
            &validation,
            &index,
            "execute_to_task",
            "TASK-001",
            &request,
        )
        .unwrap();
        assert_eq!(
            returned["source"]["execution_context"]["phase"],
            "preflight"
        );
        assert_eq!(
            work_feature::execution::preflight_index(
                &crate::execution::storage::LocalExecutionStorage {
                    project_root: storage.project_root.clone()
                },
                &validation,
                "outputs/work/executions/example",
                "TASK-001",
                &[],
                None,
                &["pending"]
            )
            .unwrap_err()
            .reason_code,
            "execute_preflight_input_confirmation_required"
        );
    }

    #[test]
    fn preflight_return_accepts_empty_target_and_rejects_orphan_or_transaction() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-diagnostics");
        let task_path = "outputs/work/tasks/example/index.json";
        let request = json!({"summary":"Review specification.","reason":"Specification defect.",
            "confirmed_approach":"Retain the interface.",
            "requested_changes":["Clarify scope."],"preserve":["Current behavior."],
            "affected_ids":["TASK-001"],"validation_requirements":["Review criteria."]});
        for (case, expected_reason) in [
            ("empty", None),
            ("orphan", Some("handoff_attempt_artifacts_present")),
            ("transaction", Some("handoff_execution_recovery_required")),
            ("index-drift", Some("any")),
            ("skill-drift", Some("any")),
        ] {
            let root = std::env::temp_dir().join(format!(
                "work-preflight-handoff-{case}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            for relative in [
                task_path,
                "outputs/work/tasks/example/tasks/TASK-001.json",
                "outputs/work/executions/example/index.json",
            ] {
                let destination = root.join(relative);
                fs::create_dir_all(destination.parent().unwrap()).unwrap();
                fs::copy(fixture.join(relative), destination).unwrap();
                crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
            }
            let execution = root.join("outputs/work/executions/example");
            let target = execution.join("TASK-001");
            fs::create_dir_all(&target).unwrap();
            if case == "orphan" {
                fs::create_dir(target.join("ATTEMPT-001")).unwrap();
            } else if case == "transaction" {
                fs::write(
                    execution.join(".work-attempt-start-TASK-001-ATTEMPT-001-lock.tmp"),
                    b"pending",
                )
                .unwrap();
            } else if case == "index-drift" || case == "skill-drift" {
                let index_path = execution.join("index.json");
                let mut index = parse_json_contract(&fs::read(&index_path).unwrap()).unwrap();
                if case == "index-drift" {
                    index["task_collection_sha256"] = json!("0".repeat(64));
                } else {
                    index["tasks"][0]["instructions_sha256"] = json!("0".repeat(64));
                }
                fs::write(
                    &index_path,
                    work_operations::execution::index::render_execution_index(&index).unwrap(),
                )
                .unwrap();
            }
            let storage = LocalHandoffStorage {
                project_root: root,
                skill_root: repo.join("../skills/work"),
                skill_configs: vec![],
            };
            let result =
                storage.build_preflight_return("execute_to_task", task_path, "TASK-001", &request);
            if let Some(reason) = expected_reason {
                if reason == "any" {
                    assert!(result.is_err(), "{case}");
                } else {
                    assert_eq!(result.unwrap_err().reason_code, reason, "{case}");
                }
            } else {
                assert_eq!(
                    result.unwrap()["source"]["execution_context"]["phase"],
                    "preflight"
                );
                storage
                    .require_unstarted_task_directory("outputs/work/executions/example", "TASK-001")
                    .unwrap();
                fs::create_dir(target.join("ATTEMPT-001")).unwrap();
                assert_eq!(
                    storage
                        .require_unstarted_task_directory(
                            "outputs/work/executions/example",
                            "TASK-001"
                        )
                        .unwrap_err()
                        .reason_code,
                    "handoff_attempt_artifacts_present"
                );
            }
        }
    }

    #[test]
    fn specification_transaction_marker_gates_task_and_execute_handoffs() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-diagnostics");
        let root = std::env::temp_dir().join(format!(
            "work-handoff-spec-marker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let task_path = "outputs/work/tasks/example/index.json";
        for relative in [
            task_path,
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
            crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        }
        let storage = LocalHandoffStorage {
            project_root: root.clone(),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![],
        };
        let execute_request = json!({"summary":"Start execution."});
        let return_request = json!({"summary":"Review specification.",
            "confirmed_approach":"Retain interface.","requested_changes":["Clarify scope."],
            "preserve":["Current behavior."],"affected_ids":["TASK-001"],
            "validation_requirements":["Review criteria."]});
        let mut preflight_request = return_request.clone();
        preflight_request["reason"] = json!("Specification defect.");
        let task_handoff = storage
            .build_task_to_execute(task_path, "TASK-001", &execute_request)
            .unwrap();
        let return_handoff = storage
            .build_preflight_return("execute_to_task", task_path, "TASK-001", &preflight_request)
            .unwrap();
        let handoff = return_handoff.clone();
        let (validation, snapshot) = storage.task_snapshot(task_path).unwrap();
        assert_eq!(
            storage
                .verify_task_to_execute(task_path, "TASK-001", &handoff)
                .unwrap_err()
                .reason_code,
            "handoff_direction_mismatch"
        );
        let journal =
            root.join("outputs/work/executions/example/.work-spec-update-SPEC-UPDATE-001.json");
        let journal_raw = b"{\"transaction\":\"test\"}\n";
        fs::write(&journal, journal_raw).unwrap();
        assert_eq!(
            storage
                .verify_task_to_execute(task_path, "TASK-001", &task_handoff)
                .unwrap_err()
                .reason_code,
            "spec_update_pending"
        );
        assert_eq!(
            storage
                .verify_preflight_return("execute_to_task", task_path, "TASK-001", &return_handoff)
                .unwrap_err()
                .reason_code,
            "spec_update_pending"
        );
        assert_eq!(
            storage
                .recheck_task(&validation, &snapshot)
                .unwrap_err()
                .reason_code,
            "spec_update_pending"
        );
        let check_pending = || {
            let results = [
                storage.verify_task_to_execute(task_path, "TASK-001", &task_handoff),
                storage.build_task_to_execute(task_path, "TASK-001", &execute_request),
                storage.verify_preflight_return(
                    "execute_to_task",
                    task_path,
                    "TASK-001",
                    &return_handoff,
                ),
                storage.build_preflight_return(
                    "execute_to_task",
                    task_path,
                    "TASK-001",
                    &preflight_request,
                ),
            ];
            for result in results {
                let error = result.unwrap_err();
                assert_eq!(error.reason_code, "spec_update_pending");
                assert_eq!(error.details["recovery_required"], true);
            }
        };
        check_pending();
        let marker = PathBuf::from(format!("{}.done", journal.display()));
        fs::write(&marker, b"incorrect\n").unwrap();
        check_pending();
        fs::write(
            &marker,
            work_operations::derivation::publication::completion_marker(journal_raw),
        )
        .unwrap();
        assert_eq!(
            storage
                .verify_task_to_execute(task_path, "TASK-001", &task_handoff)
                .unwrap()["status"],
            "valid"
        );
        storage
            .build_task_to_execute(task_path, "TASK-001", &execute_request)
            .unwrap();
        storage
            .build_preflight_return("execute_to_task", task_path, "TASK-001", &preflight_request)
            .unwrap();

        storage
            .verify_preflight_return("execute_to_task", task_path, "TASK-001", &return_handoff)
            .unwrap();
        assert!(!root.join("outputs/work/plans").exists());
    }
    #[test]
    fn closed_return_requires_completed_specification_transaction() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let historical_skill =
            crate::fixture_support::historical_execute_skill_root(&repo.join("../skills/work"))
                .unwrap();
        let fixture = repo.join("crates/work-infrastructure/fixtures/handoff-closed/stopped");
        let root = std::env::temp_dir().join(format!(
            "work-handoff-closed-marker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for relative in [
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/tasks/example/tasks/TASK-002.json",
            "outputs/work/executions/example/index.json",
            "outputs/work/executions/example/TASK-001/ATTEMPT-004/attempt.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
            crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        }
        let storage = LocalHandoffStorage {
            project_root: root.clone(),
            skill_root: historical_skill.clone(),
            skill_configs: vec![],
        };
        let request = json!({"summary":"調整已確認範圍",
            "confirmed_approach":"保留既有介面", "requested_changes":["新增驗收條件"],
            "preserve":["既有功能"], "affected_ids":["ACCEPTANCE-001","TASK-001"],
            "validation_requirements":["重新確認驗收條件"], "reason":"Clarify specification"});
        let journal =
            root.join("outputs/work/executions/example/.work-spec-update-SPEC-UPDATE-001.json");
        let journal_raw = b"{\"transaction\":\"test\"}\n";
        fs::write(&journal, journal_raw).unwrap();
        {
            let direction = "execute_to_task";
            assert_eq!(
                storage
                    .build_closed_return(
                        direction,
                        "outputs/work/tasks/example/index.json",
                        "TASK-001",
                        "ATTEMPT-004",
                        &request,
                    )
                    .unwrap_err()
                    .reason_code,
                "spec_update_pending"
            );
        }
        fs::write(
            PathBuf::from(format!("{}.done", journal.display())),
            work_operations::derivation::publication::completion_marker(journal_raw),
        )
        .unwrap();
        {
            let direction = "execute_to_task";
            let handoff = storage
                .build_closed_return(
                    direction,
                    "outputs/work/tasks/example/index.json",
                    "TASK-001",
                    "ATTEMPT-004",
                    &request,
                )
                .unwrap();
            assert_eq!(handoff["direction"], direction);
        }
        let task_path = "outputs/work/tasks/example/index.json";
        let handoff = storage
            .build_closed_return(
                "execute_to_task",
                task_path,
                "TASK-001",
                "ATTEMPT-004",
                &request,
            )
            .unwrap();
        for invalid in [
            {
                let mut value = request.clone();
                value["source"] = json!({});
                value
            },
            {
                let mut value = request.clone();
                value["phase"] = json!("preflight");
                value
            },
            {
                let mut value = request.clone();
                value["reason"] = json!("");
                value
            },
            {
                let mut value = request.clone();
                value["affected_ids"] = json!(["VAL-999"]);
                value
            },
            {
                let mut value = request.clone();
                value.as_object_mut().unwrap().remove("reason");
                value
            },
        ] {
            assert!(
                storage
                    .build_closed_return(
                        "execute_to_task",
                        task_path,
                        "TASK-001",
                        "ATTEMPT-004",
                        &invalid
                    )
                    .is_err(),
                "{invalid}"
            );
        }
        let attempt_path =
            root.join("outputs/work/executions/example/TASK-001/ATTEMPT-004/attempt.json");
        let attempt_raw = fs::read(&attempt_path).unwrap();
        for field in [
            "task_collection_sha256",
            "task_index_sha256",
            "task_item_sha256",
            "task_instructions_sha256",
            "execute_instructions_sha256",
            "execute_skill_selection_sha256",
            "skill_id",
        ] {
            let mut attempt = parse_json_contract(&attempt_raw).unwrap();
            attempt[field] = if field == "skill_id" {
                json!("unknown")
            } else {
                json!("0".repeat(64))
            };
            fs::write(
                &attempt_path,
                work_operations::execution::attempt::render_attempt(&attempt).unwrap(),
            )
            .unwrap();
            assert!(
                storage
                    .build_closed_return(
                        "execute_to_task",
                        task_path,
                        "TASK-001",
                        "ATTEMPT-004",
                        &request
                    )
                    .is_err(),
                "{field}"
            );
        }
        fs::write(&attempt_path, &attempt_raw).unwrap();
        for status in ["in_progress", "completed"] {
            let mut attempt = parse_json_contract(&attempt_raw).unwrap();
            attempt["status"] = json!(status);
            for field in ["final_type", "reason", "closing_authorization_evidence"] {
                attempt.as_object_mut().unwrap().remove(field);
            }
            if status == "in_progress" {
                attempt.as_object_mut().unwrap().remove("ended_at");
            }
            fs::write(
                &attempt_path,
                work_operations::execution::attempt::render_attempt(&attempt).unwrap(),
            )
            .unwrap();
            assert!(
                storage
                    .build_closed_return(
                        "execute_to_task",
                        task_path,
                        "TASK-001",
                        "ATTEMPT-004",
                        &request
                    )
                    .is_err(),
                "{status}"
            );
        }
        let mut changed_status = parse_json_contract(&attempt_raw).unwrap();
        changed_status["status"] = json!("blocked");
        changed_status["final_type"] = json!("required_input");
        fs::write(
            &attempt_path,
            work_operations::execution::attempt::render_attempt(&changed_status).unwrap(),
        )
        .unwrap();
        assert_eq!(
            LocalHandoffStorage::recheck_file(&attempt_path, &attempt_raw, "attempt_path")
                .unwrap_err()
                .reason_code,
            "handoff_source_changed"
        );
        assert!(
            storage
                .verify_closed_return(
                    "execute_to_task",
                    task_path,
                    "TASK-001",
                    "ATTEMPT-004",
                    &handoff
                )
                .is_err()
        );
        fs::write(&attempt_path, &attempt_raw).unwrap();
        let index_path = root.join("outputs/work/executions/example/index.json");
        let index_raw = fs::read(&index_path).unwrap();
        let mut index = parse_json_contract(&index_raw).unwrap();
        index["task_collection_sha256"] = json!("0".repeat(64));
        fs::write(
            &index_path,
            work_operations::execution::index::render_execution_index(&index).unwrap(),
        )
        .unwrap();
        assert_eq!(
            LocalHandoffStorage::recheck_file(&index_path, &index_raw, "execution_index")
                .unwrap_err()
                .reason_code,
            "handoff_source_changed"
        );
        assert!(
            storage
                .build_closed_return(
                    "execute_to_task",
                    task_path,
                    "TASK-001",
                    "ATTEMPT-004",
                    &request
                )
                .is_err()
        );
        fs::write(&index_path, &index_raw).unwrap();
        let recovery = root.join("outputs/work/executions/example/.work-attempt-close-pending.tmp");
        fs::write(&recovery, b"pending").unwrap();
        assert_eq!(
            storage
                .build_closed_return(
                    "execute_to_task",
                    task_path,
                    "TASK-001",
                    "ATTEMPT-004",
                    &request
                )
                .unwrap_err()
                .reason_code,
            "handoff_execution_recovery_required"
        );
    }

    #[test]
    fn task_handoff_rejects_invalid_request_before_reading_and_preserves_inputs() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-diagnostics");
        let storage = LocalHandoffStorage {
            project_root: fixture.clone(),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![],
        };
        let path = "outputs/work/tasks/example/index.json";
        let before = fs::read(fixture.join(path)).unwrap();
        for field in [
            "schema",
            "direction",
            "source",
            "target",
            "artifacts",
            "requirement_id",
        ] {
            let mut request = json!({"summary":"Start execution."});
            request[field] = json!("override");
            assert_eq!(
                storage
                    .build_task_to_execute("missing/index.json", "TASK-001", &request)
                    .unwrap_err()
                    .reason_code,
                "invalid_object_fields"
            );
        }
        assert_eq!(
            storage
                .build_task_to_execute("missing/index.json", "TASK-001", &json!({}))
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
        assert!(
            storage
                .build_task_to_execute(
                    "../index.json",
                    "TASK-001",
                    &json!({"summary":"Start execution."})
                )
                .is_err()
        );
        assert_eq!(fs::read(fixture.join(path)).unwrap(), before);
    }
}
