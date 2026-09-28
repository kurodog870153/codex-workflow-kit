//! Read-only Handoff construction from a validated formal Plan.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::handoff::{
    ClosedReturnInput, build_closed_return, build_plan_to_task, build_preflight_return,
    build_task_to_execute, build_task_to_plan, validate_handoff, validate_plan_to_task_request,
    validate_preflight_return_request, validate_task_to_execute_request,
    validate_task_to_plan_request, verify_plan_to_task, verify_return_against_expected,
    verify_task_to_execute, verify_task_to_plan,
};
use work_feature::handoff::{HandoffCommandRepository, HandoffStorageAction};
use work_feature::instruction::select;
use work_feature::plan::{PlanPathRepository, validate_plan_bytes};
use work_feature::ports::ArtifactStore;
use work_feature::skill::SkillRoot;
use work_feature::task::{TaskCollectionRepository, load_collection};
use work_operations::canonical::parse_json_contract;
use work_operations::execution::attempt::validate_attempt_bytes;
use work_operations::execution::index::validate_execution_index;

use crate::files::{LocalFiles, resolve_project_path};
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::plan_storage::LocalPlanStorage;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::specification::storage::require_no_spec_update;
use crate::task::storage::LocalTaskStorage;

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
            HandoffStorageAction::PlanToTask { verify, plan_path } => {
                if verify {
                    self.verify_plan_to_task(plan_path, request)
                } else {
                    self.build_plan_to_task(plan_path, request)
                }
            }
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
            HandoffStorageAction::TaskToPlan {
                verify,
                plan_path,
                task_path,
                task_id,
            } => {
                if verify {
                    self.verify_task_to_plan(
                        plan_path.expect("verify requires plan"),
                        task_path,
                        task_id,
                        request,
                    )
                } else {
                    self.build_task_to_plan(task_path, task_id, request)
                }
            }
            HandoffStorageAction::ExecuteReturn {
                verify,
                preflight,
                direction,
                plan_path,
                task_path,
                task_id,
                attempt_id,
            } => {
                if verify {
                    let plan_path = plan_path.expect("verify requires plan");
                    if preflight {
                        self.verify_preflight_return(
                            direction, plan_path, task_path, task_id, request,
                        )
                    } else {
                        self.verify_closed_return(
                            direction,
                            plan_path,
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

    fn task_snapshot(
        &self,
        task_path: &str,
    ) -> Result<(Value, BTreeMap<String, Vec<u8>>), WorkError> {
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
        let paths = LocalPlanStorage {
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
        if work_operations::canonical::sha256_hex(&index_raw) != validation["task_index_sha256"] {
            return Err(Self::source_changed("task_path"));
        }
        snapshot.insert(normalized.clone(), index_raw);
        let directory = normalized.rsplit_once('/').map_or("", |(parent, _)| parent);
        for reference in contract["tasks"].as_array().expect("validated TASK items") {
            let id = reference["id"].as_str().expect("validated ID");
            let relative = format!("{directory}/tasks/{id}.json");
            let raw = repository.read_task_file(&relative)?;
            if work_operations::canonical::sha256_hex(&raw) != validation["task_item_sha256"][id] {
                return Err(Self::source_changed("task_path"));
            }
            snapshot.insert(relative, raw);
        }
        let plan_path = contract["artifacts"]["plan"]
            .as_str()
            .expect("validated Plan path");
        let plan_raw = paths.read(plan_path)?;
        if work_operations::canonical::sha256_hex(&plan_raw) != validation["source_plan_sha256"] {
            return Err(Self::source_changed("plan_path"));
        }
        snapshot.insert(plan_path.into(), plan_raw);
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
        snapshot: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(), WorkError> {
        let repository = LocalTaskStorage {
            project_root: self.project_root.clone(),
        };
        for (path, raw) in snapshot {
            if repository.read_task_file(path).ok().as_deref() != Some(raw.as_slice()) {
                let field = if validation["collection_contract"]["artifacts"]["plan"].as_str()
                    == Some(path.as_str())
                {
                    "plan_path"
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

    fn validated_plan(
        &self,
        plan_path: &str,
    ) -> Result<(Value, Value, PathBuf, Vec<u8>), WorkError> {
        let (normalized, resolved) = resolve_project_path(&self.project_root, plan_path)?;
        let paths = LocalPlanStorage {
            project_root: self.project_root.clone(),
        };
        let raw = paths.read(&normalized)?;
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
        let validation =
            validate_plan_bytes(&instructions, &skills, &paths, &roots, &raw, &normalized)?;
        let plan = parse_json_contract(&raw).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The Plan JSON is invalid.",
                json!({}),
            )
        })?;
        require_no_spec_update(
            &self.project_root,
            plan["artifacts"]["execution"]
                .as_str()
                .expect("validated execution path"),
            None,
        )?;
        let resolved = resolved
            .canonicalize()
            .map_err(|_| Self::source_changed("plan_path"))?;
        Ok((plan, validation, resolved, raw))
    }

    fn recheck_plan(
        &self,
        plan_path: &str,
        resolved: &PathBuf,
        raw: &[u8],
        plan: &Value,
    ) -> Result<(), WorkError> {
        let (_, current) = resolve_project_path(&self.project_root, plan_path)
            .map_err(|_| Self::source_changed("plan_path"))?;
        let current = current
            .canonicalize()
            .map_err(|_| Self::source_changed("plan_path"))?;
        let paths = LocalPlanStorage {
            project_root: self.project_root.clone(),
        };
        if current != *resolved || paths.read(plan_path).ok().as_deref() != Some(raw) {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "handoff_source_changed",
                "A source artifact changed during handoff construction.",
                json!({"field":"plan_path"}),
            ));
        }
        require_no_spec_update(
            &self.project_root,
            plan["artifacts"]["execution"]
                .as_str()
                .expect("validated execution path"),
            None,
        )
    }

    pub fn build_plan_to_task(&self, plan_path: &str, request: &Value) -> Result<Value, WorkError> {
        validate_plan_to_task_request(request)?;
        let (plan, validation, resolved, raw) = self.validated_plan(plan_path)?;
        let paths = LocalPlanStorage {
            project_root: self.project_root.clone(),
        };
        let result = build_plan_to_task(&paths, &plan, &validation, request)?;
        self.recheck_plan(plan_path, &resolved, &raw, &plan)?;
        Ok(result)
    }

    pub fn verify_plan_to_task(
        &self,
        plan_path: &str,
        incoming: &Value,
    ) -> Result<Value, WorkError> {
        let paths = LocalPlanStorage {
            project_root: self.project_root.clone(),
        };
        validate_handoff(&paths, incoming)?;
        let (plan, validation, resolved, raw) = self.validated_plan(plan_path)?;
        let result = verify_plan_to_task(&paths, &plan, &validation, incoming)?;
        self.recheck_plan(plan_path, &resolved, &raw, &plan)?;
        Ok(result)
    }

    pub fn build_task_to_execute(
        &self,
        task_path: &str,
        task_id: &str,
        request: &Value,
    ) -> Result<Value, WorkError> {
        validate_task_to_execute_request(request)?;
        let (validation, snapshot) = self.task_snapshot(task_path)?;
        let paths = LocalPlanStorage {
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
        let paths = LocalPlanStorage {
            project_root: self.project_root.clone(),
        };
        validate_handoff(&paths, incoming)?;
        let (validation, snapshot) = self.task_snapshot(task_path)?;
        let result = verify_task_to_execute(&paths, &validation, task_id, incoming)?;
        self.recheck_task(&validation, &snapshot)?;
        Ok(result)
    }

    pub fn build_task_to_plan(
        &self,
        task_path: &str,
        task_id: Option<&str>,
        request: &Value,
    ) -> Result<Value, WorkError> {
        validate_task_to_plan_request(request)?;
        let (validation, snapshot) = self.task_snapshot(task_path)?;
        let plan_path = validation["collection_contract"]["artifacts"]["plan"]
            .as_str()
            .expect("validated Plan path");
        let plan = parse_json_contract(&snapshot[plan_path]).expect("validated Plan bytes");
        let paths = LocalPlanStorage {
            project_root: self.project_root.clone(),
        };
        let result = build_task_to_plan(&paths, &validation, &plan, task_id, request)?;
        self.recheck_task(&validation, &snapshot)?;
        Ok(result)
    }

    pub fn verify_task_to_plan(
        &self,
        plan_path: &str,
        task_path: &str,
        task_id: Option<&str>,
        incoming: &Value,
    ) -> Result<Value, WorkError> {
        let paths = LocalPlanStorage {
            project_root: self.project_root.clone(),
        };
        validate_handoff(&paths, incoming)?;
        work_feature::handoff::validate_return_direction(incoming, "task_to_plan", false)?;
        let normalized_plan = resolve_project_path(&self.project_root, plan_path)?.0;
        let normalized_task = resolve_project_path(&self.project_root, task_path)?.0;
        work_feature::handoff::validate_return_paths(incoming, &normalized_plan, &normalized_task)?;
        let (validation, snapshot) = self.task_snapshot(&normalized_task)?;
        let plan_raw = snapshot.get(&normalized_plan).ok_or_else(|| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                "handoff_source_mismatch",
                "The return does not match the receiver's confirmed artifact paths.",
                json!({"fields":["artifacts"]}),
            )
        })?;
        let plan = parse_json_contract(plan_raw).expect("validated Plan bytes");
        let result = verify_task_to_plan(&paths, &validation, &plan, task_id, incoming)?;
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
        let plan_path = validation["collection_contract"]["artifacts"]["plan"]
            .as_str()
            .expect("validated Plan path");
        let plan = parse_json_contract(&snapshot[plan_path]).expect("validated Plan bytes");
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
        let paths = LocalPlanStorage {
            project_root: self.project_root.clone(),
        };
        let result = build_preflight_return(
            &paths,
            &validation,
            &plan,
            &index,
            direction,
            task_id,
            request,
        )?;
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
        plan_path: &str,
        task_path: &str,
        task_id: &str,
        incoming: &Value,
    ) -> Result<Value, WorkError> {
        let paths = LocalPlanStorage {
            project_root: self.project_root.clone(),
        };
        validate_handoff(&paths, incoming)?;
        work_feature::handoff::validate_return_direction(incoming, direction, true)?;
        let normalized_plan = resolve_project_path(&self.project_root, plan_path)?.0;
        let normalized_task = resolve_project_path(&self.project_root, task_path)?.0;
        work_feature::handoff::validate_return_paths(incoming, &normalized_plan, &normalized_task)?;
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
        let plan_path = validation["collection_contract"]["artifacts"]["plan"]
            .as_str()
            .expect("validated Plan path");
        let plan = parse_json_contract(&snapshot[plan_path]).expect("validated Plan bytes");
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
        let paths = LocalPlanStorage {
            project_root: self.project_root.clone(),
        };
        let result = build_closed_return(
            &paths,
            &ClosedReturnInput {
                validation: &validation,
                plan: &plan,
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
        plan_path: &str,
        task_path: &str,
        task_id: &str,
        attempt_id: &str,
        incoming: &Value,
    ) -> Result<Value, WorkError> {
        let paths = LocalPlanStorage {
            project_root: self.project_root.clone(),
        };
        validate_handoff(&paths, incoming)?;
        work_feature::handoff::validate_return_direction(incoming, direction, true)?;
        let normalized_plan = resolve_project_path(&self.project_root, plan_path)?.0;
        let normalized_task = resolve_project_path(&self.project_root, task_path)?.0;
        work_feature::handoff::validate_return_paths(incoming, &normalized_plan, &normalized_task)?;
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

    use work_feature::plan::prepare_semantic;
    use work_operations::plan::render_plan_value;

    #[test]
    fn task_handoff_rejects_stale_plan_revised_task_and_noncanonical_index() {
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
        for relative in [
            plan_path,
            task_path,
            "outputs/work/tasks/example/tasks/TASK-001.json",
        ] {
            let target = root.join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), target).unwrap();
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
        let return_request = json!({"summary":"Review specification.",
            "confirmed_approach":"Retain the interface.",
            "requested_changes":["Clarify scope."],"preserve":["Current behavior."],
            "affected_ids":["TASK-001"],"validation_requirements":["Review criteria."]});
        let return_handoff = storage
            .build_task_to_plan(task_path, None, &return_request)
            .unwrap();
        let mut wrong_direction = return_handoff.clone();
        wrong_direction["direction"] = json!("execute_to_plan");
        assert!(
            storage
                .verify_task_to_plan(plan_path, task_path, None, &wrong_direction)
                .is_err()
        );
        assert!(
            storage
                .verify_task_to_plan(
                    "custom/plans/example.json",
                    task_path,
                    None,
                    &return_handoff
                )
                .is_err()
        );
        let mut unknown_affected = return_handoff.clone();
        unknown_affected["affected_ids"] = json!(["GOAL-999"]);
        assert!(
            storage
                .verify_task_to_plan(plan_path, task_path, None, &unknown_affected)
                .is_err()
        );
        let mut missing_collection = return_handoff.clone();
        missing_collection["source"]
            .as_object_mut()
            .unwrap()
            .remove("task_collection_sha256");
        assert!(
            storage
                .verify_task_to_plan(plan_path, task_path, None, &missing_collection)
                .is_err()
        );
        let (validation, snapshot) = storage.task_snapshot(task_path).unwrap();
        storage.recheck_task(&validation, &snapshot).unwrap();
        let (formal_plan, _, resolved_plan, checked_plan_raw) =
            storage.validated_plan(plan_path).unwrap();
        storage
            .recheck_plan(plan_path, &resolved_plan, &checked_plan_raw, &formal_plan)
            .unwrap();
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
        assert!(
            storage
                .verify_task_to_plan(plan_path, task_path, None, &return_handoff)
                .is_err()
        );
        fs::write(root.join(task_path), &index_raw).unwrap();
        let plan_raw = fs::read(root.join(plan_path)).unwrap();
        let mut plan = parse_json_contract(&plan_raw).unwrap();
        plan["summary"] = json!("Revised Plan");
        fs::write(root.join(plan_path), render_plan_value(&plan).unwrap()).unwrap();
        assert_eq!(
            storage
                .recheck_task(&validation, &snapshot)
                .unwrap_err()
                .reason_code,
            "handoff_source_changed"
        );
        assert_eq!(
            storage
                .recheck_plan(plan_path, &resolved_plan, &checked_plan_raw, &formal_plan)
                .unwrap_err()
                .reason_code,
            "handoff_source_changed"
        );
        assert_eq!(
            storage
                .build_task_to_execute(task_path, "TASK-001", &request)
                .unwrap_err()
                .reason_code,
            "source_plan_fingerprint_mismatch"
        );
        assert!(
            storage
                .verify_task_to_execute(task_path, "TASK-001", &handoff)
                .is_err()
        );
        assert_eq!(
            storage
                .build_task_to_plan(task_path, None, &return_request)
                .unwrap_err()
                .reason_code,
            "source_plan_fingerprint_mismatch"
        );
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
        #[cfg(unix)]
        {
            let original = root.join(plan_path);
            let redirected = root.join("alternate-plan.json");
            fs::rename(&original, &redirected).unwrap();
            std::os::unix::fs::symlink(&redirected, &original).unwrap();
            assert_eq!(
                storage
                    .recheck_plan(plan_path, &resolved_plan, &checked_plan_raw, &formal_plan)
                    .unwrap_err()
                    .reason_code,
                "handoff_source_changed"
            );
        }
    }

    #[test]
    fn closed_attempt_returns_match_python_stopped_and_blocked() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let request = json!({"summary":"調整已確認範圍",
            "confirmed_approach":"保留既有介面", "requested_changes":["新增驗收條件"],
            "preserve":["既有功能"],"affected_ids":["GOAL-001","TASK-001"],
            "validation_requirements":["重新確認驗收條件"],"reason":"Clarify specification"});
        let plan_path = "outputs/work/plans/example.json";
        let task_path = "outputs/work/tasks/example/index.json";
        for status in ["stopped", "blocked"] {
            let fixture = repo
                .join("crates/work-infrastructure/fixtures/handoff-closed")
                .join(status);
            let storage = LocalHandoffStorage {
                project_root: fixture.clone(),
                skill_root: repo.join("../skills/work"),
                skill_configs: vec![],
            };
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
            for direction in ["execute_to_task", "execute_to_plan"] {
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
                            "ATTEMPT-001",
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
                            plan_path,
                            task_path,
                            "TASK-001",
                            "ATTEMPT-001",
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
                            plan_path,
                            task_path,
                            "TASK-001",
                            "ATTEMPT-001",
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
                            "ATTEMPT-002",
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
    fn plan_to_task_matches_python_reference_from_formal_collection() {
        let root = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/fixtures/task-diagnostics"
        ));
        let storage = LocalHandoffStorage {
            project_root: root,
            skill_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work")),
            skill_configs: vec![],
        };
        let plan_path = "outputs/work/plans/example.json";
        let request = json!({"summary":"Build the tasks.","affected_ids":["GOAL-001"]});
        let expected = json!({"schema":"work-handoff/v1","marker":"WORK-HANDOFF",
            "direction":"plan_to_task","requirement_id":"example",
            "artifacts":{"plan":plan_path,"task":"outputs/work/tasks/example/index.json",
                "execution":"outputs/work/executions/example"},
            "source":{"stage":"plan","plan_sha256":"fbe1d757987f30984e4ec769c13175da71bdc0b630a16d9adc88bbdd851b1217",
                "skill_selection_sha256":"a09357ef9f22c43dca16b489da61b287838bf64963a5956bf52ae0327e04a959"},
            "target":{"stage":"task"},"summary":"Build the tasks.","affected_ids":["GOAL-001"]});
        assert_eq!(
            storage.build_plan_to_task(plan_path, &request).unwrap(),
            expected
        );
        let checked = storage.verify_plan_to_task(plan_path, &expected).unwrap();
        assert_eq!(
            checked,
            json!({"schema":"work-handoff-source-validation/v1",
            "status":"valid","direction":"plan_to_task","marker":"WORK-HANDOFF",
            "requirement_id":"example","source_stage":"plan","target_stage":"task",
            "plan_path":plan_path,"source":expected["source"]})
        );
    }

    #[test]
    fn task_to_execute_matches_python_reference_from_formal_collection() {
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
            "task_collection_sha256":"e6a91e156bfa91f022fe0e4caea2a2e6a5f5680dd05a5bd0f29ca07514d0dc3e",
            "task_index_sha256":"c702e63ab5c9fd94e66a0ae2036286b05aa7535677b0963d2ca6b6a65890ff19",
            "task_item_sha256":"77b657de77811217b23514ae570473331da07c204b7553c51da60ba505c0a523",
            "task_instructions_sha256":"df8fac8b103d0139e30419503d6237e2edcd219b45bb9cca32d9e84dcfef532b",
            "skill_id":null,"skill_selection_sha256":"a09357ef9f22c43dca16b489da61b287838bf64963a5956bf52ae0327e04a959"});
        let expected = json!({"schema":"work-handoff/v1","marker":"WORK-HANDOFF",
            "direction":"task_to_execute","requirement_id":"example",
            "artifacts":{"plan":"outputs/work/plans/example.json","task":task_path,
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
            json!({"schema":"work-handoff-source-validation/v1","status":"valid",
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
            ("plan", "custom/example.json"),
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
                .verify_plan_to_task("outputs/work/plans/example.json", &expected)
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
        let paths = LocalPlanStorage {
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
    fn task_to_plan_matches_python_whole_spec_and_selected_task() {
        let storage = LocalHandoffStorage {
            project_root: PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/work-infrastructure/fixtures/task-diagnostics"
            )),
            skill_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work")),
            skill_configs: vec![],
        };
        let plan_path = "outputs/work/plans/example.json";
        let task_path = "outputs/work/tasks/example/index.json";
        let request = json!({"summary":"Review specification.",
            "confirmed_approach":"Retain the interface.",
            "requested_changes":["Clarify scope."],"preserve":["Current behavior."],
            "affected_ids":["TASK-001"],"validation_requirements":["Review criteria."]});
        let base_source = json!({"stage":"task","plan_sha256":"fbe1d757987f30984e4ec769c13175da71bdc0b630a16d9adc88bbdd851b1217",
            "task_spec_id":"TASK-SPEC-001",
            "task_collection_sha256":"e6a91e156bfa91f022fe0e4caea2a2e6a5f5680dd05a5bd0f29ca07514d0dc3e",
            "task_index_sha256":"c702e63ab5c9fd94e66a0ae2036286b05aa7535677b0963d2ca6b6a65890ff19",
            "skill_selection_sha256":"a09357ef9f22c43dca16b489da61b287838bf64963a5956bf52ae0327e04a959"});
        for task_id in [None, Some("TASK-001")] {
            let mut source = base_source.clone();
            if let Some(id) = task_id {
                source["task_id"] = json!(id);
                source["task_item_sha256"] =
                    json!("77b657de77811217b23514ae570473331da07c204b7553c51da60ba505c0a523");
                source["skill_id"] = Value::Null;
            }
            let expected = json!({"schema":"work-handoff/v1","marker":"WORK-HANDOFF",
                "direction":"task_to_plan","requirement_id":"example",
                "artifacts":{"plan":plan_path,"task":task_path,"execution":"outputs/work/executions/example"},
                "source":source,"target":{"stage":"plan"},
                "summary":request["summary"],"confirmed_approach":request["confirmed_approach"],
                "requested_changes":request["requested_changes"],"preserve":request["preserve"],
                "affected_ids":request["affected_ids"],
                "validation_requirements":request["validation_requirements"]});
            assert_eq!(
                storage
                    .build_task_to_plan(task_path, task_id, &request)
                    .unwrap(),
                expected
            );
            assert_eq!(
                storage
                    .verify_task_to_plan(plan_path, task_path, task_id, &expected)
                    .unwrap(),
                json!({"schema":"work-handoff-source-validation/v1","status":"valid",
                    "direction":"task_to_plan","marker":"WORK-HANDOFF","requirement_id":"example",
                    "source_stage":"task","target_stage":"plan","plan_path":plan_path,
                    "task_path":task_path,"source":source})
            );
            let mut wrong_plan = expected.clone();
            wrong_plan["artifacts"]["plan"] = json!("outputs/work/plans/alternate/example.json");
            assert_eq!(
                storage
                    .verify_task_to_plan(
                        "outputs/work/plans/alternate/example.json",
                        task_path,
                        task_id,
                        &wrong_plan,
                    )
                    .unwrap_err()
                    .reason_code,
                "handoff_source_mismatch"
            );
        }
        assert_eq!(
            storage
                .build_task_to_plan(task_path, Some("TASK-999"), &request)
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
                    .build_task_to_plan(task_path, None, &invalid)
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
                    .build_task_to_plan(task_path, None, &invalid)
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
                    .build_task_to_plan(task_path, None, &invalid)
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
                .build_task_to_plan(task_path, None, &empty_changes)
                .unwrap_err()
                .reason_code,
            "invalid_string_array"
        );
    }

    #[test]
    fn execute_preflight_returns_match_python_both_directions() {
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
            "task_collection_sha256":"e6a91e156bfa91f022fe0e4caea2a2e6a5f5680dd05a5bd0f29ca07514d0dc3e",
            "task_index_sha256":"c702e63ab5c9fd94e66a0ae2036286b05aa7535677b0963d2ca6b6a65890ff19",
            "task_item_sha256":"77b657de77811217b23514ae570473331da07c204b7553c51da60ba505c0a523",
            "task_instructions_sha256":"df8fac8b103d0139e30419503d6237e2edcd219b45bb9cca32d9e84dcfef532b",
            "skill_id":null,"execute_skill_selection_sha256":"a09357ef9f22c43dca16b489da61b287838bf64963a5956bf52ae0327e04a959",
            "execution_context":{"attempt":{"status":"not_created"},"phase":"preflight",
                "issue_type":"specification_defect","reason":"Specification defect."}});
        for (direction, target) in [("execute_to_task", "task"), ("execute_to_plan", "plan")] {
            let result = storage
                .build_preflight_return(direction, task_path, "TASK-001", &request)
                .unwrap();
            assert_eq!(
                result,
                json!({"schema":"work-handoff/v1","marker":"WORK-HANDOFF",
                "direction":direction,"requirement_id":"example",
                "artifacts":{"plan":"outputs/work/plans/example.json","task":task_path,
                    "execution":"outputs/work/executions/example"},
                "source":source,"target":{"stage":target},
                "summary":request["summary"],"confirmed_approach":request["confirmed_approach"],
                "requested_changes":request["requested_changes"],"preserve":request["preserve"],
                "affected_ids":request["affected_ids"],
                "validation_requirements":request["validation_requirements"]})
            );
            assert_eq!(
                storage
                    .verify_preflight_return(
                        direction,
                        "outputs/work/plans/example.json",
                        task_path,
                        "TASK-001",
                        &result,
                    )
                    .unwrap(),
                json!({"schema":"work-handoff-source-validation/v1","status":"valid",
                "direction":direction,"marker":"WORK-HANDOFF","requirement_id":"example",
                "source_stage":"execute","target_stage":target,
                "plan_path":"outputs/work/plans/example.json","task_path":task_path,"source":source})
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
        let paths = LocalPlanStorage {
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
        let plan = parse_json_contract(
            &fs::read(storage.project_root.join("outputs/work/plans/example.json")).unwrap(),
        )
        .unwrap();
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
            &paths,
            &validation,
            &plan,
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
                &crate::execution_storage::LocalExecutionStorage {
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
                "outputs/work/plans/example.json",
                task_path,
                "outputs/work/tasks/example/tasks/TASK-001.json",
                "outputs/work/executions/example/index.json",
            ] {
                let destination = root.join(relative);
                fs::create_dir_all(destination.parent().unwrap()).unwrap();
                fs::copy(fixture.join(relative), destination).unwrap();
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
    fn specification_transaction_marker_gates_plan_task_and_preflight_handoffs() {
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
        let plan_path = "outputs/work/plans/example.json";
        let task_path = "outputs/work/tasks/example/index.json";
        for relative in [
            plan_path,
            task_path,
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let storage = LocalHandoffStorage {
            project_root: root.clone(),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![],
        };
        let plan_request = json!({"summary":"Build tasks.","affected_ids":["GOAL-001"]});
        let handoff = storage
            .build_plan_to_task(plan_path, &plan_request)
            .unwrap();
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
            .build_task_to_plan(task_path, None, &return_request)
            .unwrap();
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
                .verify_task_to_plan(plan_path, task_path, None, &return_handoff)
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
                storage.build_plan_to_task(plan_path, &plan_request),
                storage.verify_plan_to_task(plan_path, &handoff),
                storage.build_task_to_execute(task_path, "TASK-001", &execute_request),
                storage.build_task_to_plan(task_path, None, &return_request),
                storage.build_preflight_return(
                    "execute_to_task",
                    task_path,
                    "TASK-001",
                    &preflight_request,
                ),
                storage.build_preflight_return(
                    "execute_to_plan",
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
            crate::transaction_storage::completion_marker(journal_raw),
        )
        .unwrap();
        assert_eq!(
            storage.verify_plan_to_task(plan_path, &handoff).unwrap()["status"],
            "valid"
        );
        storage
            .build_task_to_execute(task_path, "TASK-001", &execute_request)
            .unwrap();
        storage
            .build_task_to_plan(task_path, None, &return_request)
            .unwrap();
        storage
            .build_preflight_return("execute_to_task", task_path, "TASK-001", &preflight_request)
            .unwrap();
        storage
            .build_preflight_return("execute_to_plan", task_path, "TASK-001", &preflight_request)
            .unwrap();
    }

    #[test]
    fn closed_return_requires_completed_specification_transaction() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
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
            "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let storage = LocalHandoffStorage {
            project_root: root.clone(),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![],
        };
        let request = json!({"summary":"調整已確認範圍",
            "confirmed_approach":"保留既有介面", "requested_changes":["新增驗收條件"],
            "preserve":["既有功能"], "affected_ids":["GOAL-001","TASK-001"],
            "validation_requirements":["重新確認驗收條件"], "reason":"Clarify specification"});
        let journal =
            root.join("outputs/work/executions/example/.work-spec-update-SPEC-UPDATE-001.json");
        let journal_raw = b"{\"transaction\":\"test\"}\n";
        fs::write(&journal, journal_raw).unwrap();
        for direction in ["execute_to_task", "execute_to_plan"] {
            assert_eq!(
                storage
                    .build_closed_return(
                        direction,
                        "outputs/work/tasks/example/index.json",
                        "TASK-001",
                        "ATTEMPT-001",
                        &request,
                    )
                    .unwrap_err()
                    .reason_code,
                "spec_update_pending"
            );
        }
        fs::write(
            PathBuf::from(format!("{}.done", journal.display())),
            crate::transaction_storage::completion_marker(journal_raw),
        )
        .unwrap();
        for direction in ["execute_to_task", "execute_to_plan"] {
            let handoff = storage
                .build_closed_return(
                    direction,
                    "outputs/work/tasks/example/index.json",
                    "TASK-001",
                    "ATTEMPT-001",
                    &request,
                )
                .unwrap();
            assert_eq!(handoff["direction"], direction);
        }
        let plan_path = "outputs/work/plans/example.json";
        let task_path = "outputs/work/tasks/example/index.json";
        let handoff = storage
            .build_closed_return(
                "execute_to_task",
                task_path,
                "TASK-001",
                "ATTEMPT-001",
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
                        "ATTEMPT-001",
                        &invalid
                    )
                    .is_err(),
                "{invalid}"
            );
        }
        let attempt_path =
            root.join("outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json");
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
                        "ATTEMPT-001",
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
                        "ATTEMPT-001",
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
                    plan_path,
                    task_path,
                    "TASK-001",
                    "ATTEMPT-001",
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
                    "ATTEMPT-001",
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
                    "ATTEMPT-001",
                    &request
                )
                .unwrap_err()
                .reason_code,
            "handoff_execution_recovery_required"
        );
    }

    #[test]
    fn plan_to_task_uses_validated_plan_and_rejects_changed_source() {
        let root = std::env::temp_dir().join(format!(
            "work-handoff-plan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalHandoffStorage {
            project_root: root.clone(),
            skill_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work")),
            skill_configs: vec![],
        };
        let instructions = LocalHierarchyCatalog {
            skill_root: storage.skill_root.clone(),
        };
        let skills = LocalSkillCatalog { roots: vec![] };
        let paths = LocalPlanStorage {
            project_root: root.clone(),
        };
        let prepared = prepare_semantic(
            &instructions,
            &skills,
            &paths,
            &[],
            &json!({"requirement_id":"example","title":"Plan","summary":"Result",
                "goals":["Result"],"scope":["Source"],"deliverables":["Artifact"],
                "acceptance_criteria":["Observable"],
                "hierarchy_selection_request":{"decision":"general_only","selections":[]},
                "skill_selection_request":{"decision":"base_only","skills":[]},"references":[]}),
        )
        .unwrap();
        let plan_path = prepared["path"].as_str().unwrap();
        let file = root.join(plan_path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, render_plan_value(&prepared["plan"]).unwrap()).unwrap();
        let request = json!({"summary":"Build the tasks.","affected_ids":["GOAL-001"]});
        assert_eq!(
            storage
                .build_plan_to_task("missing.json", &json!({"summary":"No IDs"}))
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
        assert_eq!(
            storage
                .verify_plan_to_task("missing.json", &json!({"schema":"invalid"}))
                .unwrap_err()
                .reason_code,
            "invalid_handoff_direction"
        );
        let handoff = storage.build_plan_to_task(plan_path, &request).unwrap();
        assert_eq!(
            handoff["source"]["plan_sha256"],
            prepared["validation"]["plan_sha256"]
        );
        assert_eq!(handoff["source"]["stage"], "plan");
        assert_eq!(handoff["target"]["stage"], "task");
        assert_eq!(
            storage.verify_plan_to_task(plan_path, &handoff).unwrap()["status"],
            "valid"
        );
        for field in ["plan_sha256", "skill_selection_sha256"] {
            let mut forged = handoff.clone();
            forged["source"][field] = json!("0".repeat(64));
            assert_eq!(
                storage
                    .verify_plan_to_task(plan_path, &forged)
                    .unwrap_err()
                    .reason_code,
                "handoff_source_mismatch",
                "{field}"
            );
        }
        for (field, path) in [
            ("plan", "custom/example.json"),
            ("task", "custom/tasks/example/index.json"),
            ("execution", "custom/executions/example"),
        ] {
            let mut forged = handoff.clone();
            forged["artifacts"][field] = json!(path);
            assert_eq!(
                storage
                    .verify_plan_to_task(plan_path, &forged)
                    .unwrap_err()
                    .reason_code,
                "handoff_source_mismatch",
                "{field}"
            );
        }
        let mut wrong_requirement = handoff.clone();
        wrong_requirement["requirement_id"] = json!("other");
        wrong_requirement["artifacts"] = json!({"plan":"custom/other.json",
            "task":"custom/tasks/other/index.json","execution":"custom/executions/other"});
        assert_eq!(
            storage
                .verify_plan_to_task(plan_path, &wrong_requirement)
                .unwrap_err()
                .reason_code,
            "handoff_source_mismatch"
        );
        let mut unknown = handoff.clone();
        unknown["affected_ids"] = json!(["GOAL-999"]);
        assert_eq!(
            storage
                .verify_plan_to_task(plan_path, &unknown)
                .unwrap_err()
                .reason_code,
            "handoff_unknown_plan_ids"
        );
        assert!(
            storage
                .verify_plan_to_task("missing/plan.json", &handoff)
                .is_err()
        );
        assert_eq!(
            storage
                .build_plan_to_task(
                    plan_path,
                    &json!({"summary":"Build the tasks.","affected_ids":["GOAL-999"]})
                )
                .unwrap_err()
                .reason_code,
            "handoff_unknown_plan_ids"
        );
        for (ids, reason) in [
            (json!(["GOAL-001", "GOAL-001"]), "duplicate_array_value"),
            (json!([]), "invalid_string_array"),
            (json!(["PLAN"]), "invalid_handoff_identifier"),
        ] {
            assert_eq!(
                storage
                    .build_plan_to_task(
                        plan_path,
                        &json!({"summary":"Build the tasks.","affected_ids":ids}),
                    )
                    .unwrap_err()
                    .reason_code,
                reason
            );
        }
        for field in [
            "schema",
            "direction",
            "source",
            "target",
            "artifacts",
            "requirement_id",
        ] {
            let mut forged = request.clone();
            forged[field] = json!("override");
            assert_eq!(
                storage
                    .build_plan_to_task(plan_path, &forged)
                    .unwrap_err()
                    .reason_code,
                "invalid_object_fields",
                "{field}"
            );
        }
        for (invalid, reason) in [
            (
                json!({"summary":" ","affected_ids":["GOAL-001"]}),
                "empty_text_value",
            ),
            (
                json!({"affected_ids":["GOAL-001"]}),
                "invalid_object_fields",
            ),
        ] {
            assert_eq!(
                storage
                    .build_plan_to_task(plan_path, &invalid)
                    .unwrap_err()
                    .reason_code,
                reason
            );
        }
        let original = fs::read(&file).unwrap();
        assert!(
            storage
                .build_plan_to_task("../example.json", &request)
                .is_err()
        );
        assert_eq!(fs::read(&file).unwrap(), original);
        let mut draft_plan = prepared["plan"].clone();
        draft_plan["status"] = json!("draft");
        fs::write(&file, render_plan_value(&draft_plan).unwrap()).unwrap();
        assert_eq!(
            storage
                .build_plan_to_task(plan_path, &request)
                .unwrap_err()
                .reason_code,
            "invalid_plan_status"
        );
        let mut wrong_skill = prepared["plan"].clone();
        wrong_skill["skill_selection"]["selection_sha256"] = json!("0".repeat(64));
        fs::write(&file, render_plan_value(&wrong_skill).unwrap()).unwrap();
        assert_eq!(
            storage
                .build_plan_to_task(plan_path, &request)
                .unwrap_err()
                .reason_code,
            "skill_selection_fingerprint_mismatch"
        );
        assert!(storage.verify_plan_to_task(plan_path, &handoff).is_err());
        let noncanonical = serde_json::to_vec(&prepared["plan"]).unwrap();
        fs::write(&file, &noncanonical).unwrap();
        assert!(storage.build_plan_to_task(plan_path, &request).is_err());
        assert_eq!(fs::read(&file).unwrap(), noncanonical);
        fs::write(&file, &original).unwrap();
        let custom_path = "custom/plans/example.json";
        let mut custom_plan = prepared["plan"].clone();
        custom_plan["artifacts"] = json!({"plan":custom_path,
            "task":"custom/tasks/example/index.json","execution":"custom/executions/example"});
        let custom_file = root.join(custom_path);
        fs::create_dir_all(custom_file.parent().unwrap()).unwrap();
        fs::write(&custom_file, render_plan_value(&custom_plan).unwrap()).unwrap();
        assert_eq!(
            storage.build_plan_to_task(custom_path, &request).unwrap()["artifacts"],
            custom_plan["artifacts"]
        );
        let mut changed = prepared["plan"].clone();
        changed["summary"] = json!("Changed result");
        fs::write(&file, render_plan_value(&changed).unwrap()).unwrap();
        assert_eq!(
            storage
                .verify_plan_to_task(plan_path, &handoff)
                .unwrap_err()
                .reason_code,
            "handoff_source_mismatch"
        );
    }
}
