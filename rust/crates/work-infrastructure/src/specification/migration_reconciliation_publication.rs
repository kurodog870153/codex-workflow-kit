//! Publication orchestration for final migration reconciliation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::plan::{PlanValidationInput, default_artifact_paths, validate_plan};
use work_feature::ports::ArtifactStore;
use work_feature::skill::SkillRoot;
use work_feature::task::{CollectionInput, validate_collection};
use work_operations::canonical::parse_json_contract;
use work_operations::derivation::fingerprint;
use work_operations::derivation::graph::{
    ArtifactNode, rebind_validated_execution, reconcile_artifact_bindings,
};
use work_operations::derivation::transaction::{
    PublicationOrder, TransactionDeriver, TransactionInput, TransactionKind,
};
use work_operations::execution::index::{render_execution_index, validate_execution_index};
use work_operations::identifiers::RequirementId;
use work_operations::specification::transaction::{render_transaction, validate_transaction};

use crate::files::LocalFiles;
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::plan_storage::LocalPlanStorage;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::specification::storage::{
    execution_history_bytes, publish_journal, storage_path, write_journal,
};

fn fail(code: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, code, message, json!({}))
}

struct Update {
    path: String,
    before: Vec<u8>,
    after: Vec<u8>,
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
    let changed_roots = BTreeSet::from([ArtifactNode::PlanBytes]);
    let new_index =
        reconcile_artifact_bindings(&plan_raw, &mut index, &item_raw, None, &changed_roots)
            .map_err(|_| {
                fail(
                    "migration_task_invalid",
                    "The TASK index bindings cannot be derived.",
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
            after: new_index.clone(),
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
    reconcile_artifact_bindings(
        &plan_raw,
        &mut index,
        &item_raw,
        Some(&mut execution),
        &changed_roots,
    )
    .map_err(|_| {
        fail(
            "migration_execution_binding",
            "Execute TASK bindings cannot be derived.",
        )
    })?;
    rebind_validated_execution(&mut execution, &validation).map_err(|_| {
        fail(
            "migration_execution_binding",
            "Execute TASK rows differ from the installed collection.",
        )
    })?;
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

pub fn verify_final_chain(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    requirement: &str,
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
    if !storage_path(root, &paths["plan"])?.is_file() {
        return Err(fail(
            "migration_plan_missing",
            "The installed Plan is missing.",
        ));
    }
    if !build_updates(root, skill_root, configs, requirement, &BTreeMap::new())?.is_empty() {
        return Err(fail(
            "migration_verify_chain_mismatch",
            "The installed cross-artifact fingerprint chain is stale.",
        ));
    }
    let mut installed = BTreeMap::new();
    for relative in [
        paths["plan"].clone(),
        paths["task"].clone(),
        format!("{}/index.json", paths["execution"]),
    ] {
        let path = storage_path(root, &relative)?;
        if path.is_file() {
            installed.insert(relative, fingerprint::raw(&LocalFiles.read_raw(&path)?));
        }
    }
    Ok(json!({"status":"valid","installed_sha256":installed}))
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
    let journal = work_operations::derivation::publication::journal_path(
        execution,
        work_operations::derivation::publication::JournalKind::SpecificationMigrationReconcile(
            request_sha256,
        ),
    );
    let marker = work_operations::derivation::publication::completion_marker_path(&journal);
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
    let source = updates
        .iter()
        .map(|update| (update.path.clone(), update.before.clone()))
        .collect::<BTreeMap<_, _>>();
    let candidate = updates
        .iter()
        .map(|update| (update.path.clone(), update.after.clone()))
        .collect::<BTreeMap<_, _>>();
    let history = execution_history_bytes(root, execution)?;
    let derived = TransactionDeriver::derive(TransactionInput {
        kind: TransactionKind::Reconciliation,
        order: PublicationOrder::FinalReconciliation {
            task_index_path: paths["task"].clone(),
        },
        request: json!({"request_sha256":request_sha256,"phase":"reconciliation"}),
        artifacts: json!(paths),
        affected_task_ids: Vec::new(),
        history,
        source,
        candidate,
    })
    .map_err(|_| {
        fail(
            "migration_reconciliation",
            "Reconciliation transaction is invalid.",
        )
    })?;
    let transaction = derived.journal;
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
