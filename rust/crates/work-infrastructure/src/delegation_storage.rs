//! Delegation contexts derived from validated formal artifacts.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_feature::delegation::DelegationSourceRepository;
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;

use crate::files::{LocalFiles, resolve_project_path};
use crate::plan_storage::LocalPlanStorage;
use crate::skill_catalog::SkillRootConfig;

#[derive(Debug, Clone)]
pub struct LocalDelegationStorage {
    pub project_root: PathBuf,
    pub skill_root: PathBuf,
    pub skill_configs: Vec<SkillRootConfig>,
}

fn boundary(message: &'static str) -> WorkError {
    WorkError::new(
        ExitCode::Contract,
        "delegation_boundary_mismatch",
        message,
        json!({}),
    )
}

impl LocalDelegationStorage {
    pub fn validate_task_skill(&self, envelope: &Value, sender: &str) -> Result<Value, WorkError> {
        let project_root = self
            .project_root
            .canonicalize()
            .map_err(|_| boundary("The project root cannot be resolved."))?;
        let skill_root = self
            .skill_root
            .canonicalize()
            .map_err(|_| boundary("The Work skill root cannot be resolved."))?;
        work_feature::delegation::validate_task_skill(
            envelope,
            sender,
            &project_root.to_string_lossy(),
            &skill_root.to_string_lossy(),
        )
    }

    pub fn build_task_skill(&self, request: &Value) -> Result<Value, WorkError> {
        work_feature::delegation::build_task_skill(self, request)
    }

    pub fn validate_artifact_editor(
        &self,
        envelope: &Value,
        sender: &str,
    ) -> Result<Value, WorkError> {
        let project_root = self
            .project_root
            .canonicalize()
            .map_err(|_| boundary("The project root cannot be resolved."))?;
        let skill_root = self
            .skill_root
            .canonicalize()
            .map_err(|_| boundary("The Work skill root cannot be resolved."))?;
        work_feature::delegation::validate_artifact_editor(
            &LocalPlanStorage {
                project_root: self.project_root.clone(),
            },
            envelope,
            sender,
            &project_root.to_string_lossy(),
            &skill_root.to_string_lossy(),
        )
    }

    pub fn build_artifact_editor(&self, request: &Value) -> Result<Value, WorkError> {
        work_feature::delegation::build_artifact_editor(
            self,
            &LocalPlanStorage {
                project_root: self.project_root.clone(),
            },
            request,
        )
    }

    pub fn validate_progress_saver(
        &self,
        envelope: &Value,
        sender: &str,
    ) -> Result<Value, WorkError> {
        let project_root = self
            .project_root
            .canonicalize()
            .map_err(|_| boundary("The project root cannot be resolved."))?;
        let skill_root = self
            .skill_root
            .canonicalize()
            .map_err(|_| boundary("The Work skill root cannot be resolved."))?;
        work_feature::delegation::validate_progress_saver(
            envelope,
            sender,
            &project_root.to_string_lossy(),
            &skill_root.to_string_lossy(),
        )
    }

    pub fn build_progress_saver(&self, request: &Value) -> Result<Value, WorkError> {
        work_feature::delegation::build_progress_saver(self, request)
    }

    pub fn validate_execute_role(
        &self,
        envelope: &Value,
        sender: &str,
    ) -> Result<Value, WorkError> {
        let project_root = self
            .project_root
            .canonicalize()
            .map_err(|_| boundary("The project root cannot be resolved."))?;
        let skill_root = self
            .skill_root
            .canonicalize()
            .map_err(|_| boundary("The Work skill root cannot be resolved."))?;
        work_feature::delegation::validate_execute_role(
            envelope,
            sender,
            &project_root.to_string_lossy(),
            &skill_root.to_string_lossy(),
        )
    }

    pub fn build_execute_role(&self, request: &Value) -> Result<Value, WorkError> {
        work_feature::delegation::build_execute_role(self, request)
    }

    pub fn validate_plan_role(
        &self,
        envelope: &Value,
        role: &str,
        sender: &str,
    ) -> Result<Value, WorkError> {
        work_feature::delegation::validate_plan_role_name(role)?;
        let project_root = self
            .project_root
            .canonicalize()
            .map_err(|_| boundary("The project root cannot be resolved."))?;
        let skill_root = self
            .skill_root
            .canonicalize()
            .map_err(|_| boundary("The Work skill root cannot be resolved."))?;
        work_feature::delegation::validate_plan_role(
            envelope,
            role,
            sender,
            &project_root.to_string_lossy(),
            &skill_root.to_string_lossy(),
        )
    }

    pub fn build_plan_role(&self, request: &Value) -> Result<Value, WorkError> {
        work_feature::delegation::build_plan_role(self, request)
    }
}

impl DelegationSourceRepository for LocalDelegationStorage {
    fn resolve_project_path(&self, relative: &str) -> Result<(String, PathBuf), WorkError> {
        resolve_project_path(&self.project_root, relative)
    }

    fn read_raw(&self, path: &Path) -> Result<Vec<u8>, WorkError> {
        LocalFiles.read_raw(path)
    }

    fn canonical_project_root(&self) -> Result<String, WorkError> {
        self.project_root
            .canonicalize()
            .map(|root| root.to_string_lossy().into_owned())
            .map_err(|_| boundary("The project root cannot be resolved."))
    }

    fn canonical_skill_root(&self) -> Result<String, WorkError> {
        self.skill_root
            .canonicalize()
            .map(|root| root.to_string_lossy().into_owned())
            .map_err(|_| boundary("The Work skill root cannot be resolved."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use work_operations::canonical::parse_json_contract;
    use work_operations::delegation::{build_envelope, validation_result};

    fn python_expected(role: &str, storage: &LocalDelegationStorage) -> Value {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let raw = std::fs::read(
            repo.join("crates/work-infrastructure/fixtures/delegation-role")
                .join(format!("{role}-expected.json")),
        )
        .unwrap();
        let mut expected: Value = serde_json::from_slice(&raw).unwrap();
        expected["project_root"] = json!(storage.project_root.canonicalize().unwrap());
        expected["skill_root"] = json!(storage.skill_root.canonicalize().unwrap());
        expected
    }

    #[test]
    fn plan_and_task_coordinator_contexts_bind_formal_plan() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let storage = LocalDelegationStorage {
            project_root: repo.join("crates/work-infrastructure/fixtures/task-diagnostics"),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![],
        };
        let plan_path = "outputs/work/plans/example.json";
        let raw = LocalFiles
            .read_raw(&storage.project_root.join(plan_path))
            .unwrap();
        let plan = parse_json_contract(&raw).unwrap();
        for (role, mode) in [("plan", "plan"), ("task-coordinator", "task")] {
            let envelope = storage
                .build_plan_role(&json!({
                    "schema":"work-delegation-build-request/v1", "role":role,
                    "request":"Coordinate confirmed work.", "source_plan_path":plan_path,
                }))
                .unwrap();
            assert_eq!(envelope, python_expected(role, &storage));
            assert_eq!(envelope["role"], role);
            assert_eq!(envelope["mode"], mode);
            assert_eq!(
                envelope["context"]["hierarchy_selection"],
                plan["hierarchy_selection"]
            );
            assert_eq!(
                envelope["context"]["skill_selection"],
                plan["skill_selection"]
            );
            assert_eq!(
                envelope["context"]["source_plan"],
                if role == "task-coordinator" {
                    plan.clone()
                } else {
                    Value::Null
                }
            );
            assert_eq!(
                envelope["project_root"],
                storage
                    .project_root
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref()
            );
            assert_eq!(
                storage
                    .validate_plan_role(&envelope, role, "parent")
                    .unwrap(),
                validation_result(role, mode, false)
            );
            let mut drifted = envelope.clone();
            drifted["context"]["hierarchy_selection"]["selection_sha256"] = json!("0".repeat(64));
            assert_eq!(
                storage
                    .validate_plan_role(&drifted, role, "parent")
                    .unwrap_err()
                    .reason_code,
                "delegation_boundary_mismatch"
            );
        }
    }

    #[test]
    fn execute_context_uses_formal_task_and_execution_identity() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let storage = LocalDelegationStorage {
            project_root: repo.join("crates/work-infrastructure/fixtures/task-diagnostics"),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![],
        };
        let envelope = storage
            .build_execute_role(&json!({
                "schema":"work-delegation-build-request/v1","role":"execute",
                "request":"Execute selected TASK.",
                "source_plan_path":"outputs/work/plans/example.json","task_id":"TASK-001",
            }))
            .unwrap();
        assert_eq!(envelope, python_expected("execute", &storage));
        assert_eq!(envelope["mode"], "execute");
        assert_eq!(envelope["context"]["target_task"]["id"], "TASK-001");
        assert_eq!(
            envelope["context"]["execute_skill_selection"]["decision"],
            "base_only"
        );
        assert_eq!(
            envelope["context"]["execute_skill_selection"]["selection_sha256"],
            "a09357ef9f22c43dca16b489da61b287838bf64963a5956bf52ae0327e04a959"
        );
        assert_eq!(
            storage.validate_execute_role(&envelope, "parent").unwrap(),
            validation_result("execute", "execute", false)
        );
        let mut drifted = envelope.clone();
        drifted["context"]["hierarchy_selection_sha256"] = json!("0".repeat(64));
        assert_eq!(
            storage
                .validate_execute_role(&drifted, "parent")
                .unwrap_err()
                .reason_code,
            "delegation_boundary_mismatch"
        );
        let mut wrong_skill = envelope.clone();
        wrong_skill["context"]["target_task"]["skill_id"] = json!("unconfirmed");
        assert_eq!(
            storage
                .validate_execute_role(&wrong_skill, "parent")
                .unwrap_err()
                .reason_code,
            "delegation_boundary_mismatch"
        );
    }

    #[test]
    fn progress_saver_context_matches_python_reference() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let storage = LocalDelegationStorage {
            project_root: repo.join("crates/work-infrastructure/fixtures/delegation-role"),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![],
        };
        let progress: Value = serde_json::from_slice(
            &std::fs::read(
                storage
                    .project_root
                    .join("outputs/work/progress/example/task/progress.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let mut content = progress.as_object().unwrap().clone();
        for field in ["schema", "requirement_id", "mode", "revision", "status"] {
            content.remove(field);
        }
        let envelope = storage
            .build_progress_saver(&json!({
                "schema":"work-delegation-build-request/v1","role":"progress-saver",
                "mode":"task","request":"Save discussion progress.",
                "source_progress_path":"outputs/work/progress/example/task/progress.json",
                "content":content,"continuation_point":"Continue discussion",
            }))
            .unwrap();
        assert_eq!(envelope, python_expected("progress-saver", &storage));
        assert_eq!(
            storage
                .validate_progress_saver(&envelope, "parent")
                .unwrap(),
            validation_result("progress-saver", "task", false)
        );
        let mut wrong = envelope.clone();
        wrong["context"]["expected_revision"] = json!(true);
        assert_eq!(
            storage
                .validate_progress_saver(&wrong, "parent")
                .unwrap_err()
                .reason_code,
            "delegation_boundary_mismatch"
        );
        let mut foreign = envelope.clone();
        foreign["context"]["content"]["mode"] = json!("execute");
        assert_eq!(
            storage
                .validate_progress_saver(&foreign, "parent")
                .unwrap_err()
                .reason_code,
            "delegation_boundary_mismatch"
        );
    }

    #[test]
    fn artifact_editor_context_matches_python_legacy_plan_reference() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let storage = LocalDelegationStorage {
            project_root: repo
                .join("crates/work-infrastructure/fixtures/delegation-role/legacy-plan"),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![],
        };
        let envelope = storage
            .build_artifact_editor(&json!({
                "schema":"work-delegation-build-request/v1","role":"artifact-editor",
                "mode":"execute","request":"Revise confirmed artifact.",
                "source_plan_path":"outputs/work/plans/example.json",
                "confirmed_request":{"reason":"Reviewed"},"decisions":["Confirmed revision"],
                "affected_task_ids":["TASK-001"],"continuation_point":"Return to Execute",
            }))
            .unwrap();
        assert_eq!(envelope, python_expected("artifact-editor", &storage));
        assert_eq!(
            storage
                .validate_artifact_editor(&envelope, "parent")
                .unwrap(),
            validation_result("artifact-editor", "execute", false)
        );
    }

    #[test]
    fn artifact_editor_accepts_formal_task_collection_path() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let storage = LocalDelegationStorage {
            project_root: repo.join("crates/work-infrastructure/fixtures/task-diagnostics"),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![],
        };
        let envelope = storage
            .build_artifact_editor(&json!({
                "schema":"work-delegation-build-request/v1","role":"artifact-editor",
                "mode":"execute","request":"Revise confirmed artifact.",
                "source_plan_path":"outputs/work/plans/example.json",
                "confirmed_request":{"reason":"Reviewed"},"decisions":["Confirmed revision"],
                "affected_task_ids":["TASK-001"],"continuation_point":"Return to Execute",
            }))
            .unwrap();
        assert_eq!(
            envelope,
            python_expected("artifact-editor-formal", &storage)
        );
        assert_eq!(
            envelope["context"]["artifacts"]["task"],
            "outputs/work/tasks/example/index.json"
        );
        assert_eq!(
            storage
                .validate_artifact_editor(&envelope, "parent")
                .unwrap(),
            validation_result("artifact-editor", "execute", false)
        );
    }

    #[test]
    fn task_skill_context_matches_python_formal_skill_reference() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let storage = LocalDelegationStorage {
            project_root: repo
                .join("crates/work-infrastructure/fixtures/delegation-role/task-skill"),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![],
        };
        let envelope = storage
            .build_task_skill(&json!({
                "schema":"work-delegation-build-request/v1","role":"task-skill",
                "request":"Refine selected TASK.",
                "source_plan_path":"outputs/work/plans/example.json","task_id":"TASK-001",
            }))
            .unwrap();
        assert_eq!(envelope, python_expected("task-skill", &storage));
        assert_eq!(
            storage
                .validate_task_skill(&envelope, "task-coordinator")
                .unwrap(),
            validation_result("task-skill", "task", false)
        );
        let mut wrong = envelope.clone();
        wrong["context"]["task_boundary"]["skill_id"] = json!("another");
        assert_eq!(
            storage
                .validate_task_skill(&wrong, "task-coordinator")
                .unwrap_err()
                .reason_code,
            "delegation_boundary_mismatch"
        );
    }

    #[test]
    fn plan_and_task_discussion_resume_validate_identity() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let storage = LocalDelegationStorage {
            project_root: repo.join("crates/work-infrastructure/fixtures/delegation-role"),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![],
        };
        let raw = std::fs::read(
            storage
                .project_root
                .join("outputs/work/progress/example/task/progress.json"),
        )
        .unwrap();
        let task_progress: Value = serde_json::from_slice(&raw).unwrap();
        for (role, mode) in [("plan", "plan"), ("task-coordinator", "task")] {
            let mut progress = task_progress.clone();
            progress["mode"] = json!(mode);
            if role == "plan" {
                progress["current_task_id"] = Value::Null;
            }
            let envelope = build_envelope(
                role,
                mode,
                "resume example",
                &storage
                    .project_root
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy(),
                &storage.skill_root.canonicalize().unwrap().to_string_lossy(),
                &json!({"saved_progress":progress}),
            )
            .unwrap();
            assert_eq!(
                storage
                    .validate_plan_role(&envelope, role, "parent")
                    .unwrap(),
                validation_result(role, mode, true)
            );
            let mut wrong = envelope.clone();
            wrong["request"] = json!("resume other");
            assert_eq!(
                storage
                    .validate_plan_role(&wrong, role, "parent")
                    .unwrap_err()
                    .reason_code,
                "delegation_boundary_mismatch"
            );
        }
    }

    #[test]
    fn role_context_missing_fields_match_python_rejection_messages() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        for (role, relative, required, message) in [
            (
                "plan",
                "crates/work-infrastructure/fixtures/task-diagnostics",
                "hierarchy_selection",
                "workflow_context must contain exactly the required and optional fields.",
            ),
            (
                "task-coordinator",
                "crates/work-infrastructure/fixtures/task-diagnostics",
                "source_plan",
                "workflow_context must contain exactly the required and optional fields.",
            ),
            (
                "execute",
                "crates/work-infrastructure/fixtures/task-diagnostics",
                "target_task",
                "workflow_context must contain exactly the required and optional fields.",
            ),
            (
                "task-skill",
                "crates/work-infrastructure/fixtures/delegation-role/task-skill",
                "task_boundary",
                "task_skill_context must contain exactly the required and optional fields.",
            ),
            (
                "artifact-editor",
                "crates/work-infrastructure/fixtures/delegation-role/legacy-plan",
                "confirmed_request",
                "maintenance_context must contain exactly the required and optional fields.",
            ),
            (
                "progress-saver",
                "crates/work-infrastructure/fixtures/delegation-role",
                "content",
                "maintenance_context must contain exactly the required and optional fields.",
            ),
        ] {
            let storage = LocalDelegationStorage {
                project_root: repo.join(relative),
                skill_root: repo.join("../skills/work"),
                skill_configs: vec![],
            };
            let mut envelope = python_expected(role, &storage);
            envelope["context"]
                .as_object_mut()
                .unwrap()
                .remove(required);
            let outcome = match role {
                "plan" | "task-coordinator" => {
                    storage.validate_plan_role(&envelope, role, "parent")
                }
                "execute" => storage.validate_execute_role(&envelope, "parent"),
                "task-skill" => storage.validate_task_skill(&envelope, "task-coordinator"),
                "artifact-editor" => storage.validate_artifact_editor(&envelope, "parent"),
                "progress-saver" => storage.validate_progress_saver(&envelope, "parent"),
                _ => unreachable!(),
            };
            let error = outcome.unwrap_err();
            assert_eq!(error.exit_code, ExitCode::Contract, "{role}");
            assert_eq!(error.reason_code, "delegation_boundary_mismatch", "{role}");
            assert_eq!(error.message, message, "{role}");
        }
    }

    #[test]
    fn all_role_fixtures_validate_without_mutation_or_authority() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        for (role, relative) in [
            (
                "plan",
                "crates/work-infrastructure/fixtures/task-diagnostics",
            ),
            (
                "task-coordinator",
                "crates/work-infrastructure/fixtures/task-diagnostics",
            ),
            (
                "execute",
                "crates/work-infrastructure/fixtures/task-diagnostics",
            ),
            (
                "task-skill",
                "crates/work-infrastructure/fixtures/delegation-role/task-skill",
            ),
            (
                "artifact-editor",
                "crates/work-infrastructure/fixtures/delegation-role/legacy-plan",
            ),
            (
                "progress-saver",
                "crates/work-infrastructure/fixtures/delegation-role",
            ),
        ] {
            let storage = LocalDelegationStorage {
                project_root: repo.join(relative),
                skill_root: repo.join("../skills/work"),
                skill_configs: vec![],
            };
            let envelope = python_expected(role, &storage);
            let before = envelope.clone();
            let result = match role {
                "plan" | "task-coordinator" => {
                    storage.validate_plan_role(&envelope, role, "parent")
                }
                "execute" => storage.validate_execute_role(&envelope, "parent"),
                "task-skill" => storage.validate_task_skill(&envelope, "task-coordinator"),
                "artifact-editor" => storage.validate_artifact_editor(&envelope, "parent"),
                "progress-saver" => storage.validate_progress_saver(&envelope, "parent"),
                _ => unreachable!(),
            }
            .unwrap();
            assert_eq!(result["status"], "valid", "{role}");
            assert_eq!(result["source_validation"], "not_checked", "{role}");
            assert_eq!(result["grants_authorization"], false, "{role}");
            assert_eq!(envelope, before, "{role}");
        }
    }

    #[test]
    fn saved_progress_cannot_hide_execute_or_mix_current_context() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        for (role, relative) in [
            (
                "plan",
                "crates/work-infrastructure/fixtures/task-diagnostics",
            ),
            (
                "execute",
                "crates/work-infrastructure/fixtures/task-diagnostics",
            ),
            (
                "progress-saver",
                "crates/work-infrastructure/fixtures/delegation-role",
            ),
        ] {
            let storage = LocalDelegationStorage {
                project_root: repo.join(relative),
                skill_root: repo.join("../skills/work"),
                skill_configs: vec![],
            };
            let mut envelope = python_expected(role, &storage);
            envelope["context"]["saved_progress"] = json!({"schema":"work-discussion-progress/v1"});
            let result = match role {
                "plan" => storage.validate_plan_role(&envelope, role, "parent"),
                "execute" => storage.validate_execute_role(&envelope, "parent"),
                "progress-saver" => storage.validate_progress_saver(&envelope, "parent"),
                _ => unreachable!(),
            };
            assert_eq!(
                result.unwrap_err().reason_code,
                "delegation_boundary_mismatch",
                "{role}"
            );
        }
    }
}
