//! Project-backed semantic TASK planning preparation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::skill::SkillRoot;
use work_feature::task::semantic_prepare::{
    SemanticTaskRepository, SemanticTaskRequest, prepare_semantic_task_request as prepare_feature,
};
use work_model::schema::PublicSchema;
use work_model::task::response::TaskDraftPrepare;
#[cfg(test)]
use work_operations::canonical::parse_json_contract;
use work_operations::task::draft::validate_planning_index;

use crate::files::resolve_project_path;
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::plan_storage::LocalPlanStorage;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::task::draft_storage::{LocalTaskDraftStorage, TaskSourceUpdateProjectRequest};

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

/// Commit the exact initial candidate returned by `task prepare` after checking
/// that the planning sources still produce the same candidate.
pub fn save_prepared_initial_task(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    requirement_id: &str,
    plan_path: &str,
    candidate: &Value,
) -> Result<Value, WorkError> {
    let prepared: TaskDraftPrepare = serde_json::from_value(candidate.clone()).map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_task_prepare_result",
            "Supply the complete result from task prepare.",
            json!({}),
        )
    })?;
    let index = &candidate["index"];
    if prepared.schema != PublicSchema::WorkTaskDraftPrepareV1
        || candidate["request"] != *index
        || !prepared.drafts.is_empty()
        || index["requirement_id"] != requirement_id
        || index["revision"] != 1
    {
        return Err(WorkError::new(
            ExitCode::Contract,
            "invalid_task_prepare_result",
            "The initial TASK candidate does not match the requested planning index.",
            json!({}),
        ));
    }
    validate_planning_index(index).map_err(|issue| {
        WorkError::new(
            ExitCode::Contract,
            issue.reason_code,
            issue.message,
            json!({}),
        )
    })?;
    let tasks = index["tasks"].as_array().expect("typed planning tasks");
    let positions = tasks
        .iter()
        .enumerate()
        .filter_map(|(position, task)| task["id"].as_str().map(|id| (id.to_owned(), position + 1)))
        .collect::<std::collections::BTreeMap<_, _>>();
    let upsert = tasks
        .iter()
        .map(|task| {
            let dependencies = task["dependencies"]
                .as_array()
                .expect("typed dependencies")
                .iter()
                .map(|id| json!({"upsert_position":positions[id.as_str().expect("typed ID")]}))
                .collect::<Vec<_>>();
            json!({"title":task["title"],"goal":task["goal"],"scope":task["scope"],
                "skill_id":task["skill_id"],"instruction_selection":task["instruction_selection"],
                "dependencies":dependencies})
        })
        .collect::<Vec<_>>();
    let current = index["current_task_id"]
        .as_str()
        .map(|id| json!({"upsert_position":positions[id]}))
        .unwrap_or(Value::Null);
    let semantic = json!({"upsert":upsert,"remove_task_ids":[],
        "current_task":current,"reason":null});
    let storage = LocalTaskDraftStorage {
        project_root: root.to_path_buf(),
    };
    match prepare_semantic_task_request(
        root,
        skill_root,
        configs,
        requirement_id,
        plan_path,
        0,
        &semantic,
    ) {
        Ok(fresh) => {
            if fresh["index"] != *index
                || fresh["affected_task_ids"] != candidate["affected_task_ids"]
            {
                return Err(WorkError::new(
                    ExitCode::WorkflowState,
                    "draft_source_drift",
                    "The approved TASK candidate no longer matches current planning sources.",
                    json!({}),
                ));
            }
            storage.save_planning(index, 0, None)
        }
        Err(error) if error.reason_code == "draft_initial_storage_exists" => {
            storage.recover_planning(index, 0, None)
        }
        Err(error) => Err(error),
    }
}

/// Save a reviewed list/boundary candidate through the same public Task save
/// command used for initial planning and discussion checkpoints.
pub fn save_prepared_list_task(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    requirement_id: &str,
    plan_path: &str,
    candidate: &Value,
) -> Result<Value, WorkError> {
    let prepared: TaskDraftPrepare = serde_json::from_value(candidate.clone()).map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_task_prepare_result",
            "Supply the complete result from task prepare.",
            json!({}),
        )
    })?;
    let proposed = &candidate["request"]["index"];
    let revision = proposed["revision"].as_u64().ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_task_prepare_result",
            "The prepared planning revision is missing.",
            json!({}),
        )
    })?;
    if prepared.schema != PublicSchema::WorkTaskDraftPrepareV1
        || revision < 2
        || proposed["requirement_id"] != requirement_id
        || candidate["index"]["revision"] != revision
    {
        return Err(WorkError::new(
            ExitCode::Contract,
            "invalid_task_prepare_result",
            "The list candidate does not match the requested planning index.",
            json!({}),
        ));
    }
    validate_planning_index(proposed).map_err(|issue| {
        WorkError::new(
            ExitCode::Contract,
            issue.reason_code,
            issue.message,
            json!({}),
        )
    })?;
    let expected = revision - 1;
    let reason = candidate["request"]["reason"].as_str().ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_task_prepare_result",
            "The list change reason is missing.",
            json!({}),
        )
    })?;
    let storage = LocalTaskDraftStorage {
        project_root: root.to_path_buf(),
    };
    let current = storage.read_planning_index(requirement_id)?;
    if current["revision"] == revision {
        return storage.update_planning_list(proposed, expected, reason, true);
    }
    if current["revision"] != expected {
        return Err(WorkError::new(
            ExitCode::WorkflowState,
            "draft_revision_conflict",
            "Reload the current planning index before saving a list change.",
            json!({}),
        ));
    }
    let old = current["tasks"]
        .as_array()
        .expect("validated planning tasks")
        .iter()
        .filter_map(|task| task["id"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let tasks = proposed["tasks"]
        .as_array()
        .expect("validated planning tasks");
    let positions = tasks
        .iter()
        .enumerate()
        .filter_map(|(position, task)| task["id"].as_str().map(|id| (id.to_owned(), position + 1)))
        .collect::<BTreeMap<_, _>>();
    let removed = old
        .iter()
        .filter(|id| !positions.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    let upsert = tasks
        .iter()
        .map(|task| {
            let id = task["id"].as_str().expect("validated TASK ID");
            let dependencies = task["dependencies"]
                .as_array()
                .expect("validated dependencies")
                .iter()
                .map(|value| {
                    let dependency = value.as_str().expect("validated dependency ID");
                    if old.contains(dependency) {
                        json!({"existing_task_id":dependency})
                    } else {
                        json!({"upsert_position":positions[dependency]})
                    }
                })
                .collect::<Vec<_>>();
            let mut item = json!({"title":task["title"],"goal":task["goal"],"scope":task["scope"],
            "skill_id":task["skill_id"],"instruction_selection":task["instruction_selection"],
            "dependencies":dependencies});
            if old.contains(id) {
                item["existing_task_id"] = json!(id);
            }
            item
        })
        .collect::<Vec<_>>();
    let selected = proposed["current_task_id"]
        .as_str()
        .map(|id| {
            if old.contains(id) {
                json!({"existing_task_id":id})
            } else {
                json!({"upsert_position":positions[id]})
            }
        })
        .unwrap_or(Value::Null);
    let semantic = json!({"upsert":upsert,"remove_task_ids":removed,
        "current_task":selected,"reason":reason});
    let fresh = prepare_semantic_task_request(
        root,
        skill_root,
        configs,
        requirement_id,
        plan_path,
        expected,
        &semantic,
    )?;
    if fresh != *candidate {
        return Err(WorkError::new(
            ExitCode::WorkflowState,
            "draft_source_drift",
            "The approved list candidate no longer matches current planning sources.",
            json!({}),
        ));
    }
    match storage.update_planning_list(proposed, expected, reason, false) {
        Err(error) if error.reason_code == "draft_list_update_interrupted" => {
            storage.update_planning_list(proposed, expected, reason, true)
        }
        result => result,
    }
}

pub fn save_prepared_source_task(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    requirement_id: &str,
    plan_path: &str,
    candidate: &Value,
) -> Result<Value, WorkError> {
    let prepared: TaskDraftPrepare = serde_json::from_value(candidate.clone()).map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_task_prepare_result",
            "Supply the complete result from task prepare.",
            json!({}),
        )
    })?;
    let revision = candidate["index"]["revision"].as_u64().ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_task_prepare_result",
            "The prepared planning revision is missing.",
            json!({}),
        )
    })?;
    if prepared.schema != PublicSchema::WorkTaskDraftPrepareV1
        || revision < 2
        || candidate["index"]["requirement_id"] != requirement_id
        || candidate["request"]["selections"].as_object().is_none()
    {
        return Err(WorkError::new(
            ExitCode::Contract,
            "invalid_task_prepare_result",
            "The source candidate does not match the requested planning index.",
            json!({}),
        ));
    }
    let storage = LocalTaskDraftStorage {
        project_root: root.to_path_buf(),
    };
    let current = storage.read_planning_index(requirement_id)?;
    let expected = revision - 1;
    if current["revision"] != expected && current["revision"] != revision {
        return Err(WorkError::new(
            ExitCode::WorkflowState,
            "draft_revision_conflict",
            "Reload the current planning index before saving a source change.",
            json!({}),
        ));
    }
    if current["revision"] == revision && current != candidate["index"] {
        return Err(WorkError::new(
            ExitCode::WorkflowState,
            "draft_revision_conflict",
            "The committed planning index differs from the approved source change.",
            json!({}),
        ));
    }
    let (plan_path, _) = resolve_project_path(root, plan_path)?;
    let update = |recover| TaskSourceUpdateProjectRequest {
        requirement_id,
        raw_request: &candidate["request"],
        expected_revision: expected,
        plan_path: &plan_path,
        skill_root,
        skill_configs: configs,
        recover,
    };
    if current["revision"] == revision {
        return storage.update_sources_from_project(update(true));
    }
    let fresh = storage.prepare_sources_from_project(update(false))?;
    if fresh != *candidate {
        return Err(WorkError::new(
            ExitCode::WorkflowState,
            "draft_source_drift",
            "The approved source candidate no longer matches current planning sources.",
            json!({}),
        ));
    }
    match storage.update_sources_from_project(update(false)) {
        Err(error) if error.reason_code == "draft_source_update_interrupted" => {
            storage.update_sources_from_project(update(true))
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn prepared_initial_save_is_repeatable_and_recovers_matching_history() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-draft-sources/valid");
        let skill = repo.join("../skills/work");
        let plan_path = "outputs/work/plans/example.json";
        let semantic = json!({"upsert":[{"title":"Task","goal":"Result","scope":["Source"],
            "skill_id":null,"instruction_selection":{"selected_paths":[],"references":[]},
            "dependencies":[]}],"remove_task_ids":[],
            "current_task":{"upsert_position":1},"reason":null});
        for interrupted in [false, true] {
            let root = std::env::temp_dir().join(format!(
                "work-task-facade-initial-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                interrupted
            ));
            let destination = root.join(plan_path);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(plan_path), &destination).unwrap();
            let mut plan = parse_json_contract(&fs::read(&destination).unwrap()).unwrap();
            plan["artifacts"]["task"] = json!("outputs/work/tasks/example/index.json");
            fs::write(
                &destination,
                work_operations::plan::render_plan_value(&plan).unwrap(),
            )
            .unwrap();
            let candidate = prepare_semantic_task_request(
                &root,
                &skill,
                &[],
                "example",
                plan_path,
                0,
                &semantic,
            )
            .unwrap();
            let storage = LocalTaskDraftStorage {
                project_root: root.clone(),
            };
            if interrupted {
                let history = root.join("outputs/work/tasks/example/drafts/history/1");
                fs::create_dir_all(&history).unwrap();
                let raw = work_operations::canonical::canonical_json(&candidate["index"]).unwrap();
                fs::write(history.join("index.json"), &raw).unwrap();
                fs::write(history.join("index-current.tmp"), &raw).unwrap();
            }
            let saved =
                save_prepared_initial_task(&root, &skill, &[], "example", plan_path, &candidate)
                    .unwrap();
            assert_eq!(
                saved["status"],
                if interrupted { "recovered" } else { "saved" }
            );
            assert_eq!(
                storage.read_planning_index("example").unwrap(),
                candidate["index"]
            );
            assert_eq!(
                save_prepared_initial_task(&root, &skill, &[], "example", plan_path, &candidate)
                    .unwrap()["status"],
                "already_completed"
            );
        }
    }

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
        let skill = repo.join("../skills/work");
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
        let skill = repo.join("../skills/work");
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
        let staged = work_feature::task::draft::prepare_list_update(
            &storage,
            &storage.read_planning_index("example").unwrap(),
            &prepared["request"]["index"],
            1,
            "Confirmed split",
        )
        .unwrap();
        let history = root.join("outputs/work/tasks/example/drafts/history/2");
        fs::create_dir_all(&history).unwrap();
        for (name, raw) in &staged.files {
            fs::write(history.join(name), raw).unwrap();
        }
        assert_eq!(
            save_prepared_list_task(&root, &skill, &[], "example", plan_path, &prepared).unwrap()["status"],
            "recovered"
        );
        assert_eq!(
            storage.read_planning_index("example").unwrap(),
            prepared["index"]
        );
        assert_eq!(
            save_prepared_list_task(&root, &skill, &[], "example", plan_path, &prepared).unwrap()["status"],
            "already_completed"
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
        let skill = repo.join("../skills/work");
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
        save_prepared_list_task(&root, &skill, &[], "example", plan_path, &prepared).unwrap();
        assert_eq!(
            storage.read_planning_index("example").unwrap(),
            prepared["index"]
        );
    }
}
