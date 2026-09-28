//! Plan path validation and exclusive publication adapter.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::plan::{PlanPathRepository, PlanPreparedOutput};
use work_feature::ports::ArtifactStore;
use work_operations::identifiers::RequirementId;

use crate::files::{LocalFiles, resolve_project_path};
use work_feature::plan::default_artifact_paths;

#[derive(Debug, Clone)]
pub struct LocalPlanStorage {
    pub project_root: PathBuf,
}

fn error(code: ExitCode, reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

impl LocalPlanStorage {
    fn path(&self, relative: &str) -> Result<PathBuf, WorkError> {
        Ok(resolve_project_path(&self.project_root, relative)?.1)
    }
}

impl PlanPathRepository for LocalPlanStorage {
    fn default_paths(&self, requirement_id: &RequirementId) -> Result<Value, WorkError> {
        let entries = default_artifact_paths(requirement_id);
        let mut paths = serde_json::Map::new();
        for (field, path) in entries {
            self.path(&path)?;
            paths.insert(field.into(), json!(path));
        }
        Ok(Value::Object(paths))
    }

    fn validate_paths(
        &self,
        requirement_id: &RequirementId,
        artifacts: &Value,
        actual_plan_path: &str,
        allow_task_index: bool,
    ) -> Result<(), WorkError> {
        let Some(object) = artifacts.as_object().filter(|object| {
            object.len() == 3
                && ["plan", "task", "execution"]
                    .iter()
                    .all(|field| object.contains_key(*field))
        }) else {
            return Err(error(
                ExitCode::Contract,
                "invalid_artifact_paths",
                "Artifacts must contain exactly plan, task, and execution paths.",
                json!({}),
            ));
        };
        let mut resolved = Vec::new();
        for field in ["plan", "task", "execution"] {
            let path = object[field].as_str().ok_or_else(|| {
                error(
                    ExitCode::Contract,
                    "invalid_artifact_path",
                    "Each artifact path must be a string.",
                    json!({"field": field}),
                )
            })?;
            let (normalized, absolute) = resolve_project_path(&self.project_root, path)?;
            resolved.push((field, normalized, absolute));
        }
        let id = requirement_id.as_str();
        let plan = Path::new(&resolved[0].1);
        if plan.extension().and_then(|ext| ext.to_str()) != Some("json")
            || plan.file_stem().and_then(|stem| stem.to_str()) != Some(id)
        {
            return Err(error(
                ExitCode::Contract,
                "plan_path_requirement_mismatch",
                "The Plan artifact path must end with the requirement ID and .json.",
                json!({}),
            ));
        }
        let task = Path::new(&resolved[1].1);
        let task_name = task.file_name().and_then(|name| name.to_str());
        if (!matches!(task_name, Some("task.json" | "index.json"))
            || (!allow_task_index && task_name == Some("index.json")))
            || task
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                != Some(id)
        {
            return Err(error(
                ExitCode::Contract,
                "task_path_requirement_mismatch",
                "The TASK artifact path has the wrong requirement directory or filename.",
                json!({}),
            ));
        }
        if Path::new(&resolved[2].1)
            .file_name()
            .and_then(|name| name.to_str())
            != Some(id)
        {
            return Err(error(
                ExitCode::Contract,
                "execution_path_requirement_mismatch",
                "The execution artifact path must end with the requirement ID.",
                json!({}),
            ));
        }
        let normalized_actual = resolve_project_path(&self.project_root, actual_plan_path)?.0;
        if normalized_actual != resolved[0].1 {
            return Err(error(
                ExitCode::Contract,
                "plan_artifact_path_mismatch",
                "The Plan contract path does not match the validated artifact path.",
                json!({"expected": resolved[0].1, "actual": normalized_actual}),
            ));
        }
        for first in 0..resolved.len() {
            for second in first + 1..resolved.len() {
                if resolved[first].2.to_string_lossy().to_lowercase()
                    == resolved[second].2.to_string_lossy().to_lowercase()
                {
                    return Err(error(
                        ExitCode::Contract,
                        "artifact_path_alias",
                        "Two artifact paths resolve to the same portable path identity.",
                        json!({"first": resolved[first].0, "second": resolved[second].0}),
                    ));
                }
            }
        }
        Ok(())
    }

    fn exists(&self, relative_path: &str) -> Result<bool, WorkError> {
        Ok(self.path(relative_path)?.exists())
    }

    fn create_exclusive(&self, relative_path: &str, content: &[u8]) -> Result<(), WorkError> {
        let path = self.path(relative_path)?;
        if path.exists() {
            return Err(error(
                ExitCode::WorkflowState,
                "plan_already_exists",
                "The Plan target already exists; plan create never overwrites it.",
                json!({"path": relative_path}),
            ));
        }
        fs::create_dir_all(path.parent().expect("plan path has parent")).map_err(|_| {
            error(
                ExitCode::IoFailure,
                "plan_create_failed",
                "The canonical Plan could not be created.",
                json!({"path": relative_path}),
            )
        })?;
        LocalFiles.create_new(&path, content).map_err(|failure| {
            if path.exists() && failure.reason_code == "file_write_failed" {
                error(
                    ExitCode::WorkflowState,
                    "plan_already_exists",
                    "The Plan target already exists; plan create never overwrites it.",
                    json!({"path": relative_path}),
                )
            } else {
                error(
                    ExitCode::IoFailure,
                    "plan_create_failed",
                    "The canonical Plan could not be created.",
                    json!({"path": relative_path}),
                )
            }
        })
    }

    fn read(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        LocalFiles.read_raw(&self.path(relative_path)?)
    }
}

impl PlanPreparedOutput for LocalPlanStorage {
    fn create_prepared_output(&self, path: &str, content: &[u8]) -> Result<(), WorkError> {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|failure| {
                if failure.kind() == std::io::ErrorKind::AlreadyExists {
                    error(
                        ExitCode::WorkflowState,
                        "plan_prepare_output_exists",
                        "The prepared Plan output file already exists.",
                        json!({"path": path}),
                    )
                } else {
                    error(
                        ExitCode::IoFailure,
                        "plan_prepare_output_failed",
                        "The prepared Plan output file could not be created.",
                        json!({"path": path}),
                    )
                }
            })?;
        file.write_all(content).map_err(|_| {
            error(
                ExitCode::IoFailure,
                "plan_prepare_output_failed",
                "The prepared Plan output file could not be created.",
                json!({"path": path}),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hierarchy_catalog::LocalHierarchyCatalog;
    use crate::skill_catalog::LocalSkillCatalog;
    use work_feature::plan::{
        PlanValidationInput, create_plan, prepare_semantic, prepare_semantic_to_file,
        validate_plan, validate_plan_file,
    };

    #[test]
    fn artifact_paths_keep_collection_defaults_and_reject_legacy_layouts() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "work-plan-artifact-paths-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalPlanStorage { project_root: root };
        let id: RequirementId = "feature-1".parse().unwrap();
        let defaults = storage.default_paths(&id).unwrap();
        assert_eq!(defaults["plan"], "outputs/work/plans/feature-1.json");
        assert_eq!(defaults["task"], "outputs/work/tasks/feature-1/index.json");
        assert_eq!(defaults["execution"], "outputs/work/executions/feature-1");
        storage
            .validate_paths(&id, &defaults, defaults["plan"].as_str().unwrap(), true)
            .unwrap();

        let mut legacy = defaults.clone();
        legacy["task"] = json!("custom/feature-1/task.json");
        storage
            .validate_paths(&id, &legacy, defaults["plan"].as_str().unwrap(), false)
            .unwrap();
        for task in [
            "outputs/work/tasks/feature-1/task.md",
            "outputs/work/tasks/feature-1.json",
            "custom/feature-1.json",
            "outputs/work/tasks/other/task.json",
            "outputs/work/tasks/feature-1/other.json",
            "outputs/work/tasks/feature-1/drafts/task.json",
        ] {
            let mut invalid = defaults.clone();
            invalid["task"] = json!(task);
            assert_eq!(
                storage
                    .validate_paths(&id, &invalid, defaults["plan"].as_str().unwrap(), false)
                    .unwrap_err()
                    .reason_code,
                "task_path_requirement_mismatch",
                "{task}"
            );
        }
        let mut invalid = defaults.clone();
        invalid["plan"] = json!("outputs/work/plans/feature-1.md");
        assert_eq!(
            storage
                .validate_paths(&id, &invalid, "outputs/work/plans/feature-1.md", false)
                .unwrap_err()
                .reason_code,
            "plan_path_requirement_mismatch"
        );
        let alias = json!({
            "plan": "outputs/task/task.json",
            "task": "outputs/task/task.json",
            "execution": "outputs/executions/task"
        });
        let task_id: RequirementId = "task".parse().unwrap();
        assert_eq!(
            storage
                .validate_paths(&task_id, &alias, "outputs/task/task.json", false)
                .unwrap_err()
                .reason_code,
            "artifact_path_alias"
        );
    }

    #[test]
    fn semantic_prepare_matches_python_plan_sha() {
        let work = LocalHierarchyCatalog {
            skill_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work")),
        };
        let skills = LocalSkillCatalog { roots: vec![] };
        let storage = LocalPlanStorage {
            project_root: std::env::temp_dir(),
        };
        let request = json!({
            "requirement_id": "issue55-plan-parity", "title": "Parity Plan", "summary": "Check Rust parity.",
            "goals": ["Ship one result."], "scope": ["Create result."], "deliverables": ["Result artifact."],
            "acceptance_criteria": ["Artifact is verified."],
            "hierarchy_selection_request": {"decision": "general_only", "selections": []},
            "skill_selection_request": {"decision": "base_only", "skills": []}, "references": []
        });
        let prepared = prepare_semantic(&work, &skills, &storage, &[], &request).unwrap();
        assert!(
            !storage
                .project_root
                .join(prepared["path"].as_str().unwrap())
                .exists()
        );
        assert_eq!(
            prepared["validation"]["plan_sha256"],
            "d43c84b6b59aa0cdd34f0b0ce21fd9d7b14d013f4c794871f41ba03309617062"
        );
        assert_eq!(prepared["validation"]["item_count"], 4);
        assert_eq!(prepared["validation"]["schema"], "work-plan-validation/v1");
        assert_eq!(
            prepared["validation"]["hierarchy_selection_sha256"],
            prepared["plan"]["hierarchy_selection"]["selection_sha256"]
        );
        assert_eq!(
            prepared["validation"]["work_instructions_sha256"],
            prepared["plan"]["work_instruction_selection"]["instructions_sha256"]
        );
        assert!(prepared["validation"].get("rules_sha256").is_none());
        assert_eq!(
            prepared["path"],
            "outputs/work/plans/issue55-plan-parity.json"
        );
        let mut stale = prepared["plan"].clone();
        stale["work_instruction_selection"]["instructions_sha256"] = json!("0".repeat(64));
        let raw = work_operations::plan::render_plan_value(&stale).unwrap();
        assert_eq!(
            validate_plan(
                &work,
                &skills,
                &storage,
                &[],
                &stale,
                PlanValidationInput {
                    raw: &raw,
                    actual_plan_path: prepared["path"].as_str().unwrap(),
                    allow_task_index: true
                }
            )
            .unwrap_err()
            .reason_code,
            "work_instructions_fingerprint_mismatch"
        );
        stale["work_instruction_selection"] =
            prepared["plan"]["work_instruction_selection"].clone();
        stale["work_instruction_selection"]["sources"]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        let raw = work_operations::plan::render_plan_value(&stale).unwrap();
        assert_eq!(
            validate_plan(
                &work,
                &skills,
                &storage,
                &[],
                &stale,
                PlanValidationInput {
                    raw: &raw,
                    actual_plan_path: prepared["path"].as_str().unwrap(),
                    allow_task_index: true
                }
            )
            .unwrap_err()
            .reason_code,
            "work_instruction_selection_sources_mismatch"
        );
        let mut legacy = prepared["plan"].clone();
        let selection = legacy
            .as_object_mut()
            .unwrap()
            .remove("work_instruction_selection")
            .unwrap();
        legacy["rule_selection"] = selection;
        let raw = work_operations::plan::render_plan_value(&legacy).unwrap();
        let error = validate_plan(
            &work,
            &skills,
            &storage,
            &[],
            &legacy,
            PlanValidationInput {
                raw: &raw,
                actual_plan_path: prepared["path"].as_str().unwrap(),
                allow_task_index: true,
            },
        )
        .unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(
            error.details["missing"],
            json!(["work_instruction_selection"])
        );
        assert_eq!(error.details["unknown"], json!(["rule_selection"]));
        let mut legacy = prepared["plan"].clone();
        legacy["work_instruction_selection"]["sources"][0]["layer"] = json!("project");
        let raw = work_operations::plan::render_plan_value(&legacy).unwrap();
        let error = validate_plan(
            &work,
            &skills,
            &storage,
            &[],
            &legacy,
            PlanValidationInput {
                raw: &raw,
                actual_plan_path: prepared["path"].as_str().unwrap(),
                allow_task_index: true,
            },
        )
        .unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(error.details["unknown"], json!(["layer"]));
    }

    #[test]
    fn semantic_prepare_rejects_legacy_fields_and_invalid_selections_without_writes() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let project_root =
            std::env::temp_dir().join(format!("work-plan-invalid-{}-{nonce}", std::process::id()));
        let work = LocalHierarchyCatalog {
            skill_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work")),
        };
        let skills = LocalSkillCatalog { roots: vec![] };
        let storage = LocalPlanStorage { project_root };
        let request = json!({
            "requirement_id": "example", "title": "Plan", "summary": "Summary.",
            "goals": ["Goal."], "scope": ["Scope."], "deliverables": ["Delivery."],
            "acceptance_criteria": ["Accepted."],
            "hierarchy_selection_request": {"decision": "general_only", "selections": []},
            "skill_selection_request": {"decision": "base_only", "skills": []},
            "references": []
        });
        let mut cases = Vec::new();
        let mut missing = request.clone();
        missing.as_object_mut().unwrap().remove("goals");
        cases.push(missing);
        let mut legacy = request.clone();
        legacy["content"] = json!({"title": "Plan"});
        cases.push(legacy);
        let mut status_override = request.clone();
        status_override["status"] = json!("confirmed");
        cases.push(status_override);
        let mut wrong_hierarchy = request.clone();
        wrong_hierarchy["hierarchy_selection_request"]["decision"] = json!("instruction_paths");
        cases.push(wrong_hierarchy);
        let mut wrong_skill = request.clone();
        wrong_skill["skill_selection_request"]["decision"] = json!("external_skills");
        cases.push(wrong_skill);
        let mut unknown_reference = request;
        unknown_reference["references"] = json!(["unknown.reference"]);
        cases.push(unknown_reference);
        for candidate in cases {
            assert!(prepare_semantic(&work, &skills, &storage, &[], &candidate).is_err());
            assert!(!storage.project_root.exists());
        }
    }

    #[test]
    fn exclusive_create_persists_prepared_plan() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let project_root =
            std::env::temp_dir().join(format!("work-plan-create-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&project_root).unwrap();
        let work = LocalHierarchyCatalog {
            skill_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work")),
        };
        let skills = LocalSkillCatalog { roots: vec![] };
        let storage = LocalPlanStorage { project_root };
        let request = json!({
            "requirement_id": format!("example-{}", std::process::id()), "title": "Plan", "summary": "Summary.",
            "goals": ["Goal."], "scope": ["Scope."], "deliverables": ["Delivery."], "acceptance_criteria": ["Accepted."],
            "hierarchy_selection_request": {"decision": "general_only", "selections": []},
            "skill_selection_request": {"decision": "base_only", "skills": []}, "references": []
        });
        let prepared = prepare_semantic(&work, &skills, &storage, &[], &request).unwrap();
        let output = storage.project_root.join("candidate.json");
        let output_path = output.to_str().unwrap();
        assert_eq!(
            prepare_semantic_to_file(
                &work,
                &skills,
                &storage,
                &storage,
                &[],
                &request,
                output_path
            )
            .unwrap()["path"],
            prepared["path"]
        );
        assert_eq!(
            fs::read(&output).unwrap(),
            work_operations::plan::render_plan_value(&prepared["plan"]).unwrap()
        );
        assert_eq!(
            prepare_semantic_to_file(
                &work,
                &skills,
                &storage,
                &storage,
                &[],
                &request,
                output_path
            )
            .unwrap_err()
            .reason_code,
            "plan_prepare_output_exists"
        );
        let path = prepared["path"].as_str().unwrap();
        let created = create_plan(&work, &skills, &storage, &[], &prepared["plan"], path).unwrap();
        assert_eq!(created["schema"], "work-plan-create/v1");
        assert_eq!(
            validate_plan_file(&work, &skills, &storage, &[], path).unwrap()["plan_sha256"],
            created["plan_sha256"]
        );
        assert_eq!(
            created["plan_sha256"],
            prepared["validation"]["plan_sha256"]
        );
        assert_eq!(
            create_plan(&work, &skills, &storage, &[], &prepared["plan"], path)
                .unwrap_err()
                .reason_code,
            "plan_already_exists"
        );
        assert_eq!(
            fs::read(storage.project_root.join(path)).unwrap(),
            work_operations::plan::render_plan_value(&prepared["plan"]).unwrap()
        );
    }

    #[test]
    fn handoff_uses_plan_path_repository_for_project_identity() {
        let root = std::env::temp_dir().join(format!(
            "work-handoff-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalPlanStorage { project_root: root };
        let mut contract = json!({"schema":"work-handoff/v1","marker":"WORK-HANDOFF","direction":"plan_to_task","requirement_id":"example","artifacts":{"plan":"outputs/work/plans/example.json","task":"outputs/work/tasks/example/index.json","execution":"outputs/work/executions/example"},"source":{"stage":"plan","plan_sha256":"a".repeat(64),"skill_selection_sha256":"d".repeat(64)},"target":{"stage":"task"},"summary":"Continue.","affected_ids":["GOAL-001"]});
        assert_eq!(
            work_feature::handoff::validate_handoff(&storage, &contract).unwrap()["status"],
            "valid"
        );
        contract["artifacts"]["plan"] = json!("./outputs/work/plans/example.json");
        assert_eq!(
            work_feature::handoff::validate_handoff(&storage, &contract)
                .unwrap_err()
                .reason_code,
            "noncanonical_artifact_paths"
        );
    }
}
