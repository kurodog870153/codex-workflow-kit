//! Reconstruct a reviewed Plan, TASK collection and execution index from semantic sources.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::instruction::{
    load as load_instructions, select as select_instructions, task_document_selection,
};
use work_feature::plan::{PlanPathRepository, prepare_semantic};
use work_feature::ports::ArtifactStore;
use work_feature::skill::SkillRoot;
use work_feature::task::{CollectionInput, validate_collection};
use work_operations::canonical::{parse_json_contract, sha256_hex};
use work_operations::execution::index::{build_initial_execution_index, render_execution_index};
use work_operations::identifiers::RequirementId;
use work_operations::plan::render_plan_value;
use work_operations::task::candidate::build_semantic_candidate;
use work_operations::task::ordering::{TaskDocumentKind, render_task};

use crate::files::LocalFiles;
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::plan_storage::LocalPlanStorage;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::specification::storage::{execution_history_fingerprints, storage_path};

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

struct ReconstructionPaths(LocalPlanStorage);

impl PlanPathRepository for ReconstructionPaths {
    fn default_paths(&self, id: &RequirementId) -> Result<Value, WorkError> {
        self.0.default_paths(id)
    }
    fn validate_paths(
        &self,
        id: &RequirementId,
        artifacts: &Value,
        actual: &str,
        allow_task_index: bool,
    ) -> Result<(), WorkError> {
        self.0
            .validate_paths(id, artifacts, actual, allow_task_index)
    }
    fn exists(&self, _: &str) -> Result<bool, WorkError> {
        Ok(false)
    }
    fn create_exclusive(&self, path: &str, content: &[u8]) -> Result<(), WorkError> {
        self.0.create_exclusive(path, content)
    }
    fn read(&self, path: &str) -> Result<Vec<u8>, WorkError> {
        self.0.read(path)
    }
}

pub fn prepare_reconstruction_request(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    raw_request: &[u8],
) -> Result<Value, WorkError> {
    let request = parse_json_contract(raw_request).map_err(|_| {
        fail(
            "invalid_json_contract",
            "The reconstruction preparation is invalid.",
        )
    })?;
    if request["schema"] != "work-spec-migration-prepare-request/v1"
        || request["mode"] != "reconstruction"
    {
        return Err(fail(
            "migration_prepare_mode",
            "A reconstruction request is required.",
        ));
    }
    let semantic = &request["plan"];
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
    let paths = ReconstructionPaths(LocalPlanStorage {
        project_root: root.to_path_buf(),
    });
    let prepared_plan = prepare_semantic(&hierarchy, &skills, &paths, &roots, semantic)?;
    let plan = &prepared_plan["plan"];
    let artifacts = &plan["artifacts"];
    let plan_path = artifacts["plan"].as_str().ok_or_else(|| {
        fail(
            "migration_candidate_set_incomplete",
            "The reconstructed Plan path is missing.",
        )
    })?;
    let index_path = artifacts["task"].as_str().ok_or_else(|| {
        fail(
            "migration_candidate_set_incomplete",
            "The reconstructed TASK path is missing.",
        )
    })?;
    let execution_dir = artifacts["execution"].as_str().ok_or_else(|| {
        fail(
            "migration_candidate_set_incomplete",
            "The reconstructed execution path is missing.",
        )
    })?;
    let plan_raw = render_plan_value(plan).map_err(|_| {
        fail(
            "invalid_contract_value",
            "The reconstructed Plan cannot be rendered.",
        )
    })?;
    let tasks = request["tasks"]
        .as_array()
        .filter(|tasks| !tasks.is_empty())
        .ok_or_else(|| {
            fail(
                "migration_candidate_set_incomplete",
                "At least one semantic TASK is required.",
            )
        })?;
    let task_ids = (1..=tasks.len())
        .map(|position| format!("TASK-{position:03}"))
        .collect::<Vec<_>>();
    let dependency_files = tasks
        .iter()
        .enumerate()
        .map(|(position, task)| {
            let files = task["candidate"]["files"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .filter_map(|(index, row)| {
                    row["key"]
                        .as_str()
                        .map(|key| (key.to_owned(), format!("FILE-{:03}", index + 1)))
                })
                .collect::<BTreeMap<_, _>>();
            (task_ids[position].clone(), files)
        })
        .collect::<BTreeMap<_, _>>();
    let goal_ids = plan["goals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let deliverable_ids = plan["deliverables"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let acceptance_ids = plan["acceptance_criteria"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let mut items = BTreeMap::new();
    let mut source_sets = Vec::new();
    for (position, task) in tasks.iter().enumerate() {
        let id = &task_ids[position];
        let mut dependencies = Vec::new();
        let mut seen = BTreeSet::new();
        for position_value in task["dependency_positions"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let dependency = position_value
                .as_u64()
                .map(|number| number as usize)
                .filter(|number| *number > 0 && *number <= position)
                .ok_or_else(|| {
                    fail(
                        "migration_dependency_position",
                        "TASK dependencies must refer to distinct earlier positions.",
                    )
                })?;
            if !seen.insert(dependency) {
                return Err(fail(
                    "migration_dependency_position",
                    "TASK dependencies must refer to distinct earlier positions.",
                ));
            }
            dependencies.push(task_ids[dependency - 1].clone());
        }
        let selected = task["selected_paths"]
            .as_array()
            .ok_or_else(|| {
                fail(
                    "invalid_source_selection",
                    "TASK selected instruction paths are required.",
                )
            })?
            .iter()
            .filter_map(|row| row.as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let references = task["references"]
            .as_array()
            .ok_or_else(|| {
                fail(
                    "invalid_source_selection",
                    "TASK instruction references are required.",
                )
            })?
            .iter()
            .filter_map(|row| row.as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let selection = select_instructions(&hierarchy, "task", &selected, &references)?;
        source_sets.push(load_instructions(
            &hierarchy,
            "task",
            &selected,
            &references,
        )?);
        let nested = build_semantic_candidate(
            &task["candidate"],
            &acceptance_ids,
            &dependencies,
            &dependency_files,
        )
        .map_err(|issue| {
            WorkError::new(
                ExitCode::Contract,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
        let mut item = json!({"schema":"work-task-item/v1","id":id,"title":task["title"],
            "goal":task["goal"],"skill_id":task["skill_id"],
            "instruction_selection":selection,
            "traceability":{"goal_ids":goal_ids,"deliverable_ids":deliverable_ids,
                "acceptance_ids":acceptance_ids}});
        if !dependencies.is_empty() {
            item["dependencies"] = json!(dependencies);
        }
        for (key, value) in nested.as_object().expect("semantic candidate object") {
            item[key] = value.clone();
        }
        items.insert(id.clone(), item);
    }
    let mut index = json!({"schema":"work-task-index/v1",
        "requirement_id":plan["requirement_id"],"spec_id":"TASK-SPEC-001",
        "status":"confirmed","title":request["task_title"],"summary":request["task_summary"],
        "artifacts":artifacts,
        "source_plan":{"canonical_sha256":sha256_hex(&plan_raw),
            "hierarchy_selection_sha256":prepared_plan["validation"]["hierarchy_selection_sha256"]},
        "instruction_selection":task_document_selection(&source_sets)?,
        "readiness":{"status":"passed","spec_id":"TASK-SPEC-001"}});
    if !request["execution_defaults"].is_null() {
        index["execution_defaults"] = request["execution_defaults"].clone();
    }
    let directory = index_path.rsplit_once('/').map_or("", |(parent, _)| parent);
    let mut item_raw = BTreeMap::new();
    let mut references = Vec::new();
    for (id, item) in &items {
        let raw = render_task(item, TaskDocumentKind::Item).map_err(|_| {
            fail(
                "invalid_contract_value",
                "A reconstructed TASK item cannot be rendered.",
            )
        })?;
        references.push(json!({"id":id,"path":format!("tasks/{id}.json"),
            "canonical_sha256":sha256_hex(&raw)}));
        item_raw.insert(id.clone(), raw);
    }
    index["tasks"] = Value::Array(references);
    let index_raw = render_task(&index, TaskDocumentKind::Index).map_err(|_| {
        fail(
            "invalid_contract_value",
            "The reconstructed TASK index cannot be rendered.",
        )
    })?;
    let validation = validate_collection(
        &hierarchy,
        &skills,
        &paths,
        &roots,
        CollectionInput {
            index_raw: &index_raw,
            item_raw: &item_raw,
            index_path,
            source_plan_raw: &plan_raw,
        },
    )?;
    let execution = build_initial_execution_index(&validation["collection_contract"], &validation)
        .map_err(|issue| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
    let execution_path = format!("{execution_dir}/index.json");
    let execution_raw = render_execution_index(&execution).map_err(|_| {
        fail(
            "invalid_contract_value",
            "The reconstructed execution index cannot be rendered.",
        )
    })?;
    let mut documents = vec![
        json!({"path":plan_path,"kind":"plan","content":parse_json_contract(&plan_raw).map_err(|_| fail("invalid_json_contract","The Plan is invalid."))?}),
        json!({"path":index_path,"kind":"task_index","content":parse_json_contract(&index_raw).map_err(|_| fail("invalid_json_contract","The TASK index is invalid."))?}),
        json!({"path":execution_path,"kind":"execution_index","content":parse_json_contract(&execution_raw).map_err(|_| fail("invalid_json_contract","The execution index is invalid."))?}),
    ];
    for (id, raw) in &item_raw {
        documents.push(json!({"path":format!("{directory}/tasks/{id}.json"),
            "kind":"task_item","task_id":id,
            "content":parse_json_contract(raw).map_err(|_| fail("invalid_json_contract","A TASK item is invalid."))?}));
    }
    let mut sources = Vec::new();
    for document in &documents {
        let path = document["path"].as_str().expect("generated path");
        let absolute = storage_path(root, path)?;
        if absolute.exists() {
            let raw = LocalFiles.read_raw(&absolute)?;
            sources.push(json!({"path":path,"raw_sha256":sha256_hex(&raw)}));
        }
    }
    if sources.is_empty() {
        return Err(fail(
            "migration_source_missing",
            "Cross-file migration requires retained source bytes.",
        ));
    }
    let item_directory = storage_path(root, &format!("{directory}/tasks"))?;
    if item_directory.is_dir() {
        for entry in fs::read_dir(item_directory).map_err(|_| {
            fail(
                "migration_unreviewed_task_source",
                "The TASK source directory cannot be inspected.",
            )
        })? {
            let entry = entry.map_err(|_| {
                fail(
                    "migration_unreviewed_task_source",
                    "The TASK source directory cannot be inspected.",
                )
            })?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !items.keys().any(|id| name == format!("{id}.json")) {
                return Err(fail(
                    "migration_unreviewed_task_source",
                    "Unknown TASK source files require a separate semantic decision.",
                ));
            }
        }
    }
    if !execution_history_fingerprints(root, execution_dir)?.is_empty() {
        return Err(fail(
            "migration_history_present",
            "Reconstruction cannot replace an execution index with immutable history.",
        ));
    }
    Ok(work_model::specification::verified::<
        work_model::specification::SpecMigrationPreviewRequest,
    >(
        json!({"schema":"work-spec-migration-preview-request/v1",
        "sources":sources,"candidates":documents,
        "semantic_decisions":request["semantic_decisions"].as_array().cloned().unwrap_or_default()})
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::specification::migration::preview_migration;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn semantic_reconstruction_matches_python_candidate_and_preview() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-migration/reconstruction");
        let root = std::env::temp_dir().join(format!(
            "work-reconstruction-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for relative in [
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let request = fs::read(fixture.join("semantic-request.json")).unwrap();
        let expected: Value =
            serde_json::from_slice(&fs::read(fixture.join("request.json")).unwrap()).unwrap();
        let actual = prepare_reconstruction_request(
            &root,
            &repo.join("crates/work-infrastructure/legacy-work-skill"),
            &[],
            &request,
        )
        .unwrap();
        assert_eq!(actual, expected);
        let expected_preview: Value =
            serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        assert_eq!(
            preview_migration(
                &root,
                &repo.join("crates/work-infrastructure/legacy-work-skill"),
                &[],
                &actual
            )
            .unwrap(),
            expected_preview
        );
    }
}
