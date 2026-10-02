//! Formal TASK assembly over saved draft history and approved creation.

use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::skill::SkillRoot;
use work_feature::task::assembly::{
    ProjectAssemblyInput, TaskAssemblyRepository, approved_contract, assemble_from_repository,
};
use work_operations::task::ordering::{TaskDocumentKind, render_task};

use crate::artifact_paths::LocalArtifactPaths;
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::task::create_storage::{CreateTaskRequest, create_task_artifacts};
use crate::task::draft_storage::LocalTaskDraftStorage;

pub struct TaskAssemblyRequest<'a> {
    pub requirement_id: &'a str,
    pub metadata: &'a Value,
    pub expected_revision: u64,
}

pub struct LocalTaskAssembly {
    pub project_root: std::path::PathBuf,
}

impl TaskAssemblyRepository for LocalTaskAssembly {
    fn read_planning_index(&self, requirement_id: &str) -> Result<Value, WorkError> {
        LocalTaskDraftStorage {
            project_root: self.project_root.to_path_buf(),
        }
        .read_planning_index(requirement_id)
    }

    fn read_draft(&self, index: &Value, task_id: &str) -> Result<Value, WorkError> {
        LocalTaskDraftStorage {
            project_root: self.project_root.to_path_buf(),
        }
        .read_draft(index, task_id)
    }
}

pub fn assemble_from_project(
    project_root: &Path,
    skill_root: &Path,
    skill_configs: &[SkillRootConfig],
    request: &TaskAssemblyRequest<'_>,
) -> Result<Value, WorkError> {
    let instructions = LocalHierarchyCatalog {
        skill_root: skill_root.to_path_buf(),
    };
    let skills = LocalSkillCatalog {
        roots: skill_configs.to_vec(),
    };
    let roots = skill_configs
        .iter()
        .map(|config| SkillRoot {
            scope: config.scope.clone(),
            locator: config.locator.clone(),
        })
        .collect::<Vec<_>>();
    let paths = LocalArtifactPaths {
        project_root: project_root.to_path_buf(),
    };
    assemble_from_repository(
        &LocalTaskAssembly {
            project_root: project_root.to_path_buf(),
        },
        &instructions,
        &skills,
        &paths,
        &roots,
        ProjectAssemblyInput {
            requirement_id: request.requirement_id,
            metadata: request.metadata,
            expected_revision: request.expected_revision,
        },
    )
}

pub fn create_from_drafts(
    project_root: &Path,
    skill_root: &Path,
    skill_configs: &[SkillRootConfig],
    request: &TaskAssemblyRequest<'_>,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    let assembled = assemble_from_project(project_root, skill_root, skill_configs, request)?;
    let contract = approved_contract(&assembled, approved_sha256)?;
    let raw = render_task(contract, TaskDocumentKind::Collection).map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_contract_value",
            "The TASK contract cannot be rendered.",
            json!({}),
        )
    })?;
    let artifacts = &contract["artifacts"];
    let mut created = create_task_artifacts(
        project_root,
        skill_root,
        skill_configs,
        CreateTaskRequest {
            raw: &raw,
            source_root: artifacts["source"].as_str().unwrap(),
            task_path: artifacts["task"].as_str().unwrap(),
            execution_dir: artifacts["execution"].as_str().unwrap(),
            recovery: false,
        },
    )?;
    created["approval_sha256"] = json!(approved_sha256);
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use work_feature::task::assembly::{AssemblyInput, assemble_task_drafts};

    fn snapshot(path: &Path, files: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                snapshot(&path, files);
            } else if path.is_file() {
                files.insert(path.clone(), fs::read(path).unwrap());
            }
        }
    }

    #[test]
    fn source_only_draft_assembly_and_creation_preserve_approval() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-assembly");
        let root = std::env::temp_dir().join(format!(
            "work-task-assembly-storage-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        let index: Value =
            serde_json::from_slice(&fs::read(fixture.join("index.json")).unwrap()).unwrap();
        let draft: Value =
            serde_json::from_slice(&fs::read(fixture.join("draft.json")).unwrap()).unwrap();
        let metadata: Value =
            serde_json::from_slice(&fs::read(fixture.join("metadata.json")).unwrap()).unwrap();
        let storage = LocalTaskDraftStorage {
            project_root: root.clone(),
        };
        let mut initial = index.clone();
        initial["revision"] = json!(1);
        initial["tasks"][0]["status"] = json!("planned");
        initial["tasks"][0]
            .as_object_mut()
            .unwrap()
            .remove("draft_ref");
        storage.save_planning(&initial, 0, None).unwrap();
        storage.save_planning(&index, 1, Some(&draft)).unwrap();
        let request = TaskAssemblyRequest {
            requirement_id: "example",
            metadata: &metadata,
            expected_revision: 2,
        };
        let skill = repo.join("../skills/work");
        let mut before_assembly = BTreeMap::new();
        snapshot(&root, &mut before_assembly);
        let assembled = assemble_from_project(&root, &skill, &[], &request).unwrap();
        let mut after_assembly = BTreeMap::new();
        snapshot(&root, &mut after_assembly);
        assert_eq!(after_assembly, before_assembly);
        assert!(!root.join("outputs/work/plans").exists());
        assert_eq!(assembled["contract"]["source"]["kind"], "snapshot");
        assert_eq!(
            assembled["contract"]["acceptance_criteria"][0]["id"],
            "ACCEPTANCE-001"
        );
        assert_eq!(
            assembled["contract"]["tasks"][0]["acceptance_criteria"][0]["id"],
            "TASK-001-ACCEPTANCE-001"
        );
        assert!(!root.join("outputs/work/tasks/example/index.json").exists());
        let instructions = LocalHierarchyCatalog {
            skill_root: skill.clone(),
        };
        let skills = LocalSkillCatalog { roots: vec![] };
        let paths = LocalArtifactPaths {
            project_root: root.clone(),
        };
        for (missing_candidate, reason) in [
            (true, "task_candidate_required"),
            (false, "draft_not_refined"),
        ] {
            let mut case_draft = draft.clone();
            let mut case_index = index.clone();
            if missing_candidate {
                case_draft.as_object_mut().unwrap().remove("task_candidate");
            } else {
                case_draft["status"] = json!("needs_review");
                case_draft["next_discussion_point"] = json!("Review change");
                case_index["tasks"][0]["status"] = json!("needs_review");
            }
            let cases = BTreeMap::from([("TASK-001".to_owned(), case_draft)]);
            let error = assemble_task_drafts(
                &instructions,
                &skills,
                &paths,
                &[],
                AssemblyInput {
                    index: &case_index,
                    drafts: &cases,
                    metadata: &metadata,
                    expected_revision: 2,
                },
            )
            .unwrap_err();
            assert_eq!(error.reason_code, reason);
        }
        let expected: Value =
            serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        assert_eq!(assembled, expected);
        let mut changed_metadata = metadata.clone();
        changed_metadata["summary"] = json!("Changed summary");
        assert_eq!(
            create_from_drafts(
                &root,
                &skill,
                &[],
                &TaskAssemblyRequest {
                    requirement_id: request.requirement_id,
                    metadata: &changed_metadata,
                    expected_revision: request.expected_revision,
                },
                assembled["approval_sha256"].as_str().unwrap(),
            )
            .unwrap_err()
            .reason_code,
            "draft_approval_mismatch"
        );
        assert!(!root.join("outputs/work/tasks/example/index.json").exists());
        let source_path = root.join("outputs/work/sources/example/SRC-001/source.txt");
        let original_source = fs::read(&source_path).unwrap();
        let mut changed_source = original_source.clone();
        changed_source[0] ^= 1;
        fs::write(&source_path, &changed_source).unwrap();
        assert_eq!(
            assemble_from_project(&root, &skill, &[], &request)
                .unwrap_err()
                .reason_code,
            "source_hash_mismatch"
        );
        fs::write(&source_path, original_source).unwrap();
        assert_eq!(
            create_from_drafts(&root, &skill, &[], &request, &"0".repeat(64))
                .unwrap_err()
                .reason_code,
            "draft_approval_mismatch"
        );
        assert!(!root.join("outputs/work/tasks/example/index.json").exists());
        let approved = assembled["approval_sha256"].as_str().unwrap();
        assert_eq!(
            create_from_drafts(&root, &skill, &[], &request, approved).unwrap()["status"],
            "created"
        );
        assert!(root.join("outputs/work/tasks/example/index.json").is_file());
        assert!(
            root.join("outputs/work/executions/example/index.json")
                .is_file()
        );
        let mut newer_index = index.clone();
        newer_index["revision"] = json!(3);
        let mut newer_draft = draft.clone();
        newer_draft["revision"] = json!(2);
        newer_draft["notes"] = json!(["More discussion evidence"]);
        storage
            .save_planning(&newer_index, 2, Some(&newer_draft))
            .unwrap();
        assert_eq!(
            assemble_from_project(&root, &skill, &[], &request)
                .unwrap_err()
                .reason_code,
            "draft_revision_conflict"
        );
        let updated = assemble_from_project(
            &root,
            &skill,
            &[],
            &TaskAssemblyRequest {
                expected_revision: 3,
                ..request
            },
        )
        .unwrap();
        for field in [
            "task_collection_sha256",
            "task_index_sha256",
            "task_item_sha256",
        ] {
            assert_eq!(assembled[field], updated[field], "{field}");
        }
        assert_ne!(assembled["approval_sha256"], updated["approval_sha256"]);
    }
}
