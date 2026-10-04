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
use work_operations::task::draft::validate_planning_index;

use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::task::draft_storage::{LocalTaskDraftStorage, TaskSourceUpdateProjectRequest};

impl SemanticTaskRepository for LocalTaskDraftStorage {
    fn require_initial_storage_free(&self, requirement_id: &str) -> Result<(), WorkError> {
        LocalTaskDraftStorage::require_initial_storage_free(self, requirement_id)
    }
    fn read_planning_index(&self, requirement_id: &str) -> Result<Value, WorkError> {
        LocalTaskDraftStorage::read_planning_index(self, requirement_id)
    }
}

pub fn prepare_semantic_task_request(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    requirement_id: &str,
    expected_revision: u64,
    semantic: &Value,
) -> Result<Value, WorkError> {
    let storage = LocalTaskDraftStorage {
        project_root: root.to_path_buf(),
    };
    let paths = crate::artifact_paths::LocalArtifactPaths {
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
            expected_revision,
            semantic,
        },
    )
}

fn verify_prepared_source(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    index: &Value,
) -> Result<(), WorkError> {
    let paths = crate::artifact_paths::LocalArtifactPaths {
        project_root: root.to_path_buf(),
    };
    let hierarchy = LocalHierarchyCatalog {
        skill_root: skill_root.to_path_buf(),
    };
    let skills = LocalSkillCatalog {
        roots: configs.to_vec(),
    };
    let roots = configs
        .iter()
        .map(|config| SkillRoot {
            scope: config.scope.clone(),
            locator: config.locator.clone(),
        })
        .collect::<Vec<_>>();
    work_feature::task::source::validate_context(
        &paths,
        &hierarchy,
        &skills,
        &paths,
        &roots,
        index["requirement_id"].as_str().ok_or_else(|| {
            WorkError::new(
                ExitCode::Contract,
                "invalid_requirement_id",
                "Prepared Task requires an explicit requirement.",
                json!({}),
            )
        })?,
        &index["source"],
    )?;
    Ok(())
}
/// Commit the exact initial candidate returned by `task prepare` after checking
/// that the planning sources still produce the same candidate.
pub fn save_prepared_initial_task(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    requirement_id: &str,
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
    if prepared.schema != PublicSchema::WorkTaskDraftPrepare
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
    verify_prepared_source(root, skill_root, configs, index)?;
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
        "current_task":current,"reason":null,"source":index["source"]});
    let storage = LocalTaskDraftStorage {
        project_root: root.to_path_buf(),
    };
    match prepare_semantic_task_request(root, skill_root, configs, requirement_id, 0, &semantic) {
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
    if prepared.schema != PublicSchema::WorkTaskDraftPrepare
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
    verify_prepared_source(root, skill_root, configs, proposed)?;
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
        "current_task":selected,"reason":reason,"source":proposed["source"]});
    let fresh = prepare_semantic_task_request(
        root,
        skill_root,
        configs,
        requirement_id,
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
    if prepared.schema != PublicSchema::WorkTaskDraftPrepare
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
    verify_prepared_source(root, skill_root, configs, &candidate["index"])?;
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
    let update = |recover| TaskSourceUpdateProjectRequest {
        requirement_id,
        raw_request: &candidate["request"],
        expected_revision: expected,
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

    struct ChangingSource {
        storage: LocalTaskDraftStorage,
        reads: std::cell::Cell<usize>,
        metadata_drift: bool,
    }

    impl work_feature::ports::SourceSnapshotReader for ChangingSource {
        fn read_snapshot(
            &self,
            id: &work_model::identifiers::RequirementId,
            source_id: &work_model::identifiers::SourceId,
        ) -> Result<work_feature::ports::SnapshotBytes, WorkError> {
            self.read_snapshot_at(
                id,
                source_id,
                &format!("outputs/work/sources/{}", id.as_str()),
            )
        }

        fn read_snapshot_at(
            &self,
            id: &work_model::identifiers::RequirementId,
            source_id: &work_model::identifiers::SourceId,
            root: &str,
        ) -> Result<work_feature::ports::SnapshotBytes, WorkError> {
            let mut snapshot = work_feature::ports::SourceSnapshotReader::read_snapshot_at(
                &self.storage,
                id,
                source_id,
                root,
            )?;
            let reads = self.reads.get() + 1;
            self.reads.set(reads);
            if reads == 2 {
                if self.metadata_drift {
                    snapshot.manifest.captured_at = "2026-10-04T00:00:00Z".into();
                } else {
                    snapshot.bytes[0] ^= 1;
                }
            }
            Ok(snapshot)
        }
    }

    impl work_feature::task::draft::TaskDraftHistoryRepository for ChangingSource {
        fn read_historical_draft(
            &self,
            requirement: &str,
            revision: u64,
            task: &str,
        ) -> Result<Vec<u8>, WorkError> {
            work_feature::task::draft::TaskDraftHistoryRepository::read_historical_draft(
                &self.storage,
                requirement,
                revision,
                task,
            )
        }
    }

    impl SemanticTaskRepository for ChangingSource {
        fn require_initial_storage_free(&self, requirement: &str) -> Result<(), WorkError> {
            self.storage.require_initial_storage_free(requirement)
        }
        fn read_planning_index(&self, requirement: &str) -> Result<Value, WorkError> {
            self.storage.read_planning_index(requirement)
        }
    }

    #[test]
    fn initial_and_list_reject_second_source_read_drift_without_publication() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-draft-sources/valid");
        for list in [false, true] {
            for metadata_drift in [false, true] {
                let root = std::env::temp_dir().join(format!(
                    "work-task-second-source-read-{}-{}-{list}-{metadata_drift}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
                let index_path = root.join("outputs/work/tasks/example/drafts/index.json");
                if list {
                    for path in [
                        "outputs/work/tasks/example/drafts/index.json",
                        "outputs/work/tasks/example/drafts/history/1/index.json",
                    ] {
                        let target = root.join(path);
                        fs::create_dir_all(target.parent().unwrap()).unwrap();
                        fs::copy(fixture.join(path), target).unwrap();
                    }
                }
                let index_before = fs::read(&index_path).ok();
                let source_path = root.join("outputs/work/sources/example/SRC-001/source.txt");
                let source_before = fs::read(&source_path).unwrap();
                let mut upsert = json!({"title":"Task","goal":"Updated result","scope":["Source"],"skill_id":null,"dependencies":[]});
                if list {
                    upsert["existing_task_id"] = json!("TASK-001");
                } else {
                    upsert["instruction_selection"] = json!({"selected_paths":[],"references":[]});
                }
                let mut semantic = json!({"upsert":[upsert],"remove_task_ids":[],"current_task":{"upsert_position":1},"reason":if list {json!("Review boundary.")}else{Value::Null}});
                if !list {
                    semantic["source"] = fixture_source(&fixture);
                }
                let repository = ChangingSource {
                    storage: LocalTaskDraftStorage {
                        project_root: root.clone(),
                    },
                    reads: std::cell::Cell::new(0),
                    metadata_drift,
                };
                let error = prepare_feature(
                    &repository,
                    &LocalHierarchyCatalog {
                        skill_root: repo.join("../skills/work"),
                    },
                    &LocalSkillCatalog { roots: vec![] },
                    &crate::artifact_paths::LocalArtifactPaths {
                        project_root: root.clone(),
                    },
                    &[],
                    SemanticTaskRequest {
                        requirement_id: "example",
                        expected_revision: u64::from(list),
                        semantic: &semantic,
                    },
                )
                .unwrap_err();
                assert_eq!(repository.reads.get(), 2);
                assert_eq!(
                    error.reason_code,
                    if metadata_drift {
                        "source_snapshot_mismatch"
                    } else {
                        "source_hash_mismatch"
                    }
                );
                assert_eq!(fs::read(&index_path).ok(), index_before);
                assert_eq!(fs::read(&source_path).unwrap(), source_before);
                assert!(!root.join("outputs/work/plans").exists());
                assert!(
                    !root
                        .join("outputs/work/tasks/example/drafts/history/2")
                        .exists()
                );
            }
        }
    }

    fn fixture_source(fixture: &Path) -> Value {
        let index: Value = serde_json::from_slice(
            &std::fs::read(fixture.join("outputs/work/tasks/example/drafts/index.json")).unwrap(),
        )
        .unwrap();
        index["source"].clone()
    }
    #[test]
    fn prepared_initial_save_is_repeatable_and_recovers_matching_history() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-draft-sources/valid");
        let skill = repo.join("../skills/work");
        let semantic = json!({"upsert":[{"title":"Task","goal":"Result","scope":["Source"],
            "skill_id":null,"instruction_selection":{"selected_paths":[],"references":[]},
            "dependencies":[]}],"remove_task_ids":[],
            "current_task":{"upsert_position":1},"reason":null,"source":fixture_source(&fixture)});
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
            crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
            let candidate =
                prepare_semantic_task_request(&root, &skill, &[], "example", 0, &semantic).unwrap();
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
                save_prepared_initial_task(&root, &skill, &[], "example", &candidate).unwrap();
            assert_eq!(
                saved["status"],
                if interrupted { "recovered" } else { "saved" }
            );
            assert_eq!(
                storage.read_planning_index("example").unwrap(),
                candidate["index"]
            );
            assert_eq!(
                save_prepared_initial_task(&root, &skill, &[], "example", &candidate).unwrap()["status"],
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
        crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        let request = json!({"upsert":[{"title":"Task","goal":"Result","scope":["Source"],
            "skill_id":null,"instruction_selection":{"selected_paths":[],"references":[]},
            "dependencies":[]}],"remove_task_ids":[],
            "current_task":{"upsert_position":1},"reason":null,"source":fixture_source(&fixture)});
        let skill = repo.join("../skills/work");
        let prepared =
            prepare_semantic_task_request(&root, &skill, &[], "example", 0, &request).unwrap();
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
                prepare_semantic_task_request(&root, &skill, &[], "example", 0, &invalid).is_err(),
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
        assert!(!root.join("outputs/work/tasks/example/index.json").exists());
        assert!(!root.join("outputs/work/executions/example").exists());
        storage.save_planning(index, 0, None).unwrap();
        assert_eq!(storage.read_planning_index("example").unwrap(), *index);
        assert_eq!(
            prepare_semantic_task_request(&root, &skill, &[], "example", 0, &request)
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
        assert!(!root.join("outputs/work/tasks/example/index.json").exists());
        assert!(!root.join("outputs/work/executions/example").exists());
        let reserved = std::env::temp_dir().join(format!(
            "work-task-semantic-reserved-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        crate::fixture_support::copy_fixture_sources(&fixture, &reserved).unwrap();
        fs::create_dir_all(reserved.join("outputs/work/tasks/example/drafts/history/1")).unwrap();
        let reserved_storage = LocalTaskDraftStorage {
            project_root: reserved.clone(),
        };
        assert_eq!(
            prepare_semantic_task_request(&reserved, &skill, &[], "example", 0, &request)
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
        for relative in [
            "outputs/work/tasks/example/drafts/index.json",
            "outputs/work/tasks/example/drafts/history/1/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
            crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
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
            prepare_semantic_task_request(&root, &skill, &[], "example", 1, &split).unwrap();
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
            prepare_semantic_task_request(&root, &skill, &[], "example", 1, &old_shape)
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
            prepare_semantic_task_request(&root, &skill, &[], "example", 1, &removed_dependency)
                .unwrap_err()
                .reason_code,
            "invalid_semantic_task_reference"
        );
        assert_eq!(
            prepare_semantic_task_request(&root, &skill, &[], "example", 2, &split)
                .unwrap_err()
                .reason_code,
            "draft_revision_conflict"
        );
        let selection_change = json!({"upsert":[{"existing_task_id":"TASK-001",
            "title":"Task","goal":"Changed","scope":["Source"],"skill_id":null,
            "instruction_selection":{"selected_paths":[],"references":["task.general.task-records"]},
            "dependencies":[]}],"remove_task_ids":[],"current_task":null,"reason":"Review"});
        assert_eq!(
            prepare_semantic_task_request(&root, &skill, &[], "example", 1, &selection_change)
                .unwrap_err()
                .reason_code,
            "draft_selection_mismatch"
        );
        let source_path = root.join("outputs/work/sources/example/SRC-001/source.txt");
        let original_source = fs::read(&source_path).unwrap();
        let mut changed_source = original_source.clone();
        changed_source[0] ^= 1;
        fs::write(&source_path, &changed_source).unwrap();
        assert_eq!(
            prepare_semantic_task_request(&root, &skill, &[], "example", 1, &split)
                .unwrap_err()
                .reason_code,
            "source_hash_mismatch"
        );
        fs::write(&source_path, original_source).unwrap();
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
            save_prepared_list_task(&root, &skill, &[], "example", &prepared).unwrap()["status"],
            "recovered"
        );
        assert_eq!(
            storage.read_planning_index("example").unwrap(),
            prepared["index"]
        );
        assert_eq!(
            save_prepared_list_task(&root, &skill, &[], "example", &prepared).unwrap()["status"],
            "already_completed"
        );
        assert!(
            prepare_semantic_task_request(&root, &skill, &[], "example", 2, &selection_change)
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
        for relative in [
            "outputs/work/tasks/example/drafts/index.json",
            "outputs/work/tasks/example/drafts/history/1/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
            crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
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
            prepare_semantic_task_request(&root, &skill, &[], "example", 2, &semantic).unwrap();
        assert_eq!(prepared["index"]["tasks"].as_array().unwrap().len(), 1);
        assert_eq!(prepared["index"]["tasks"][0]["id"], "TASK-002");
        assert_eq!(prepared["index"]["tasks"][0]["goal"], "Merged outcome");
        assert_eq!(prepared["index"]["retired_task_ids"], json!(["TASK-001"]));
        assert_eq!(
            storage.read_planning_index("example").unwrap()["revision"],
            2
        );
        assert_eq!(
            prepare_semantic_task_request(&root, &skill, &[], "example", 3, &semantic)
                .unwrap_err()
                .reason_code,
            "draft_revision_conflict"
        );
        save_prepared_list_task(&root, &skill, &[], "example", &prepared).unwrap();
        assert_eq!(
            storage.read_planning_index("example").unwrap(),
            prepared["index"]
        );
    }
}
