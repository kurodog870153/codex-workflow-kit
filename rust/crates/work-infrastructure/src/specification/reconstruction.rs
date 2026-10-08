//! Reconstruct reviewed Task-owned decisions and Execution from retained raw evidence.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::instruction::{
    load as load_instructions, select as select_instructions, task_document_selection,
};
use work_feature::ports::ArtifactStore;
use work_feature::skill::SkillRoot;
use work_feature::task::{CollectionInput, validate_collection};
use work_operations::canonical::parse_json_contract;
use work_operations::derivation::fingerprint;
use work_operations::derivation::graph::{ArtifactNode, reconcile_artifact_bindings};
use work_operations::execution::index::{build_initial_execution_index, render_execution_index};
use work_operations::identifiers::RequirementId;
use work_operations::task::candidate::build_semantic_candidate;
use work_operations::task::ordering::{TaskDocumentKind, render_task};

use crate::files::LocalFiles;
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::specification::storage::{execution_history_fingerprints, storage_path};

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
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
    if request["schema"] != "work-spec-migration-prepare-request"
        || request["mode"] != "reconstruction"
    {
        return Err(fail(
            "migration_prepare_mode",
            "A reconstruction request is required.",
        ));
    }
    work_feature::specification::migration_prepare::validate_semantic_request(&request)?;
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
    let context = &request["task_context"];
    let artifacts = &context["artifacts"];
    let id: RequirementId = request["requirement_id"]
        .as_str()
        .expect("validated requirement")
        .parse()
        .expect("validated ID");
    let reviewed_paths =
        serde_json::from_value(artifacts.clone()).expect("validated Task artifact paths");
    work_feature::artifact_paths::ArtifactPathRepository::validate_paths(
        &crate::artifact_paths::LocalArtifactPaths {
            project_root: root.to_path_buf(),
        },
        &id,
        &reviewed_paths,
    )?;
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
    let mut context_check = context.clone();
    // Migration evidence is embedded; it does not fabricate a Source Snapshot.
    let mut evidence = Vec::new();
    for reviewed in request["sources"]
        .as_array()
        .expect("validated raw sources")
    {
        let path = reviewed["path"].as_str().expect("validated source path");
        let raw = LocalFiles.read_raw(&storage_path(root, path)?)?;
        if reviewed["raw_sha256"] != fingerprint::raw(&raw) {
            return Err(fail(
                "migration_source_changed",
                "Original bytes differ from AI-reviewed evidence.",
            ));
        }
        evidence.push(work_model::task::source::MigrationSourceEvidence {
            path: path.into(),
            raw_sha256: fingerprint::raw(&raw),
            size: raw.len() as u64,
            raw,
        });
    }
    evidence.sort_by(|left, right| left.path.cmp(&right.path));
    if evidence.is_empty() {
        return Err(fail(
            "migration_source_missing",
            "Reconstruction requires retained original bytes.",
        ));
    }
    let requirement = request["requirement_id"]
        .as_str()
        .expect("validated requirement");
    let retained = work_model::task::source::TaskProvenance::Migration {
        approval_sha256: fingerprint::migration_source_approval(requirement, &evidence),
        sources: evidence,
    };
    let provenance = if let Some(source) = context.get("source") {
        let supplied: work_model::task::source::TaskProvenance =
            serde_json::from_value(source.clone()).expect("validated typed Task provenance");
        if matches!(
            supplied,
            work_model::task::source::TaskProvenance::Migration { .. }
        ) && supplied != retained
        {
            return Err(fail(
                "migration_source_evidence_mismatch",
                "Task migration provenance must retain the exact reviewed raw evidence.",
            ));
        }
        supplied
    } else {
        retained
    };
    context_check["source"] =
        serde_json::to_value(&provenance).expect("migration evidence serializes");
    work_operations::task::source::validate_formal_context(&context_check, requirement).map_err(
        |error| {
            WorkError::new(
                ExitCode::Contract,
                error.reason_code,
                error.message,
                error.details,
            )
        },
    )?;
    if context["artifacts"]["task"] != index_path
        || context["artifacts"]["execution"] != execution_dir
    {
        return Err(fail(
            "migration_candidate_routing_mismatch",
            "Reviewed Task routes must match the reconstructed artifact destinations.",
        ));
    }
    let acceptance_ids = context["acceptance_criteria"]
        .as_array()
        .expect("validated definitions")
        .iter()
        .map(|row| row["id"].as_str().expect("validated ID").to_owned())
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
        let mut item = json!({"schema":"work-task-item","id":id,"title":task["title"],
            "goal":task["goal"],"skill_id":task["skill_id"],
            "instruction_selection":selection,
            "traceability":{"acceptance_ids":task["candidate"]["acceptance_ids"]},
            "acceptance_criteria":task["candidate"]["acceptance_criteria"]});
        if !dependencies.is_empty() {
            item["dependencies"] = json!(dependencies);
        }
        for (key, value) in nested.as_object().expect("semantic candidate object") {
            item[key] = value.clone();
        }
        items.insert(id.clone(), item);
    }
    let mut index = json!({"schema":"work-task-index",
        "requirement_id":request["requirement_id"],"spec_id":"TASK-SPEC-001",
        "status":"confirmed","title":request["task_title"],"summary":request["task_summary"],
        "artifacts":context["artifacts"],
        "source":provenance,
        "hierarchy_selection":context["hierarchy_selection"],
        "skill_selection":context["skill_selection"],
        "acceptance_criteria":context["acceptance_criteria"],
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
        references.push(json!({"id":id,"path":format!("tasks/{id}.json")}));
        item_raw.insert(id.clone(), raw);
    }
    index["tasks"] = Value::Array(references);
    let index_raw = reconcile_artifact_bindings(
        &mut index,
        &item_raw,
        None,
        &BTreeSet::from([ArtifactNode::TaskIndexBytes]),
    )
    .map_err(|_| {
        fail(
            "invalid_contract_value",
            "The reconstructed TASK index cannot be rendered.",
        )
    })?;
    let validation = validate_collection(
        &hierarchy,
        &skills,
        &crate::artifact_paths::LocalArtifactPaths {
            project_root: root.to_path_buf(),
        },
        &roots,
        CollectionInput {
            index_raw: &index_raw,
            item_raw: &item_raw,
            index_path,
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
        json!({"path":index_path,"kind":"task_index","content":parse_json_contract(&index_raw).map_err(|_| fail("invalid_json_contract","The TASK index is invalid."))?}),
        json!({"path":execution_path,"kind":"execution_index","content":parse_json_contract(&execution_raw).map_err(|_| fail("invalid_json_contract","The execution index is invalid."))?}),
    ];
    for (id, raw) in &item_raw {
        documents.push(json!({"path":format!("{directory}/tasks/{id}.json"),
            "kind":"task_item","task_id":id,
            "content":parse_json_contract(raw).map_err(|_| fail("invalid_json_contract","A TASK item is invalid."))?}));
    }
    let mut sources = request["sources"]
        .as_array()
        .expect("validated reviewed evidence")
        .clone();
    for document in &documents {
        let path = document["path"].as_str().expect("generated path");
        if storage_path(root, path)?.is_file() && !sources.iter().any(|row| row["path"] == path) {
            return Err(fail(
                "migration_source_review_incomplete",
                "Every existing replacement target requires reviewed raw evidence.",
            ));
        }
    }
    if matches!(
        provenance,
        work_model::task::source::TaskProvenance::Snapshot { .. }
    ) {
        for path in work_feature::task::source::evidence_paths(&index)? {
            if !sources.iter().any(|row| row["path"] == path) {
                return Err(fail(
                    "migration_source_review_incomplete",
                    "The fixed Source manifest, marker and original content must be included in reviewed evidence.",
                ));
            }
        }
    }
    sources.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    for reviewed in &sources {
        let raw = LocalFiles.read_raw(&storage_path(
            root,
            reviewed["path"].as_str().expect("reviewed path"),
        )?)?;
        if reviewed["raw_sha256"] != fingerprint::raw(&raw) {
            return Err(fail(
                "migration_source_changed",
                "Reviewed bytes changed while candidates were assembled.",
            ));
        }
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
    >(json!({"schema":"work-spec-migration-preview-request",
        "sources":sources,"candidates":documents,
        "semantic_decisions":request["semantic_decisions"].as_array().cloned().unwrap_or_default()})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::specification::migration::preview_migration;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn semantic_reconstruction_matches_current_contract_candidate_and_preview() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/cases/specification/migration/reconstruction/project");
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
        let request = fs::read(fixture.join("../input/semantic-request.json")).unwrap();
        let expected: Value =
            serde_json::from_slice(&fs::read(fixture.join("../input/request.json")).unwrap())
                .unwrap();
        let actual =
            prepare_reconstruction_request(&root, &repo.join("../skills/work"), &[], &request)
                .unwrap();
        assert_eq!(actual, expected);
        let expected_preview: Value =
            serde_json::from_slice(&fs::read(fixture.join("../expected/result.json")).unwrap())
                .unwrap();
        assert_eq!(
            preview_migration(&root, &repo.join("../skills/work"), &[], &actual).unwrap(),
            expected_preview
        );
    }
    #[test]
    fn reconstruction_preserves_verified_snapshot_and_requires_all_reviewed_sources() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/cases/specification/update/collection-summary/project");
        let root = std::env::temp_dir().join(format!(
            "work-reconstruction-snapshot-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut request: Value = serde_json::from_slice(&fs::read(repo.join("crates/work-infrastructure/fixtures/cases/specification/migration/reconstruction/input/semantic-request.json")).unwrap()).unwrap();
        let index: Value = serde_json::from_slice(
            &fs::read(fixture.join("outputs/work/tasks/example/index.json")).unwrap(),
        )
        .unwrap();
        request["task_context"]["source"] = index["source"].clone();
        let mut paths = work_feature::task::source::evidence_paths(&index).unwrap();
        paths.extend([
            "outputs/work/tasks/example/index.json".into(),
            "outputs/work/tasks/example/tasks/TASK-001.json".into(),
            "outputs/work/executions/example/index.json".into(),
        ]);
        let mut sources = Vec::new();
        let mut original = BTreeMap::new();
        for path in paths {
            let raw = fs::read(fixture.join(&path)).unwrap();
            let target = root.join(&path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, &raw).unwrap();
            sources.push(json!({"path":path,"raw_sha256":fingerprint::raw(&raw)}));
            original.insert(path, raw);
        }
        request["sources"] = json!(sources);
        let prepared = prepare_reconstruction_request(
            &root,
            &repo.join("../skills/work"),
            &[],
            &serde_json::to_vec(&request).unwrap(),
        )
        .unwrap();
        let candidate = prepared["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["kind"] == "task_index")
            .unwrap();
        assert_eq!(candidate["content"]["source"], index["source"]);
        assert!(
            prepared["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["kind"] != "plan")
        );
        assert_eq!(
            preview_migration(&root, &repo.join("../skills/work"), &[], &prepared).unwrap()["status"],
            "ready"
        );
        let mut incomplete = request.clone();
        incomplete["sources"]
            .as_array_mut()
            .unwrap()
            .retain(|row| !row["path"].as_str().unwrap().ends_with("/source.txt"));
        assert_eq!(
            prepare_reconstruction_request(
                &root,
                &repo.join("../skills/work"),
                &[],
                &serde_json::to_vec(&incomplete).unwrap()
            )
            .unwrap_err()
            .reason_code,
            "migration_source_review_incomplete"
        );
        for (path, raw) in original {
            assert_eq!(fs::read(root.join(path)).unwrap(), raw);
        }
    }
}
