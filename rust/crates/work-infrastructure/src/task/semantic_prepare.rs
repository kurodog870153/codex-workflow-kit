//! Project-backed semantic TASK planning preparation.

use std::path::Path;

use serde_json::Value;
#[cfg(test)]
use serde_json::json;
use work_feature::error::WorkError;
use work_feature::skill::SkillRoot;
use work_feature::task::semantic_prepare::{
    SemanticTaskRepository, SemanticTaskRequest, prepare_semantic_task_request as prepare_feature,
};
#[cfg(test)]
use work_operations::canonical::parse_json_contract;

use crate::files::resolve_project_path;
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::plan_storage::LocalPlanStorage;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::task::draft_storage::LocalTaskDraftStorage;

impl SemanticTaskRepository for LocalTaskDraftStorage {
    fn require_initial_storage_free(&self, requirement_id: &str) -> Result<(), WorkError> {
        LocalTaskDraftStorage::require_initial_storage_free(self, requirement_id)
    }
    fn read_planning_index(&self, requirement_id: &str) -> Result<Value, WorkError> {
        LocalTaskDraftStorage::read_planning_index(self, requirement_id)
    }
    fn normalize_plan_path(&self, plan_path: &str) -> Result<String, WorkError> {
        Ok(resolve_project_path(&self.project_root, plan_path)?.0)
    }
}

pub fn prepare_semantic_task_request(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    requirement_id: &str,
    plan_path: &str,
    expected_revision: u64,
    semantic: &Value,
) -> Result<Value, WorkError> {
    let storage = LocalTaskDraftStorage {
        project_root: root.to_path_buf(),
    };
    let paths = LocalPlanStorage {
        project_root: root.to_path_buf(),
    };
    let instructions = LocalHierarchyCatalog {
        skill_root: skill_root.to_path_buf(),
    };
    let skills = LocalSkillCatalog {
        roots: configs.to_vec(),
    };
    let roots: Vec<SkillRoot> = configs
        .iter()
        .map(|config| SkillRoot {
            scope: config.scope.clone(),
            locator: config.locator.clone(),
        })
        .collect();
    prepare_feature(
        &storage,
        &instructions,
        &skills,
        &paths,
        &roots,
        SemanticTaskRequest {
            requirement_id,
            plan_path,
            expected_revision,
            semantic,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn initial_semantic_preview_and_save_leave_formal_artifacts_absent() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-draft-sources/valid");
        let root = std::env::temp_dir().join(format!(
            "work-task-semantic-initial-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let plan_path = "outputs/work/plans/example.json";
        let destination = root.join(plan_path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(fixture.join(plan_path), destination).unwrap();
        let plan_raw = fs::read(root.join(plan_path)).unwrap();
        let mut plan = parse_json_contract(&plan_raw).unwrap();
        plan["artifacts"]["task"] = json!("outputs/work/tasks/example/index.json");
        fs::write(
            root.join(plan_path),
            work_operations::plan::render_plan_value(&plan).unwrap(),
        )
        .unwrap();
        assert_eq!(
            plan["artifacts"]["task"],
            "outputs/work/tasks/example/index.json"
        );
        let request = json!({"upsert":[{"title":"Task","goal":"Result","scope":["Source"],
            "skill_id":null,"instruction_selection":{"selected_paths":[],"references":[]},
            "dependencies":[]}],"remove_task_ids":[],
            "current_task":{"upsert_position":1},"reason":null});
        let skill = repo.join("crates/work-infrastructure/legacy-work-skill");
        let prepared =
            prepare_semantic_task_request(&root, &skill, &[], "example", plan_path, 0, &request)
                .unwrap();
        let index = &prepared["index"];
        assert_eq!(index["requirement_id"], "example");
        assert_eq!(index["revision"], 1);
        assert_eq!(index["tasks"][0]["status"], "planned");
        assert_eq!(index["tasks"][0]["boundary_revision"], 1);
        let expected: Value = serde_json::from_slice(
            &fs::read(fixture.join("outputs/work/tasks/example/drafts/index.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            index["tasks"][0]["instructions_sha256"],
            expected["tasks"][0]["instructions_sha256"]
        );
        let draft_dir = root.join("outputs/work/tasks/example/drafts");
        for defect in ["selection", "skill", "dependency", "metadata", "cycle"] {
            let mut invalid = request.clone();
            match defect {
                "selection" => {
                    invalid["upsert"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("instruction_selection");
                }
                "skill" => invalid["upsert"][0]["skill_id"] = json!("unknown"),
                "dependency" => {
                    invalid["upsert"][0]["dependencies"] = json!([{"existing_task_id":"TASK-999"}]);
                }
                "metadata" => invalid["upsert"][0]["status"] = json!("refined"),
                "cycle" => {
                    let second = invalid["upsert"][0].clone();
                    invalid["upsert"][0]["dependencies"] = json!([{"upsert_position":2}]);
                    invalid["upsert"].as_array_mut().unwrap().push(second);
                    invalid["upsert"][1]["dependencies"] = json!([{"upsert_position":1}]);
                }
                _ => unreachable!(),
            }
            assert!(
                prepare_semantic_task_request(
                    &root,
                    &skill,
                    &[],
                    "example",
                    plan_path,
                    0,
                    &invalid
                )
                .is_err(),
                "{defect}"
            );
            assert!(!draft_dir.exists(), "{defect} created planning storage");
        }
        let storage = LocalTaskDraftStorage {
            project_root: root.clone(),
        };
        let mut duplicate = index.clone();
        duplicate["tasks"]
            .as_array_mut()
            .unwrap()
            .push(index["tasks"][0].clone());
        assert!(storage.save_planning(&duplicate, 0, None).is_err());
        assert!(!draft_dir.exists());
        assert!(
            !root
                .join("outputs/work/tasks/example/drafts/index.json")
                .exists()
        );
        assert!(
            !root
                .join(plan["artifacts"]["task"].as_str().unwrap())
                .exists()
        );
        assert!(
            !root
                .join(plan["artifacts"]["execution"].as_str().unwrap())
                .exists()
        );
        storage.save_planning(index, 0, None).unwrap();
        assert_eq!(storage.read_planning_index("example").unwrap(), *index);
        assert_eq!(
            prepare_semantic_task_request(&root, &skill, &[], "example", plan_path, 0, &request)
                .unwrap_err()
                .reason_code,
            "draft_initial_storage_exists"
        );
        assert_eq!(
            storage
                .save_planning(index, 0, None)
                .unwrap_err()
                .reason_code,
            "draft_initial_storage_exists"
        );
        assert!(
            !root
                .join(plan["artifacts"]["task"].as_str().unwrap())
                .exists()
        );
        assert!(
            !root
                .join(plan["artifacts"]["execution"].as_str().unwrap())
                .exists()
        );
        let reserved = std::env::temp_dir().join(format!(
            "work-task-semantic-reserved-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let reserved_plan = reserved.join(plan_path);
        fs::create_dir_all(reserved_plan.parent().unwrap()).unwrap();
        fs::copy(root.join(plan_path), reserved_plan).unwrap();
        fs::create_dir_all(reserved.join("outputs/work/tasks/example/drafts/history/1")).unwrap();
        let reserved_storage = LocalTaskDraftStorage {
            project_root: reserved.clone(),
        };
        assert_eq!(
            prepare_semantic_task_request(
                &reserved,
                &skill,
                &[],
                "example",
                plan_path,
                0,
                &request
            )
            .unwrap_err()
            .reason_code,
            "draft_initial_storage_exists"
        );
        assert_eq!(
            reserved_storage
                .save_planning(index, 0, None)
                .unwrap_err()
                .reason_code,
            "draft_initial_storage_exists"
        );
        assert!(
            !reserved
                .join("outputs/work/tasks/example/drafts/index.json")
                .exists()
        );
    }

    #[test]
    fn semantic_split_preview_rejects_stale_sources_and_saves_exact_index() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-draft-sources/valid");
        let root = std::env::temp_dir().join(format!(
            "work-task-semantic-split-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let plan_path = "outputs/work/plans/example.json";
        for relative in [
            plan_path,
            "outputs/work/tasks/example/drafts/index.json",
            "outputs/work/tasks/example/drafts/history/1/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let skill = repo.join("crates/work-infrastructure/legacy-work-skill");
        let original_index =
            fs::read(root.join("outputs/work/tasks/example/drafts/index.json")).unwrap();
        let boundary = json!({"title":"Split","goal":"Result","scope":["Source"],
            "skill_id":null,"instruction_selection":{"selected_paths":[],"references":[]},
            "dependencies":[]});
        let mut dependent = boundary.clone();
        dependent["dependencies"] = json!([{"upsert_position":1}]);
        let split = json!({"upsert":[boundary,dependent],"remove_task_ids":["TASK-001"],
            "current_task":{"upsert_position":1},"reason":"Confirmed split"});
        let prepared =
            prepare_semantic_task_request(&root, &skill, &[], "example", plan_path, 1, &split)
                .unwrap();
        assert_eq!(prepared["index"]["tasks"][0]["id"], "TASK-002");
        assert_eq!(prepared["index"]["tasks"][1]["id"], "TASK-003");
        assert_eq!(
            prepared["index"]["tasks"][1]["dependencies"],
            json!(["TASK-002"])
        );
        assert_eq!(prepared["index"]["retired_task_ids"], json!(["TASK-001"]));
        assert_eq!(
            fs::read(root.join("outputs/work/tasks/example/drafts/index.json")).unwrap(),
            original_index
        );
        let old_shape = json!({"tasks":[],"current_task":1});
        assert_eq!(
            prepare_semantic_task_request(&root, &skill, &[], "example", plan_path, 1, &old_shape)
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
        let removed_dependency = json!({"upsert":[{
            "title":"Replacement","goal":"Result","scope":["Source"],"skill_id":null,
            "instruction_selection":{"selected_paths":[],"references":[]},
            "dependencies":[{"existing_task_id":"TASK-001"}]}],
            "remove_task_ids":["TASK-001"],"current_task":null,"reason":"Replace"});
        assert_eq!(
            prepare_semantic_task_request(
                &root,
                &skill,
                &[],
                "example",
                plan_path,
                1,
                &removed_dependency
            )
            .unwrap_err()
            .reason_code,
            "invalid_semantic_task_reference"
        );
        assert_eq!(
            prepare_semantic_task_request(&root, &skill, &[], "example", plan_path, 2, &split)
                .unwrap_err()
                .reason_code,
            "draft_revision_conflict"
        );
        let selection_change = json!({"upsert":[{"existing_task_id":"TASK-001",
            "title":"Task","goal":"Changed","scope":["Source"],"skill_id":null,
            "instruction_selection":{"selected_paths":[],"references":["task.general.task-records"]},
            "dependencies":[]}],"remove_task_ids":[],"current_task":null,"reason":"Review"});
        assert_eq!(
            prepare_semantic_task_request(
                &root,
                &skill,
                &[],
                "example",
                plan_path,
                1,
                &selection_change
            )
            .unwrap_err()
            .reason_code,
            "draft_selection_mismatch"
        );
        let original_plan = fs::read(root.join(plan_path)).unwrap();
        let mut changed_plan = parse_json_contract(&original_plan).unwrap();
        changed_plan["summary"] = json!("Changed Plan");
        fs::write(
            root.join(plan_path),
            work_operations::plan::render_plan_value(&changed_plan).unwrap(),
        )
        .unwrap();
        assert_eq!(
            prepare_semantic_task_request(&root, &skill, &[], "example", plan_path, 1, &split)
                .unwrap_err()
                .reason_code,
            "draft_source_drift"
        );
        fs::write(root.join(plan_path), original_plan).unwrap();
        let storage = LocalTaskDraftStorage {
            project_root: root.clone(),
        };
        storage
            .update_planning_list(&prepared["request"]["index"], 1, "Confirmed split", false)
            .unwrap();
        assert_eq!(
            storage.read_planning_index("example").unwrap(),
            prepared["index"]
        );
        assert!(
            prepare_semantic_task_request(
                &root,
                &skill,
                &[],
                "example",
                plan_path,
                2,
                &selection_change
            )
            .is_err()
        );
    }

    #[test]
    fn semantic_merge_keeps_second_task_and_rejects_stale_revision() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-draft-sources/valid");
        let root = std::env::temp_dir().join(format!(
            "work-task-semantic-merge-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let plan_path = "outputs/work/plans/example.json";
        for relative in [
            plan_path,
            "outputs/work/tasks/example/drafts/index.json",
            "outputs/work/tasks/example/drafts/history/1/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let storage = LocalTaskDraftStorage {
            project_root: root.clone(),
        };
        let mut expanded = storage.read_planning_index("example").unwrap();
        expanded["revision"] = json!(2);
        let mut second = expanded["tasks"][0].clone();
        second["id"] = json!("TASK-002");
        expanded["tasks"].as_array_mut().unwrap().push(second);
        storage
            .update_planning_list(&expanded, 1, "Add second TASK.", false)
            .unwrap();
        let semantic = json!({"upsert":[{"existing_task_id":"TASK-002",
            "title":"Task","goal":"Merged outcome","scope":["Source"],"skill_id":null,
            "dependencies":[]}],"remove_task_ids":["TASK-001"],
            "current_task":{"existing_task_id":"TASK-002"},"reason":"Confirmed merge"});
        let skill = repo.join("crates/work-infrastructure/legacy-work-skill");
        let prepared =
            prepare_semantic_task_request(&root, &skill, &[], "example", plan_path, 2, &semantic)
                .unwrap();
        assert_eq!(prepared["index"]["tasks"].as_array().unwrap().len(), 1);
        assert_eq!(prepared["index"]["tasks"][0]["id"], "TASK-002");
        assert_eq!(prepared["index"]["tasks"][0]["goal"], "Merged outcome");
        assert_eq!(prepared["index"]["retired_task_ids"], json!(["TASK-001"]));
        assert_eq!(
            storage.read_planning_index("example").unwrap()["revision"],
            2
        );
        assert_eq!(
            prepare_semantic_task_request(&root, &skill, &[], "example", plan_path, 3, &semantic)
                .unwrap_err()
                .reason_code,
            "draft_revision_conflict"
        );
    }
}
