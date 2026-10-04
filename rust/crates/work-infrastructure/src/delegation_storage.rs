//! Delegation contexts derived from validated formal artifacts.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_feature::delegation::DelegationSourceRepository;
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;

use crate::files::{LocalFiles, resolve_project_path};
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
            self,
            envelope,
            sender,
            &project_root.to_string_lossy(),
            &skill_root.to_string_lossy(),
        )
    }

    pub fn build_artifact_editor(&self, request: &Value) -> Result<Value, WorkError> {
        work_feature::delegation::build_artifact_editor(self, request)
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
            self,
            envelope,
            sender,
            &project_root.to_string_lossy(),
            &skill_root.to_string_lossy(),
        )
    }

    pub fn build_execute_role(&self, request: &Value) -> Result<Value, WorkError> {
        work_feature::delegation::build_execute_role(self, request)
    }

    pub fn validate_task_coordinator(
        &self,
        envelope: &Value,
        role: &str,
        sender: &str,
    ) -> Result<Value, WorkError> {
        if role == "task-coordinator" {
            return work_feature::delegation::validate_task_coordinator(
                envelope,
                sender,
                &self.canonical_project_root()?,
                &self.canonical_skill_root()?,
            );
        }
        Err(boundary("Plan is not a delegation role."))
    }

    pub fn build_task_coordinator(&self, request: &Value) -> Result<Value, WorkError> {
        if request["role"] == "task-coordinator" {
            return work_feature::delegation::build_task_coordinator(self, request);
        }
        Err(boundary("Plan is not a delegation role."))
    }
}

impl work_feature::delegation::TaskDelegationRepository for LocalDelegationStorage {
    fn planning_context(&self, value: &Value) -> Result<(Value, Value), WorkError> {
        let requirement = value["snapshot"]["requirement_id"]
            .as_str()
            .ok_or_else(|| boundary("A complete Source Snapshot is required."))?;
        let instructions = crate::hierarchy_catalog::LocalHierarchyCatalog {
            skill_root: self.skill_root.clone(),
        };
        let skills = crate::skill_catalog::LocalSkillCatalog {
            roots: self.skill_configs.clone(),
        };
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: self.project_root.clone(),
        };
        let roots = self
            .skill_configs
            .iter()
            .map(|root| work_feature::skill::SkillRoot {
                scope: root.scope.clone(),
                locator: root.locator.clone(),
            })
            .collect::<Vec<_>>();
        let (_, saved) = work_feature::task::source::validate_context(
            &paths,
            &instructions,
            &skills,
            &paths,
            &roots,
            requirement,
            value,
        )?;
        let selected: Vec<String> =
            serde_json::from_value(value["hierarchy_selection"]["selected_paths"].clone())
                .map_err(|_| boundary("Confirmed hierarchy paths are required."))?;
        let selection = work_feature::instruction::select_task(
            &instructions,
            &value["hierarchy_selection"],
            &selected,
            &[],
        )?;
        let mut collection = value.clone();
        collection["requirement_id"] = json!(requirement);
        collection["source"] = json!({"kind":"snapshot", "manifest":saved.manifest});
        Ok((
            work_feature::delegation::task_source_context(&collection, json!(saved.bytes)),
            serde_json::to_value(selection).expect("instruction selection serializes"),
        ))
    }
    fn task_collection(&self, task_path: &str) -> Result<Value, WorkError> {
        let instructions = crate::hierarchy_catalog::LocalHierarchyCatalog {
            skill_root: self.skill_root.clone(),
        };
        let skills = crate::skill_catalog::LocalSkillCatalog {
            roots: self.skill_configs.clone(),
        };
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: self.project_root.clone(),
        };
        let roots = self
            .skill_configs
            .iter()
            .map(|root| work_feature::skill::SkillRoot {
                scope: root.scope.clone(),
                locator: root.locator.clone(),
            })
            .collect::<Vec<_>>();
        work_feature::task::load_collection(
            &instructions,
            &skills,
            &paths,
            &crate::task::storage::LocalTaskStorage {
                project_root: self.project_root.clone(),
            },
            &roots,
            task_path,
        )
    }
    fn source_bytes(&self, collection: &Value) -> Result<Value, WorkError> {
        use work_feature::ports::SourceSnapshotReader;
        if collection["source"]["kind"] == "migration" {
            return Ok(Value::Null);
        }
        let manifest: work_model::source_snapshot::SourceSnapshot =
            serde_json::from_value(collection["source"]["manifest"].clone())
                .map_err(|_| boundary("A Source Snapshot is required."))?;
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: self.project_root.clone(),
        };
        let saved = paths.read_snapshot_at(
            &manifest
                .requirement_id
                .parse()
                .map_err(|_| boundary("A portable requirement ID is required."))?,
            &manifest.source_id,
            collection["artifacts"]["source"]
                .as_str()
                .ok_or_else(|| boundary("The Source root is required."))?,
        )?;
        if saved.manifest != manifest {
            return Err(boundary(
                "The stored Source Snapshot differs from Task provenance.",
            ));
        }
        Ok(json!(saved.bytes))
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
    use work_operations::delegation::{build_envelope, validation_result};

    fn role_fixture(role: &str, storage: &LocalDelegationStorage) -> Value {
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
    fn task_coordinator_context_rejects_retired_plan_role() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let storage = LocalDelegationStorage {
            project_root: repo.join("crates/work-infrastructure/fixtures/task-diagnostics"),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![],
        };
        let index: Value = serde_json::from_slice(
            &std::fs::read(
                storage
                    .project_root
                    .join("outputs/work/tasks/example/index.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let request = json!({"schema":"work-delegation-build-request","role":"task-coordinator","request":"Coordinate confirmed work.","planning_source":{"snapshot":index["source"]["manifest"],"artifacts":index["artifacts"],"hierarchy_selection":index["hierarchy_selection"],"skill_selection":index["skill_selection"],"acceptance_criteria":index["acceptance_criteria"]}});
        let envelope = storage.build_task_coordinator(&request).unwrap();
        assert_eq!(envelope, role_fixture("task-coordinator", &storage));
        assert_eq!(
            storage
                .validate_task_coordinator(&envelope, "task-coordinator", "parent")
                .unwrap(),
            validation_result("task-coordinator", "task", false)
        );
        let mut drifted = envelope.clone();
        drifted["context"]["task_source"]["hierarchy_selection"]["selection_sha256"] =
            json!("0".repeat(64));
        assert!(
            storage
                .validate_task_coordinator(&drifted, "task-coordinator", "parent")
                .is_err()
        );
        let legacy = role_fixture("plan", &storage);
        assert!(
            storage
                .validate_task_coordinator(&legacy, "plan", "parent")
                .is_err()
        );
        assert!(storage.build_task_coordinator(&json!({"schema":"work-delegation-build-request","role":"plan","request":"Coordinate confirmed work.","source_plan_path":"missing.json"})).is_err());
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
                "schema":"work-delegation-build-request","role":"execute",
                "request":"Execute selected TASK.",
                "task_path":"outputs/work/tasks/example/index.json","task_id":"TASK-001",
            }))
            .unwrap();
        assert_eq!(envelope, role_fixture("execute", &storage));
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
            {
                let mut value = validation_result("execute", "execute", false);
                value["source_validation"] = json!("checked");
                value
            }
        );
        let mut drifted = envelope.clone();
        drifted["context"]["task_collection_sha256"] = json!("0".repeat(64));
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
    fn progress_saver_context_matches_current_contract_reference() {
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
                "schema":"work-delegation-build-request","role":"progress-saver",
                "mode":"task","request":"Save discussion progress.",
                "source_progress_path":"outputs/work/progress/example/task/progress.json",
                "content":content,"continuation_point":"Continue discussion",
            }))
            .unwrap();
        assert_eq!(envelope, role_fixture("progress-saver", &storage));
        assert_eq!(
            storage
                .validate_progress_saver(&envelope, "parent")
                .unwrap(),
            validation_result("progress-saver", "task", false)
        );
        for change in [
            "mode",
            "origin_mode",
            "source_bytes",
            "planning_source",
            "missing_source",
        ] {
            let mut wrong = envelope.clone();
            match change {
                "mode" => wrong["mode"] = json!("plan"),
                "origin_mode" => wrong["origin_mode"] = json!("plan"),
                "source_bytes" => wrong["context"]["task_source"]["source_bytes"][0] = json!(0),
                "planning_source" => {
                    wrong["context"]["content"]["context"]["planning_source"]["snapshot"]["source_sha256"] =
                        json!("0".repeat(64))
                }
                _ => {
                    wrong["context"]
                        .as_object_mut()
                        .unwrap()
                        .remove("task_source");
                }
            }
            assert!(
                storage.validate_progress_saver(&wrong, "parent").is_err(),
                "{change}"
            );
        }
        let source_raw = std::fs::read(
            storage
                .project_root
                .join("outputs/work/sources/example/SRC-001/source.txt"),
        )
        .unwrap();
        let root = std::env::temp_dir().join(format!(
            "work-delegation-progress-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let progress_storage = crate::progress_storage::LocalProgressStorage { project_root: root };
        let preview =
            work_feature::progress::preview_progress(&progress_storage, &progress, 0).unwrap();
        work_feature::progress::save_progress(
            &progress_storage,
            &progress,
            0,
            preview["approved_sha256"].as_str().unwrap(),
        )
        .unwrap();
        let saved =
            work_feature::progress::read_progress(&progress_storage, "example", "task").unwrap();
        assert_eq!(
            saved["progress"]["context"]["planning_source"],
            progress["context"]["planning_source"]
        );
        assert_eq!(
            std::fs::read(
                storage
                    .project_root
                    .join("outputs/work/sources/example/SRC-001/source.txt")
            )
            .unwrap(),
            source_raw
        );
        assert!(
            !progress_storage
                .project_root
                .join("outputs/work/plans")
                .exists()
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
    fn artifact_editor_rejects_legacy_plan_and_plan_origin() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let storage = LocalDelegationStorage {
            project_root: repo
                .join("crates/work-infrastructure/fixtures/delegation-role/legacy-plan"),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![],
        };
        let request = json!({"schema":"work-delegation-build-request","role":"artifact-editor","mode":"execute","request":"Revise confirmed artifact.","source_plan_path":"outputs/work/plans/example.json","confirmed_request":{"reason":"Reviewed"},"decisions":["Confirmed revision"],"affected_task_ids":["TASK-001"],"continuation_point":"Return to Execute"});
        assert_eq!(
            storage
                .build_artifact_editor(&request)
                .unwrap_err()
                .reason_code,
            "delegation_boundary_mismatch"
        );
        let mut request = request;
        request.as_object_mut().unwrap().remove("source_plan_path");
        request["task_path"] = json!("outputs/work/tasks/example/index.json");
        request["mode"] = json!("plan");
        assert_eq!(
            storage
                .build_artifact_editor(&request)
                .unwrap_err()
                .reason_code,
            "delegation_boundary_mismatch"
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
                "schema":"work-delegation-build-request","role":"artifact-editor",
                "mode":"execute","request":"Revise confirmed artifact.",
                "task_path":"outputs/work/tasks/example/index.json",
                "confirmed_request":{"schema":"work-spec-prepare-request","requirement_id":"example","reason":"Reviewed","edits":[{"target":{"artifact":"task_item","task_id":"TASK-001"},"field":"goal","after":"Reviewed goal"}]},"decisions":["Confirmed revision"],
                "affected_task_ids":["TASK-001"],"continuation_point":"Return to Execute",
            }))
            .unwrap();
        assert_eq!(envelope, role_fixture("artifact-editor-formal", &storage));
        assert_eq!(
            envelope["context"]["artifacts"]["task"],
            "outputs/work/tasks/example/index.json"
        );
        let verified = storage
            .validate_artifact_editor(&envelope, "parent")
            .unwrap();
        assert_eq!(verified["source_validation"], "checked");
        assert_eq!(verified["grants_authorization"], false);
        for field in ["source_plan", "plan", "origin_mode"] {
            let mut wrong = envelope.clone();
            wrong["context"][field] = json!("plan");
            assert!(storage.validate_artifact_editor(&wrong, "parent").is_err());
        }
        let mut wrong = envelope.clone();
        wrong["context"]["confirmed_request"]["edits"][0]["target"]["artifact"] = json!("plan");
        assert!(storage.validate_artifact_editor(&wrong, "parent").is_err());
        let mut wrong = envelope.clone();
        wrong["context"]["affected_task_ids"] = json!(["TASK-999"]);
        assert!(storage.validate_artifact_editor(&wrong, "parent").is_err());
        let mut wrong = envelope.clone();
        wrong["context"]["task_source"]["source_bytes"][0] = json!(0);
        assert!(storage.validate_artifact_editor(&wrong, "parent").is_err());
        let mut wrong = envelope.clone();
        wrong["context"]["task_collection_sha256"] = json!("0".repeat(64));
        assert!(storage.validate_artifact_editor(&wrong, "parent").is_err());
    }

    #[test]
    fn task_skill_context_binds_task_owned_skill_and_source() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let storage = LocalDelegationStorage {
            project_root: repo
                .join("crates/work-infrastructure/fixtures/delegation-role/task-skill"),
            skill_root: repo.join("../skills/work"),
            skill_configs: vec![SkillRootConfig {
                scope: "repo".into(),
                locator: "delegation-fixture".into(),
                path: repo
                    .join("crates/work-infrastructure/fixtures/delegation-role/task-skill/skills"),
            }],
        };
        let envelope = storage
            .build_task_skill(&json!({
                "schema":"work-delegation-build-request","role":"task-skill",
                "request":"Refine selected TASK.",
                "task_path":"outputs/work/tasks/example/index.json","task_id":"TASK-001",
            }))
            .unwrap();
        assert_eq!(envelope, role_fixture("task-skill", &storage));
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
    fn task_resume_validates_identity_and_rejects_plan_progress() {
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
            if role == "plan" {
                assert!(
                    build_envelope(
                        role,
                        mode,
                        "resume example",
                        &storage.canonical_project_root().unwrap(),
                        &storage.canonical_skill_root().unwrap(),
                        &json!({"saved_progress":progress})
                    )
                    .is_err()
                );
                continue;
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
            if mode == "plan" {
                assert_eq!(
                    storage
                        .validate_task_coordinator(&envelope, role, "parent")
                        .unwrap_err()
                        .reason_code,
                    "invalid_progress_mode"
                );
                continue;
            }
            assert_eq!(
                storage
                    .validate_task_coordinator(&envelope, role, "parent")
                    .unwrap(),
                validation_result(role, mode, true)
            );
            let mut missing_source = envelope.clone();
            missing_source["context"]["saved_progress"]["context"]
                .as_object_mut()
                .unwrap()
                .remove("planning_source");
            assert!(
                storage
                    .validate_task_coordinator(&missing_source, role, "parent")
                    .is_err()
            );
            let mut old_origin = envelope.clone();
            old_origin["origin_mode"] = json!("plan");
            assert!(
                storage
                    .validate_task_coordinator(&old_origin, role, "parent")
                    .is_err()
            );
            let mut wrong = envelope.clone();
            wrong["request"] = json!("resume other");
            assert_eq!(
                storage
                    .validate_task_coordinator(&wrong, role, "parent")
                    .unwrap_err()
                    .reason_code,
                "delegation_boundary_mismatch"
            );
        }
    }

    #[test]
    fn role_context_missing_fields_match_current_contract_rejection_messages() {
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
                "task_source",
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
                "crates/work-infrastructure/fixtures/task-diagnostics",
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
            if role == "plan" {
                assert!(
                    storage
                        .validate_task_coordinator(&role_fixture(role, &storage), role, "parent")
                        .is_err()
                );
                continue;
            }
            let mut envelope = role_fixture(role, &storage);
            envelope["context"]
                .as_object_mut()
                .unwrap()
                .remove(required);
            let outcome = match role {
                "plan" | "task-coordinator" => {
                    storage.validate_task_coordinator(&envelope, role, "parent")
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
            if role == "plan" {
                assert!(
                    storage
                        .validate_task_coordinator(&role_fixture(role, &storage), role, "parent")
                        .is_err()
                );
                continue;
            }
            let envelope = role_fixture(role, &storage);
            let before = envelope.clone();
            let result = match role {
                "plan" | "task-coordinator" => {
                    storage.validate_task_coordinator(&envelope, role, "parent")
                }
                "execute" => storage.validate_execute_role(&envelope, "parent"),
                "task-skill" => storage.validate_task_skill(&envelope, "task-coordinator"),
                "artifact-editor" => storage.validate_artifact_editor(&envelope, "parent"),
                "progress-saver" => storage.validate_progress_saver(&envelope, "parent"),
                _ => unreachable!(),
            }
            .unwrap();
            assert_eq!(result["status"], "valid", "{role}");
            assert_eq!(
                result["source_validation"],
                if matches!(role, "artifact-editor" | "execute") {
                    "checked"
                } else {
                    "not_checked"
                },
                "{role}"
            );
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
            if role == "plan" {
                assert!(
                    storage
                        .validate_task_coordinator(&role_fixture(role, &storage), role, "parent")
                        .is_err()
                );
                continue;
            }
            let mut envelope = role_fixture(role, &storage);
            envelope["context"]["saved_progress"] = json!({"schema":"work-discussion-progress"});
            let result = match role {
                "plan" => storage.validate_task_coordinator(&envelope, role, "parent"),
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
