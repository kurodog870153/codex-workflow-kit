//! Migration preview validation over source, plan, task and path ports.

use crate::error::{ExitCode, WorkError};
use crate::instruction::InstructionSourceRepository;
use crate::plan::{PlanPathRepository, PlanValidationInput, validate_plan};
use crate::skill::{SkillRoot, SkillSnapshotRepository};
use crate::specification::migration_preview::{MigrationPreviewInput, finish_preview};
use crate::task::{CollectionInput, validate_collection};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use work_operations::derivation::fingerprint;
use work_operations::execution::index::{render_execution_index, validate_execution_index};
use work_operations::plan::render_plan_value;
use work_operations::specification::migration_diff::unified_diff;
use work_operations::task::ordering::{TaskDocumentKind, render_task};

pub trait MigrationPreviewRepository {
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError>;
    fn validate_path(&self, relative: &str) -> Result<(), WorkError>;
}
fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

fn validation_result<T>(name: &str, result: &Result<T, WorkError>) -> Value {
    match result {
        Ok(_) => json!({"name":name,"status":"passed"}),
        Err(error) => json!({"name":name,"status":"failed",
            "code":error.reason_code,"message":error.message}),
    }
}

pub fn preview_migration<R, H, S, P>(
    repository: &R,
    instructions: &H,
    skills: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    request: &Value,
) -> Result<Value, WorkError>
where
    R: MigrationPreviewRepository,
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    if request["schema"] != "work-spec-migration-preview-request/v1" {
        return Err(fail(
            "migration_preview_schema",
            "A migration preview request is required.",
        ));
    }
    for candidate in request["candidates"].as_array().ok_or_else(|| {
        fail(
            "migration_candidate_set_incomplete",
            "Migration candidates are required.",
        )
    })? {
        let is_item = candidate["kind"] == "task_item";
        let has_task_id = candidate
            .get("task_id")
            .is_some_and(|value| !value.is_null());
        if is_item != has_task_id {
            return Err(WorkError::new(
                ExitCode::Contract,
                "invalid_contract_value",
                "Only a task_item candidate requires task_id.",
                json!({"location":"candidates"}),
            ));
        }
    }
    let mut sources = BTreeMap::new();
    for evidence in request["sources"].as_array().ok_or_else(|| {
        fail(
            "migration_source_missing",
            "Migration source evidence is required.",
        )
    })? {
        let path = evidence["path"].as_str().ok_or_else(|| {
            fail(
                "migration_source_missing",
                "A migration source path is required.",
            )
        })?;
        if sources.contains_key(path) {
            return Err(fail(
                "migration_source_duplicate",
                "Migration source paths must be unique.",
            ));
        }
        let raw = repository.read(path)?;
        if evidence["raw_sha256"] != fingerprint::raw(&raw) {
            return Err(fail(
                "migration_source_changed",
                "Migration source bytes differ from reviewed evidence.",
            ));
        }
        sources.insert(path.to_owned(), raw);
    }
    let mut candidates = BTreeMap::new();
    let mut kinds = BTreeMap::<&str, Vec<&Value>>::new();
    for row in request["candidates"].as_array().ok_or_else(|| {
        fail(
            "migration_candidate_set_incomplete",
            "Migration candidates are required.",
        )
    })? {
        let path = row["path"].as_str().ok_or_else(|| {
            fail(
                "migration_candidate_set_incomplete",
                "A migration candidate path is required.",
            )
        })?;
        if candidates.contains_key(path) {
            return Err(fail(
                "migration_candidate_duplicate",
                "Migration candidate paths must be unique.",
            ));
        }
        let kind = row["kind"].as_str().ok_or_else(|| {
            fail(
                "migration_candidate_set_incomplete",
                "A migration candidate kind is required.",
            )
        })?;
        let raw = match kind {
            "plan" => render_plan_value(&row["content"]),
            "task_index" => render_task(&row["content"], TaskDocumentKind::Index),
            "task_item" => render_task(&row["content"], TaskDocumentKind::Item),
            "execution_index" => render_execution_index(&row["content"]),
            _ => {
                return Err(fail(
                    "migration_candidate_set_incomplete",
                    "Unknown migration candidate kind.",
                ));
            }
        }
        .map_err(|_| {
            fail(
                "invalid_contract_value",
                "A migration candidate cannot be rendered.",
            )
        })?;
        repository.validate_path(path)?;
        candidates.insert(path.to_owned(), raw);
        kinds.entry(kind).or_default().push(row);
    }
    let mut relationships = Vec::new();
    let complete = ["plan", "task_index", "execution_index"]
        .iter()
        .all(|kind| kinds.get(kind).is_some_and(|rows| rows.len() == 1))
        && kinds.get("task_item").is_some_and(|rows| {
            !rows.is_empty()
                && rows
                    .iter()
                    .filter_map(|row| row["task_id"].as_str())
                    .collect::<BTreeSet<_>>()
                    .len()
                    == rows.len()
        });
    if complete {
        relationships.push(json!({"name":"candidate_set","status":"passed"}));
    } else {
        relationships.push(json!({"name":"candidate_set","status":"failed",
            "code":"migration_candidate_set_incomplete",
            "message":"A complete candidate set requires one Plan, TASK index, execution index, and unique TASK items."}));
    }
    let mut validators = Vec::new();
    if complete {
        let plan = kinds["plan"][0];
        let index = kinds["task_index"][0];
        let execution = kinds["execution_index"][0];
        let plan_path = plan["path"].as_str().unwrap();
        let index_path = index["path"].as_str().unwrap();
        let execution_path = execution["path"].as_str().unwrap();
        let plan_raw = &candidates[plan_path];
        let plan_result = validate_plan(
            instructions,
            skills,
            paths,
            skill_roots,
            &plan["content"],
            PlanValidationInput {
                raw: plan_raw,
                actual_plan_path: plan_path,
                allow_task_index: true,
            },
        );
        validators.push(validation_result("plan", &plan_result));
        let mut items = BTreeMap::new();
        for row in &kinds["task_item"] {
            let id = row["task_id"].as_str().unwrap();
            items.insert(
                id.to_owned(),
                candidates[row["path"].as_str().unwrap()].clone(),
            );
        }
        let collection_result = validate_collection(
            instructions,
            skills,
            paths,
            skill_roots,
            CollectionInput {
                index_raw: &candidates[index_path],
                item_raw: &items,
                index_path,
                source_plan_raw: plan_raw,
            },
        );
        validators.push(validation_result("task_collection", &collection_result));
        let execution_value = &execution["content"];
        let execution_result =
            validate_execution_index(execution_value, &candidates[execution_path]);
        validators.push(match &execution_result {
            Ok(_) => json!({"name":"execution_index","status":"passed"}),
            Err(issue) => json!({"name":"execution_index","status":"failed",
                "code":issue.reason_code,"message":issue.message}),
        });
        let artifacts = &plan["content"]["artifacts"];
        let paths_match = artifacts["plan"] == plan_path
            && artifacts["task"] == index_path
            && artifacts["execution"]
                .as_str()
                .is_some_and(|path| format!("{path}/index.json") == execution_path);
        relationships.push(if paths_match {
            json!({"name":"artifact_paths","status":"passed"})
        } else {
            json!({"name":"artifact_paths","status":"failed",
                "code":"migration_artifact_path_mismatch",
                "message":"Candidate paths do not match the Plan artifact routing."})
        });
        let bound = collection_result.ok().and_then(|validation| {
            execution_result.ok().map(|_| {
                let rows = execution_value["tasks"].as_array().unwrap();
                let ids = validation["task_ids"].as_array().unwrap();
                execution_value["requirement_id"] == validation["requirement_id"]
                    && execution_value["task_spec_id"] == validation["spec_id"]
                    && execution_value["task_collection_sha256"]
                        == validation["task_collection_sha256"]
                    && execution_value["task_index_sha256"] == validation["task_index_sha256"]
                    && rows.len() == ids.len()
                    && ids.iter().all(|id| {
                        rows.iter().any(|row| {
                            row["id"] == *id
                                && row["task_item_sha256"]
                                    == validation["task_item_sha256"][id.as_str().unwrap()]
                        })
                    })
            })
        });
        relationships.push(match bound {
            Some(true) => json!({"name":"execution_binding","status":"passed"}),
            Some(false) => json!({"name":"execution_binding","status":"failed",
                "code":"migration_execution_binding_mismatch",
                "message":"The execution index does not bind to the candidate TASK collection."}),
            None => json!({"name":"execution_binding","status":"failed",
                "code":"migration_execution_binding_not_checked",
                "message":"Execution binding requires valid TASK and execution candidates."}),
        });
    }
    let all_paths = sources
        .keys()
        .chain(candidates.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let diffs = all_paths.iter().map(|path| {
        let before = sources.get(path).map(Vec::as_slice);
        let after = candidates.get(path).map(Vec::as_slice);
        json!({"path":path,"operation":if before.is_none() {"add"} else if after.is_none() {"remove"} else {"replace"},
            "unified_diff":unified_diff(path,before,after)})
    }).collect::<Vec<_>>();
    finish_preview(MigrationPreviewInput {
        request,
        sources: &sources,
        candidates: &candidates,
        validators,
        relationships,
        all_paths,
        diffs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hierarchy::HierarchyCatalogRepository;
    use work_operations::hierarchy::{CrossModeCatalog, Hierarchy};
    use work_operations::identifiers::RequirementId;
    use work_operations::instruction::SourceSet;

    struct Unused;
    impl MigrationPreviewRepository for Unused {
        fn read(&self, _: &str) -> Result<Vec<u8>, WorkError> {
            panic!("invalid request must stop before source read")
        }
        fn validate_path(&self, _: &str) -> Result<(), WorkError> {
            panic!("invalid request must stop before path validation")
        }
    }
    impl HierarchyCatalogRepository for Unused {
        fn cross_mode_catalog(&self) -> Result<CrossModeCatalog, WorkError> {
            panic!("invalid request must stop before hierarchy")
        }
        fn mode_paths(&self, _: &str) -> Result<Vec<String>, WorkError> {
            panic!("invalid request must stop before hierarchy")
        }
    }
    impl InstructionSourceRepository for Unused {
        fn load_sources(
            &self,
            _: &str,
            _: &Hierarchy,
            _: &[String],
        ) -> Result<SourceSet, WorkError> {
            panic!("invalid request must stop before sources")
        }
    }
    impl SkillSnapshotRepository for Unused {
        fn snapshot(&self, _: &str, _: &str, _: &str) -> Result<Value, WorkError> {
            panic!("invalid request must stop before skills")
        }
    }
    impl PlanPathRepository for Unused {
        fn default_paths(&self, _: &RequirementId) -> Result<Value, WorkError> {
            panic!("invalid request must stop before paths")
        }
        fn validate_paths(
            &self,
            _: &RequirementId,
            _: &Value,
            _: &str,
            _: bool,
        ) -> Result<(), WorkError> {
            panic!("invalid request must stop before paths")
        }
        fn exists(&self, _: &str) -> Result<bool, WorkError> {
            panic!("invalid request must stop before paths")
        }
        fn create_exclusive(&self, _: &str, _: &[u8]) -> Result<(), WorkError> {
            panic!("invalid request must stop before paths")
        }
        fn read(&self, _: &str) -> Result<Vec<u8>, WorkError> {
            panic!("invalid request must stop before paths")
        }
    }

    #[test]
    fn invalid_schema_stops_before_all_ports() {
        let error =
            preview_migration(&Unused, &Unused, &Unused, &Unused, &[], &json!({})).unwrap_err();
        assert_eq!(error.reason_code, "migration_preview_schema");
    }
}
