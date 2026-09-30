//! One final fingerprint reconciliation after selective artifact publication.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::plan::{PlanValidationInput, default_artifact_paths, validate_plan};
use work_feature::ports::ArtifactStore;
use work_feature::skill::SkillRoot;
use work_feature::task::{CollectionInput, validate_collection};
use work_operations::canonical::{parse_json_contract, sha256_hex};
use work_operations::execution::index::{render_execution_index, validate_execution_index};
use work_operations::identifiers::RequirementId;
use work_operations::specification::transaction::{
    approval_sha256, derived_transaction_id, encode_snapshot, render_transaction,
    validate_transaction,
};
use work_operations::task::ordering::{TaskDocumentKind, render_task};

use crate::files::LocalFiles;
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::plan_storage::LocalPlanStorage;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::specification::storage::{
    execution_history_fingerprints, publish_journal, storage_path, write_journal,
};

fn fail(code: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, code, message, json!({}))
}

struct Update {
    path: String,
    before: Vec<u8>,
    after: Vec<u8>,
    phase: u64,
}

fn read_optional(
    root: &Path,
    relative: &str,
    overrides: &BTreeMap<String, Vec<u8>>,
) -> Result<Option<Vec<u8>>, WorkError> {
    if let Some(raw) = overrides.get(relative) {
        return Ok(Some(raw.clone()));
    }
    let path = storage_path(root, relative)?;
    if path.is_file() {
        LocalFiles.read_raw(&path).map(Some)
    } else {
        Ok(None)
    }
}

fn build_updates(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    requirement: &str,
    overrides: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<Update>, WorkError> {
    let id: RequirementId = requirement.parse().map_err(|_| {
        fail(
            "migration_requirement_id",
            "A valid requirement ID is required.",
        )
    })?;
    let paths = default_artifact_paths(&id)
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let plan_path = &paths["plan"];
    let index_path = &paths["task"];
    let execution_path = format!("{}/index.json", paths["execution"]);
    let plan_raw = read_optional(root, plan_path, overrides)?;
    let index_raw = read_optional(root, index_path, overrides)?;
    let execution_raw = read_optional(root, &execution_path, overrides)?;
    if plan_raw.is_none() && (index_raw.is_some() || execution_raw.is_some()) {
        return Err(fail(
            "migration_plan_missing",
            "TASK or Execute exists without a Plan.",
        ));
    }
    if index_raw.is_none() && execution_raw.is_some() {
        return Err(fail(
            "migration_task_missing",
            "Execute exists without a TASK collection.",
        ));
    }
    let Some(plan_raw) = plan_raw else {
        return Ok(Vec::new());
    };
    let plan = parse_json_contract(&plan_raw).map_err(|_| {
        fail(
            "migration_plan_invalid",
            "The installed Plan is invalid JSON.",
        )
    })?;
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
    let project_paths = LocalPlanStorage {
        project_root: root.to_path_buf(),
    };
    validate_plan(
        &hierarchy,
        &skills,
        &project_paths,
        &roots,
        &plan,
        PlanValidationInput {
            raw: &plan_raw,
            actual_plan_path: plan_path,
            allow_task_index: true,
        },
    )?;
    let mut updates = Vec::new();
    let Some(index_raw) = index_raw else {
        return Ok(updates);
    };
    let mut index = parse_json_contract(&index_raw).map_err(|_| {
        fail(
            "migration_task_invalid",
            "The installed TASK index is invalid JSON.",
        )
    })?;
    let references = index["tasks"]
        .as_array()
        .ok_or_else(|| {
            fail(
                "migration_task_invalid",
                "The TASK index has no item references.",
            )
        })?
        .clone();
    let directory = index_path
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .ok_or_else(|| fail("migration_task_path", "The TASK directory is invalid."))?;
    let mut item_raw = BTreeMap::new();
    for row in &references {
        let task_id = row["id"]
            .as_str()
            .ok_or_else(|| fail("migration_task_invalid", "A TASK reference has no ID."))?;
        let relative = row["path"]
            .as_str()
            .ok_or_else(|| fail("migration_task_invalid", "A TASK reference has no path."))?;
        if relative != format!("tasks/{task_id}.json") {
            return Err(fail("migration_task_path", "A TASK item path is invalid."));
        }
        let path = format!("{directory}/{relative}");
        let raw = read_optional(root, &path, overrides)?.ok_or_else(|| {
            fail(
                "migration_task_missing",
                "A referenced TASK item is missing.",
            )
        })?;
        let value = parse_json_contract(&raw).map_err(|_| {
            fail(
                "migration_task_invalid",
                "An installed TASK item is invalid JSON.",
            )
        })?;
        work_feature::specification::artifact_migration::validate_candidate(
            "task_item",
            &path,
            &value,
        )
        .map_err(|_| {
            fail(
                "migration_task_invalid",
                "An installed TASK item fails current validation.",
            )
        })?;
        item_raw.insert(task_id.to_owned(), raw);
    }
    index["source_plan"]["canonical_sha256"] = json!(sha256_hex(&plan_raw));
    index["source_plan"]["hierarchy_selection_sha256"] =
        plan["hierarchy_selection"]["selection_sha256"].clone();
    for row in index["tasks"].as_array_mut().ok_or_else(|| {
        fail(
            "migration_task_invalid",
            "The TASK index has no item references.",
        )
    })? {
        let task_id = row["id"]
            .as_str()
            .ok_or_else(|| fail("migration_task_invalid", "A TASK reference has no ID."))?;
        row["canonical_sha256"] = json!(sha256_hex(&item_raw[task_id]));
    }
    let mut new_index = render_task(&index, TaskDocumentKind::Index).map_err(|_| {
        fail(
            "migration_task_invalid",
            "The TASK index cannot be rendered.",
        )
    })?;
    let validation = validate_collection(
        &hierarchy,
        &skills,
        &project_paths,
        &roots,
        CollectionInput {
            index_raw: &new_index,
            item_raw: &item_raw,
            index_path,
            source_plan_raw: &plan_raw,
        },
    )?;
    if new_index != index_raw {
        updates.push(Update {
            path: index_path.clone(),
            before: index_raw,
            after: std::mem::take(&mut new_index),
            phase: 30,
        });
    }
    let Some(execution_raw) = execution_raw else {
        return Ok(updates);
    };
    let mut execution = parse_json_contract(&execution_raw).map_err(|_| {
        fail(
            "migration_execution_invalid",
            "The installed Execute index is invalid JSON.",
        )
    })?;
    if execution.get("lock").is_some() {
        return Err(fail(
            "migration_execution_locked",
            "An active Execute lock blocks reconciliation.",
        ));
    }
    let ids = validation["task_ids"]
        .as_array()
        .ok_or_else(|| fail("migration_task_invalid", "Validated TASK IDs are missing."))?;
    let rows = execution["tasks"].as_array_mut().ok_or_else(|| {
        fail(
            "migration_execution_invalid",
            "Execute TASK rows are missing.",
        )
    })?;
    if rows.len() != ids.len()
        || rows
            .iter()
            .any(|row| !ids.iter().any(|id| *id == row["id"]))
    {
        return Err(fail(
            "migration_execution_binding",
            "Execute TASK rows differ from the installed collection.",
        ));
    }
    execution["task_spec_id"] = validation["spec_id"].clone();
    execution["task_collection_sha256"] = validation["task_collection_sha256"].clone();
    execution["task_index_sha256"] = validation["task_index_sha256"].clone();
    execution["task_instructions_sha256"] = validation["instructions_sha256"].clone();
    execution["hierarchy_selection_sha256"] = validation["hierarchy_selection_sha256"].clone();
    execution["skill_selection_sha256"] = validation["skill_selection_sha256"].clone();
    for row in execution["tasks"].as_array_mut().expect("checked rows") {
        let task_id = row["id"].as_str().expect("checked TASK ID").to_owned();
        if row["skill_id"] != validation["task_skill_ids"][&task_id] {
            return Err(fail(
                "migration_execution_binding",
                "Execute skill assignment differs from TASK.",
            ));
        }
        row["task_item_sha256"] = validation["task_item_sha256"][&task_id].clone();
        row["instructions_sha256"] = validation["task_instructions_sha256"][&task_id].clone();
    }
    let new_execution = render_execution_index(&execution).map_err(|_| {
        fail(
            "migration_execution_invalid",
            "The Execute index cannot be rendered.",
        )
    })?;
    validate_execution_index(&execution, &new_execution).map_err(|_| {
        fail(
            "migration_execution_invalid",
            "The Execute index fails current validation.",
        )
    })?;
    if new_execution != execution_raw {
        updates.push(Update {
            path: execution_path,
            before: execution_raw,
            after: new_execution,
            phase: 40,
        });
    }
    Ok(updates)
}

pub fn validate_expected(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    requirement: &str,
    candidates: &BTreeMap<String, Vec<u8>>,
) -> Result<(), WorkError> {
    build_updates(root, skill_root, configs, requirement, candidates)?;
    Ok(())
}

pub fn reconcile(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    requirement: &str,
    request_sha256: &str,
) -> Result<Value, WorkError> {
    let id: RequirementId = requirement.parse().map_err(|_| {
        fail(
            "migration_requirement_id",
            "A valid requirement ID is required.",
        )
    })?;
    let paths = default_artifact_paths(&id)
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let execution = &paths["execution"];
    let journal = format!(
        "{execution}/.work-spec-migration-{}-reconcile.json",
        request_sha256[..12].to_ascii_uppercase()
    );
    let marker = format!("{journal}.done");
    if storage_path(root, &journal)?.is_file() {
        let raw = LocalFiles.read_raw(&storage_path(root, &journal)?)?;
        let transaction = parse_json_contract(&raw).map_err(|_| {
            fail(
                "migration_reconciliation_journal",
                "The reconciliation journal is invalid JSON.",
            )
        })?;
        validate_transaction(&transaction).map_err(|_| {
            fail(
                "migration_reconciliation_journal",
                "The reconciliation journal has invalid evidence.",
            )
        })?;
        let expected_index = &paths["task"];
        let expected_execution = format!("{execution}/index.json");
        let files = transaction["files"].as_array().ok_or_else(|| {
            fail(
                "migration_reconciliation_journal",
                "The reconciliation journal has no file set.",
            )
        })?;
        if render_transaction(&transaction).ok().as_deref() != Some(raw.as_slice())
            || transaction["metadata"]["request"]
                != json!({"request_sha256":request_sha256,"phase":"reconciliation"})
            || transaction["metadata"]["artifacts"] != json!(paths)
            || files.is_empty()
            || files.iter().any(|file| {
                let path = file["path"].as_str().unwrap_or_default();
                !((path == expected_index && file["phase"] == 30)
                    || (path == expected_execution && file["phase"] == 40))
                    || file["operation"] != "replace"
            })
        {
            return Err(fail(
                "migration_reconciliation_request_changed",
                "Reconciliation requires the identical approved request and artifact set.",
            ));
        }
        let publication = publish_journal(root, &journal, &marker)?;
        if !build_updates(root, skill_root, configs, requirement, &BTreeMap::new())?.is_empty() {
            return Err(fail(
                "migration_reconciliation_incomplete",
                "The installed fingerprint chain is still stale.",
            ));
        }
        return Ok(json!({"status":"valid","journal":journal,
            "publication_status":publication["status"]}));
    }
    let updates = build_updates(root, skill_root, configs, requirement, &BTreeMap::new())?;
    if updates.is_empty() {
        return Ok(json!({"status":"valid","publication_status":"unchanged"}));
    }
    let files = Value::Array(
        updates
            .iter()
            .map(|update| {
                json!({
                    "phase":update.phase,"path":update.path,"operation":"replace",
                    "before":encode_snapshot(&update.before),"after":encode_snapshot(&update.after)
                })
            })
            .collect(),
    );
    let source_sha256 = updates
        .iter()
        .map(|update| (update.path.clone(), sha256_hex(&update.before)))
        .collect::<BTreeMap<_, _>>();
    let candidate_sha256 = updates
        .iter()
        .map(|update| (update.path.clone(), sha256_hex(&update.after)))
        .collect::<BTreeMap<_, _>>();
    let history = execution_history_fingerprints(root, execution)?;
    let metadata = json!({"request":{"request_sha256":request_sha256,"phase":"reconciliation"},
        "artifacts":paths,"affected_task_ids":[],"history_sha256":history,
        "source_sha256":source_sha256,"candidate_sha256":candidate_sha256});
    let approval = approval_sha256(&files, &metadata);
    let transaction_id = derived_transaction_id("RECONCILIATION", &approval).map_err(|_| {
        fail(
            "migration_reconciliation",
            "Reconciliation ID cannot be derived.",
        )
    })?;
    let transaction = json!({"schema":"work-spec-transaction/v1","transaction_id":transaction_id,
        "approval_sha256":approval,"state":"prepared","published_count":0,
        "metadata":metadata,"files":files});
    validate_transaction(&transaction).map_err(|_| {
        fail(
            "migration_reconciliation",
            "Reconciliation transaction is invalid.",
        )
    })?;
    write_journal(root, &journal, &transaction)?;
    let publication = publish_journal(root, &journal, &marker)?;
    if !build_updates(root, skill_root, configs, requirement, &BTreeMap::new())?.is_empty() {
        return Err(fail(
            "migration_reconciliation_incomplete",
            "The installed fingerprint chain is still stale.",
        ));
    }
    Ok(json!({"status":"valid","journal":journal,
        "publication_status":publication["status"]}))
}
