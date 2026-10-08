//! Artifact compatibility migration, separate from complete-set revision migration.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::artifact_paths::default_artifact_paths;
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;
use work_feature::specification::artifact_migration::{
    analyze_artifact, validate_candidate, verify_raw_analysis,
};
use work_model::schema::PublicSchema;
use work_model::specification::{
    ArtifactMigrationAction, ArtifactMigrationAnalysis, ArtifactMigrationDecision,
    ArtifactMigrationItem, ArtifactMigrationRequest,
};
use work_operations::canonical::parse_json_contract;
use work_operations::derivation::fingerprint;
#[cfg(test)]
use work_operations::derivation::fingerprint::raw as sha256_hex;
use work_operations::derivation::snapshot::decode_snapshot;
use work_operations::derivation::transaction::{
    PublicationOrder, TransactionDeriver, TransactionInput, TransactionKind,
};
use work_operations::identifiers::RequirementId;
use work_operations::specification::migration_diff::unified_diff;
use work_operations::specification::transaction::{render_transaction, validate_transaction};

use crate::files::LocalFiles;
use crate::specification::storage::storage_path;
use crate::writer_lock::LocalWriterLock;

fn artifact_paths(id: &RequirementId) -> BTreeMap<String, String> {
    let paths = default_artifact_paths(id);
    BTreeMap::from([
        ("source".into(), paths.source),
        ("task".into(), paths.task),
        ("execution".into(), paths.execution),
    ])
}

pub(crate) fn resolved_artifact_paths(
    root: &Path,
    id: &RequirementId,
) -> Result<BTreeMap<String, String>, WorkError> {
    let defaults = artifact_paths(id);
    let discovered = crate::instruction::refresh_storage::discover_requirements(root)?;
    let Some(routes) = discovered.get(id.as_str()) else {
        return Ok(defaults);
    };
    let task = routes["task"].as_str().ok_or_else(|| {
        fail(
            "migration_artifact_context_invalid",
            "The TASK route is invalid.",
        )
    })?;
    let raw = LocalFiles.read_raw(&storage_path(root, task)?)?;
    let index = parse_json_contract(&raw).map_err(|_| {
        fail(
            "migration_artifact_context_invalid",
            "The TASK context is invalid.",
        )
    })?;
    if work_operations::task::source::validate_formal_context(&index, id.as_str()).is_err() {
        if routes == &serde_json::to_value(&defaults).expect("routes serialize") {
            return Ok(defaults);
        }
        return Err(fail(
            "migration_artifact_context_invalid",
            "Custom routes require a verifiable current TASK context.",
        ));
    }
    let mut result = BTreeMap::new();
    for key in ["source", "task", "execution"] {
        let route = routes[key]
            .as_str()
            .ok_or_else(|| fail("migration_artifact_context_invalid", "A route is missing."))?;
        storage_path(root, route)?;
        result.insert(key.to_owned(), route.to_owned());
    }
    Ok(result)
}

fn fail(code: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, code, message, json!({}))
}

fn relationship_diagnostics(
    root: &Path,
    paths: &BTreeMap<String, String>,
) -> Result<Vec<Value>, WorkError> {
    let mut diagnostics = Vec::new();
    let index_path = &paths["task"];
    let execution_path = format!("{}/index.json", paths["execution"]);
    let read = |path: &str| -> Result<Option<(Value, Vec<u8>)>, WorkError> {
        let absolute = storage_path(root, path)?;
        if !absolute.is_file() {
            return Ok(None);
        }
        let raw = LocalFiles.read_raw(&absolute)?;
        Ok(parse_json_contract(&raw).ok().map(|value| (value, raw)))
    };
    let index = read(index_path)?;
    let execution = read(&execution_path)?;
    let mut report = |code: &str, path: &str, detail: &str| {
        diagnostics.push(json!({"code":code,"path":path,"detail":detail,
            "next_command":"migration semantic-prepare","mode":"reconstruction"}));
    };
    if let Some((index, index_raw)) = &index {
        if index["schema"] == "work-task-index" {
            let requirement = index["requirement_id"].as_str().unwrap_or("");
            if let Err(error) =
                work_operations::task::source::validate_formal_context(index, requirement)
            {
                report(
                    error.reason_code,
                    index_path,
                    "TASK provenance or owned planning decisions are invalid",
                );
            } else {
                let provenance =
                    serde_json::from_value(index["source"].clone()).expect("validated provenance");
                let artifacts =
                    serde_json::from_value(index["artifacts"].clone()).expect("validated routes");
                if let Err(error) = work_feature::task::source::verify_provenance(
                    &crate::artifact_paths::LocalArtifactPaths {
                        project_root: root.to_path_buf(),
                    },
                    requirement,
                    &provenance,
                    &artifacts,
                ) {
                    report(
                        &error.reason_code,
                        index_path,
                        "TASK immutable source evidence differs from its retained binding",
                    );
                }
            }
            if let Some((execution, _)) = &execution {
                if execution["schema"] == "work-execution-index" {
                    for (code, actual, expected) in [
                        (
                            "execution_spec_mismatch",
                            &execution["task_spec_id"],
                            &index["spec_id"],
                        ),
                        (
                            "execution_index_fingerprint_mismatch",
                            &execution["task_index_sha256"],
                            &json!(fingerprint::raw(index_raw)),
                        ),
                        (
                            "execution_instruction_mismatch",
                            &execution["task_instructions_sha256"],
                            &index["instruction_selection"]["instructions_sha256"],
                        ),
                        (
                            "execution_hierarchy_mismatch",
                            &execution["hierarchy_selection_sha256"],
                            &index["hierarchy_selection"]["selection_sha256"],
                        ),
                        (
                            "execution_skill_mismatch",
                            &execution["skill_selection_sha256"],
                            &index["skill_selection"]["selection_sha256"],
                        ),
                    ] {
                        if actual != expected {
                            report(
                                code,
                                &execution_path,
                                "Execution binding differs from the installed TASK collection",
                            );
                        }
                    }
                }
            }
        }
    }
    if let Some((index, _)) = &index {
        if index["schema"] == "work-task-index" {
            let directory = index_path.rsplit_once('/').map_or("", |(parent, _)| parent);
            for row in index["tasks"].as_array().into_iter().flatten() {
                let Some(relative) = row["path"].as_str() else {
                    continue;
                };
                let path = format!("{directory}/{relative}");
                let absolute = match storage_path(root, &path) {
                    Ok(path) => path,
                    Err(_) => {
                        report("invalid_task_item_path", &path, "TASK item path is invalid");
                        continue;
                    }
                };
                if !absolute.is_file() {
                    report("missing_task_item", &path, "Indexed TASK item is absent");
                    continue;
                }
                let raw = LocalFiles.read_raw(&absolute)?;
                if row["canonical_sha256"] != fingerprint::raw(&raw) {
                    report(
                        "task_item_fingerprint_mismatch",
                        &path,
                        "Indexed TASK item SHA differs from installed bytes",
                    );
                }
            }
        }
    }
    Ok(diagnostics)
}

fn transaction_diagnostics(root: &Path, execution_dir: &str) -> Result<Vec<Value>, WorkError> {
    retained_transaction_diagnostics(root, execution_dir)
}

pub(crate) fn retained_transaction_diagnostics(
    root: &Path,
    execution: &str,
) -> Result<Vec<Value>, WorkError> {
    use crate::specification::storage::{read_retained_journal, retained_journal_paths};
    use work_operations::derivation::publication::{RuntimeOperation, parse_retained_journal_path};
    let paths = match retained_journal_paths(root, execution) {
        Ok(paths) => paths,
        Err(problem) => {
            return Ok(vec![json!({"code":problem.reason_code,"path":execution,
            "detail":"Journal layout requires explicit review before any publication",
            "next_command":"manual review","mode":"blocked"})]);
        }
    };
    let mut diagnostics = Vec::new();
    for path in paths {
        match read_retained_journal(root, execution, &path) {
            Ok(evidence)
                if evidence.completion
                    == work_operations::specification::transaction::CompletionState::Completed => {}
            Ok(_) => {
                let address = parse_retained_journal_path(execution, &path).map_err(|_| {
                    fail(
                        "journal_layout_identity",
                        "The journal identity is invalid.",
                    )
                })?;
                let command = match address.operation {
                    RuntimeOperation::SpecificationUpdate => "specification recover",
                    RuntimeOperation::SpecificationMigration
                    | RuntimeOperation::SpecificationMigrationItem
                    | RuntimeOperation::SpecificationMigrationReconcile => "migration recover",
                    _ => "manual review",
                };
                diagnostics.push(json!({"code":"incomplete_transaction","path":path,
                    "detail":"A retained journal has no complete committed marker",
                    "next_command":command,"mode":"recover"}));
            }
            Err(problem) => diagnostics.push(json!({"code":if problem.reason_code=="invalid_contract_value" {"invalid_transaction"} else {&problem.reason_code},"cause":problem.reason_code,"path":path,
                "detail":"Retained journal evidence is invalid or incomplete",
                "next_command":"manual review","mode":"blocked"})),
        }
    }
    Ok(diagnostics)
}

pub fn analyze(
    root: &Path,
    requirement: &str,
    selected_kinds: &[String],
) -> Result<Value, WorkError> {
    analyze_with_evidence(root, requirement, selected_kinds, &[])
}

pub fn analyze_with_evidence(
    root: &Path,
    requirement: &str,
    selected_kinds: &[String],
    evidence_paths: &[String],
) -> Result<Value, WorkError> {
    let id: RequirementId = requirement.parse().map_err(|_| {
        fail(
            "migration_requirement_id",
            "A valid requirement ID is required.",
        )
    })?;
    let paths = resolved_artifact_paths(root, &id)?;
    let requested =
        |kind: &str| selected_kinds.is_empty() || selected_kinds.iter().any(|row| row == kind);
    if selected_kinds
        .iter()
        .any(|kind| !matches!(kind.as_str(), "source" | "task" | "execute"))
    {
        return Err(fail(
            "migration_selection",
            "Select source, task or execute.",
        ));
    }
    let mut items = Vec::<ArtifactMigrationItem>::new();
    let mut inspect = |kind: &str, path: &str| -> Result<(), WorkError> {
        let absolute = storage_path(root, path)?;
        if !absolute.is_file() {
            return Ok(());
        }
        let raw = LocalFiles.read_raw(&absolute)?;
        let item = analyze_artifact(kind, path, &raw);
        if !item.issue.is_empty() {
            items.push(item);
        }
        Ok(())
    };
    if requested("source") {
        let source_root = storage_path(root, &paths["source"])?;
        if source_root.is_dir() {
            let mut entries = fs::read_dir(source_root)
                .map_err(|_| fail("migration_source_read", "Source evidence cannot be listed."))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| fail("migration_source_read", "Source evidence cannot be listed."))?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let name = entry.file_name().to_string_lossy().into_owned();
                let Ok(source_id) = name.parse() else {
                    continue;
                };
                use work_feature::ports::SourceSnapshotReader;
                let reader = crate::source_snapshot_storage::LocalSourceSnapshotStorage {
                    project_root: root.to_path_buf(),
                };
                if reader
                    .read_snapshot_at(&id, &source_id, &paths["source"])
                    .is_err()
                {
                    inspect(
                        "source",
                        &format!("{}/{name}/manifest.json", paths["source"]),
                    )?;
                }
            }
        }
    }
    if requested("task") {
        inspect("task_index", &paths["task"])?;
        let directory = paths["task"]
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .ok_or_else(|| fail("migration_task_path", "The TASK directory is invalid."))?;
        let task_directory = storage_path(root, &format!("{directory}/tasks"))?;
        if task_directory.is_dir() {
            let mut entries = fs::read_dir(task_directory)
                .map_err(|_| fail("migration_task_read", "TASK items cannot be listed."))?
                .map(|entry| {
                    entry.map_err(|_| fail("migration_task_read", "TASK items cannot be listed."))
                })
                .collect::<Result<Vec<_>, _>>()?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("TASK-") && name.ends_with(".json") {
                    inspect("task_item", &format!("{directory}/tasks/{name}"))?;
                }
            }
        }
    }
    if requested("execute") {
        inspect(
            "execution_index",
            &format!("{}/index.json", paths["execution"]),
        )?;
    }
    let mut seen_evidence = BTreeSet::new();
    for path in evidence_paths {
        if !seen_evidence.insert(path) {
            return Err(fail(
                "migration_evidence_duplicate",
                "Evidence paths must be unique.",
            ));
        }
        let absolute = storage_path(root, path)?;
        let raw = LocalFiles.read_raw(&absolute)?;
        let item = analyze_artifact("raw_evidence", path, &raw);
        if items.iter().any(|row| row.path == *path) {
            return Err(fail(
                "migration_evidence_duplicate",
                "Evidence paths must not duplicate an installed target.",
            ));
        }
        items.push(item);
    }
    let mut diagnostics = if requested("task") || requested("execute") {
        let mut diagnostics = relationship_diagnostics(root, &paths)?;
        diagnostics.extend(transaction_diagnostics(root, &paths["execution"])?);
        diagnostics
    } else {
        Vec::new()
    };
    diagnostics.extend(crate::specification::layout_migration::diagnostics(
        root,
        &id,
        &paths["source"],
        &paths["execution"],
    )?);
    let evidence = if diagnostics.is_empty() {
        json!({"requirement_id":requirement,"items":items})
    } else {
        json!({"requirement_id":requirement,"items":items,"diagnostics":diagnostics})
    };
    let fingerprint = fingerprint::structured(&evidence)
        .map_err(|_| fail("migration_fingerprint", "Analysis cannot be fingerprinted."))?;
    let result = json!({"schema":"work-artifact-migration-analysis",
        "requirement_id":requirement,"items":items,"fingerprint":fingerprint,
        "diagnostics":diagnostics});
    serde_json::from_value::<ArtifactMigrationAnalysis>(result.clone()).map_err(|_| {
        fail(
            "invalid_contract_value",
            "Analysis does not match its contract.",
        )
    })?;
    Ok(result)
}

pub fn prepare_request(root: &Path, analysis: &Value, choices: &Value) -> Result<Value, WorkError> {
    let reviewed = verify_raw_analysis(analysis).map_err(|code| {
        fail(
            code,
            "The reviewed raw analysis or its fingerprint is invalid.",
        )
    })?;
    let _: Vec<work_model::specification::ArtifactMigrationChoice> =
        serde_json::from_value(choices.clone()).map_err(|_| {
            fail(
                "migration_decisions",
                "Decisions have missing or unknown fields.",
            )
        })?;
    if !reviewed.diagnostics.is_empty() {
        return Err(fail(
            "migration_reconstruction_required",
            "Broken artifact relations require reviewed reconstruction.",
        ));
    }
    if reviewed.schema != PublicSchema::WorkArtifactMigrationAnalysis || reviewed.items.is_empty() {
        return Err(fail(
            "migration_analysis",
            "Analysis has no migration items.",
        ));
    }
    let mut selected = BTreeSet::new();
    for item in &reviewed.items {
        selected.insert(
            match item.kind.as_str() {
                "source" => "source",
                "task_index" | "task_item" => "task",
                "execution_index" => "execute",
                _ => {
                    return Err(fail(
                        "migration_analysis",
                        "An analysis item kind is invalid.",
                    ));
                }
            }
            .to_owned(),
        );
    }
    let live = analyze(
        root,
        &reviewed.requirement_id,
        &selected.into_iter().collect::<Vec<_>>(),
    )?;
    if &live != analysis {
        return Err(fail(
            "migration_source_changed",
            "The reviewed analysis differs from current sources.",
        ));
    }
    let choices = choices
        .as_array()
        .ok_or_else(|| fail("migration_decisions", "Decisions must be an array."))?;
    if choices.len() != reviewed.items.len() {
        return Err(fail(
            "migration_decisions",
            "Every item requires one decision.",
        ));
    }
    let mut decisions = Vec::new();
    let mut seen = BTreeSet::new();
    for item in reviewed.items {
        let choice = choices
            .iter()
            .find(|choice| choice["id"] == item.id)
            .ok_or_else(|| fail("migration_decisions", "A migration item has no decision."))?;
        if !seen.insert(item.id.clone()) {
            return Err(fail(
                "migration_decisions",
                "Duplicate migration item IDs are invalid.",
            ));
        }
        let action: ArtifactMigrationAction = serde_json::from_value(choice["action"].clone())
            .map_err(|_| fail("migration_decisions", "Use Apply, Modify, Skip or Abort."))?;
        if action == ArtifactMigrationAction::Abort {
            return Err(fail(
                "migration_aborted",
                "Abort does not create an executable request.",
            ));
        }
        let content = choice
            .get("content")
            .filter(|value| !value.is_null())
            .cloned();
        if (action == ArtifactMigrationAction::Modify && content.is_none())
            || (action == ArtifactMigrationAction::Apply && item.proposed_content.is_none())
        {
            return Err(fail(
                "migration_candidate_missing",
                "Each replacement requires complete current-contract content reviewed by the user.",
            ));
        }
        let reason = choice["reason"].as_str().map(str::to_owned);
        let decision = ArtifactMigrationDecision {
            item,
            action,
            content,
            reason,
        };
        if let Some(candidate) = match decision.action {
            ArtifactMigrationAction::Apply => decision.item.proposed_content.as_ref(),
            ArtifactMigrationAction::Modify => decision.content.as_ref(),
            _ => None,
        } {
            validate_candidate(&decision.item.kind, &decision.item.path, candidate).map_err(
                |_| {
                    fail(
                        "migration_candidate_invalid",
                        "Approved content does not match the current local contract.",
                    )
                },
            )?;
        }
        decisions.push(decision);
    }
    let request = ArtifactMigrationRequest {
        schema: PublicSchema::WorkArtifactMigrationRequest,
        requirement_id: analysis["requirement_id"]
            .as_str()
            .unwrap_or_default()
            .into(),
        analysis_fingerprint: analysis["fingerprint"].as_str().unwrap_or_default().into(),
        decisions,
    };
    request.validate().map_err(|_| {
        fail(
            "migration_decisions",
            "Migration decisions are incomplete or invalid.",
        )
    })?;
    let value = serde_json::to_value(&request)
        .map_err(|_| fail("migration_request", "Request cannot be serialized."))?;
    let hash = fingerprint::structured(&value)
        .map_err(|_| fail("migration_request", "Request cannot be fingerprinted."))?;
    let relative = format!(
        "outputs/work/migrations/{}/{}.json",
        request.requirement_id, hash
    );
    let path = storage_path(root, &relative)?;
    let mut raw = serde_json::to_vec_pretty(&value)
        .map_err(|_| fail("migration_request", "Request cannot be rendered."))?;
    raw.push(b'\n');
    if path.is_file() {
        if LocalFiles.read_raw(&path)? != raw {
            return Err(fail(
                "migration_request_changed",
                "An existing request differs from approved content.",
            ));
        }
    } else {
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| fail("migration_request", "Request path has no directory."))?,
        )
        .map_err(|_| fail("migration_request", "Request directory cannot be created."))?;
        LocalFiles.create_new(&path, &raw)?;
    }
    Ok(
        json!({"schema":"work-artifact-migration-prepared","request_path":relative,
        "request_sha256":work_operations::derivation::fingerprint::raw(&raw),"executable":request.executable(),"request":value}),
    )
}

fn read_request(
    root: &Path,
    relative: &str,
    approved_sha256: &str,
) -> Result<ArtifactMigrationRequest, WorkError> {
    let path = storage_path(root, relative)?;
    let raw = LocalFiles.read_raw(&path)?;
    if fingerprint::raw(&raw) != approved_sha256 {
        return Err(fail(
            "migration_approval_changed",
            "The saved request differs from approval.",
        ));
    }
    let value = parse_json_contract(&raw)
        .map_err(|_| fail("migration_request", "The saved request is invalid JSON."))?;
    let request: ArtifactMigrationRequest =
        serde_json::from_value(value.clone()).map_err(|_| {
            fail(
                "migration_request",
                "The saved request has an invalid shape.",
            )
        })?;
    request.validate().map_err(|_| {
        fail(
            "migration_request",
            "The saved request has invalid decisions.",
        )
    })?;
    let id: RequirementId = request.requirement_id.parse().map_err(|_| {
        fail(
            "migration_requirement_id",
            "The request requirement ID is invalid.",
        )
    })?;
    let paths = resolved_artifact_paths(root, &id)?;
    let task_directory = paths["task"]
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .ok_or_else(|| fail("migration_request_path", "The TASK directory is invalid."))?;
    for decision in &request.decisions {
        let item = &decision.item;
        if item.source_size != item.raw.len() as u64
            || fingerprint::raw(&item.raw) != item.source_sha256
        {
            return Err(fail(
                "migration_raw_evidence_invalid",
                "Saved raw evidence is invalid.",
            ));
        }
        let valid_path = match item.kind.as_str() {
            "source" => {
                item.path.starts_with(&format!("{}/", paths["source"]))
                    && item.path.ends_with("/manifest.json")
            }
            "task_index" => item.path == paths["task"],
            "execution_index" => item.path == format!("{}/index.json", paths["execution"]),
            "task_item" => {
                item.path.starts_with(&format!("{task_directory}/tasks/"))
                    && item.path.ends_with(".json")
                    && item
                        .path
                        .rsplit('/')
                        .next()
                        .is_some_and(|name| name.starts_with("TASK-"))
            }
            _ => false,
        };
        if !valid_path || item.id != format!("MIGRATION-{}", item.path.replace(['/', '.'], "-")) {
            return Err(fail(
                "migration_request_path",
                "A request item is outside its artifact boundary.",
            ));
        }
    }
    let hash = fingerprint::structured(&value).map_err(|_| {
        fail(
            "migration_request",
            "The saved request cannot be fingerprinted.",
        )
    })?;
    if relative
        != format!(
            "outputs/work/migrations/{}/{}.json",
            request.requirement_id, hash
        )
    {
        return Err(fail(
            "migration_request_path",
            "The request path does not match its identity.",
        ));
    }
    let items = request
        .decisions
        .iter()
        .map(|row| &row.item)
        .collect::<Vec<_>>();
    let analysis_hash =
        fingerprint::structured(&json!({"requirement_id":request.requirement_id,"items":items}))
            .map_err(|_| {
                fail(
                    "migration_request",
                    "The item evidence cannot be fingerprinted.",
                )
            })?;
    if analysis_hash != request.analysis_fingerprint {
        return Err(fail(
            "migration_request_changed",
            "The request no longer matches its analysis.",
        ));
    }
    Ok(request)
}

fn approved_candidate(decision: &ArtifactMigrationDecision) -> Result<Vec<u8>, WorkError> {
    let content = match decision.action {
        ArtifactMigrationAction::Apply => decision.item.proposed_content.as_ref(),
        ArtifactMigrationAction::Modify => decision.content.as_ref(),
        _ => None,
    }
    .ok_or_else(|| {
        fail(
            "migration_candidate_missing",
            "The decision has no candidate.",
        )
    })?;
    validate_candidate(&decision.item.kind, &decision.item.path, content).map_err(|_| {
        fail(
            "migration_candidate_invalid",
            "The candidate fails current local validation.",
        )
    })
}

fn approved_candidates(
    request: &ArtifactMigrationRequest,
) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
    request
        .decisions
        .iter()
        .filter(|decision| decision.action != ArtifactMigrationAction::Skip)
        .map(|decision| Ok((decision.item.path.clone(), approved_candidate(decision)?)))
        .collect()
}

fn transaction_for_file(
    request: &ArtifactMigrationRequest,
    request_sha256: &str,
    decision: &ArtifactMigrationDecision,
    before: &[u8],
    after: &[u8],
    history: &BTreeMap<String, Vec<u8>>,
    execution: &str,
) -> Result<Value, WorkError> {
    let path = &decision.item.path;
    let derived = TransactionDeriver::derive(TransactionInput {
        kind: TransactionKind::Migration,
        order: PublicationOrder::Flat,
        request: json!({"request_sha256":request_sha256,
            "analysis_fingerprint":request.analysis_fingerprint,"item_id":decision.item.id}),
        artifacts: { json!({"execution":execution}) },
        affected_task_ids: Vec::new(),
        history: history.clone(),
        source: BTreeMap::from([(path.clone(), before.to_vec())]),
        candidate: BTreeMap::from([(path.clone(), after.to_vec())]),
    })
    .map_err(|_| fail("migration_transaction", "The transaction is invalid."))?;
    Ok(derived.journal)
}

fn checked_sources(
    root: &Path,
    request: &ArtifactMigrationRequest,
    execution: &str,
    approved_sha256: &str,
) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
    let mut sources = BTreeMap::new();
    for (position, decision) in request.decisions.iter().enumerate() {
        let item = &decision.item;
        if decision.action != ArtifactMigrationAction::Skip {
            let journal = item_journal(execution, approved_sha256, position);
            if storage_path(root, &journal)?.is_file() {
                continue;
            }
        }
        let raw = LocalFiles.read_raw(&storage_path(root, &item.path)?)?;
        if raw != item.raw || fingerprint::raw(&raw) != item.source_sha256 {
            return Err(fail(
                "migration_source_changed",
                "A reviewed source changed before publication.",
            ));
        }
        sources.insert(item.path.clone(), raw);
    }
    Ok(sources)
}

fn item_journal(execution: &str, approved_sha256: &str, position: usize) -> String {
    work_operations::derivation::publication::journal_path(
        execution,
        work_operations::derivation::publication::JournalKind::SpecificationMigrationItem {
            approved: approved_sha256,
            position,
        },
    )
}

fn checked_journal(
    root: &Path,
    journal: &str,
    request: &ArtifactMigrationRequest,
    approved_sha256: &str,
    decision: &ArtifactMigrationDecision,
) -> Result<(), WorkError> {
    {
        let id: RequirementId = request.requirement_id.parse().map_err(|_| {
            fail(
                "migration_requirement_id",
                "The request requirement ID is invalid.",
            )
        })?;
        let paths = resolved_artifact_paths(root, &id)?;
        let execution = &paths["execution"];
        let position = request
            .decisions
            .iter()
            .position(|row| row.item.id == decision.item.id)
            .ok_or_else(|| {
                fail(
                    "migration_recovery_request_changed",
                    "The selected item is not part of the reviewed request.",
                )
            })?;
        if item_journal(execution, approved_sha256, position) != journal {
            return Err(fail(
                "migration_recovery_request_changed",
                "The journal must identify the reviewed item position.",
            ));
        }
        crate::specification::storage::read_retained_journal_for_recovery(
            root, execution, journal,
        )?;
    }
    let raw = LocalFiles.read_raw(&storage_path(root, journal)?)?;
    let transaction = parse_json_contract(&raw).map_err(|_| {
        fail(
            "migration_recovery_journal",
            "A migration journal is invalid.",
        )
    })?;
    validate_transaction(&transaction).map_err(|_| {
        fail(
            "migration_recovery_journal",
            "A migration journal has invalid evidence.",
        )
    })?;
    let files = transaction["files"].as_array().ok_or_else(|| {
        fail(
            "migration_recovery_journal",
            "A migration journal has no file set.",
        )
    })?;
    let candidate = approved_candidate(decision)?;
    let before = files
        .first()
        .and_then(|file| decode_snapshot(&file["before"]).ok())
        .ok_or_else(|| fail("migration_recovery_journal", "Source evidence is invalid."))?;
    let after = files
        .first()
        .and_then(|file| decode_snapshot(&file["after"]).ok())
        .ok_or_else(|| {
            fail(
                "migration_recovery_journal",
                "Candidate evidence is invalid.",
            )
        })?;
    if render_transaction(&transaction).ok().as_deref() != Some(raw.as_slice())
        || transaction["metadata"]["request"]
            != json!({"request_sha256":approved_sha256,
            "analysis_fingerprint":request.analysis_fingerprint,"item_id":decision.item.id})
        || files.len() != 1
        || files[0]["path"] != decision.item.path
        || files[0]["phase"] != 10
        || files[0]["operation"] != "replace"
        || fingerprint::raw(&before) != decision.item.source_sha256
        || after != candidate
    {
        return Err(fail(
            "migration_recovery_request_changed",
            "Recovery requires the identical approved request and candidate.",
        ));
    }
    Ok(())
}

pub fn preview(
    root: &Path,
    skill_root: &Path,
    configs: &[crate::skill_catalog::SkillRootConfig],
    relative: &str,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    let request = read_request(root, relative, approved_sha256)?;
    let id: RequirementId = request.requirement_id.parse().map_err(|_| {
        fail(
            "migration_requirement_id",
            "The request requirement ID is invalid.",
        )
    })?;
    let paths = resolved_artifact_paths(root, &id)?;
    let sources = checked_sources(root, &request, &paths["execution"], approved_sha256)?;
    let mut items = Vec::new();
    for (position, decision) in request.decisions.iter().enumerate() {
        let item = &decision.item;
        let (status, diff, error) = if decision.action == ArtifactMigrationAction::Skip {
            ("skipped", String::new(), Value::Null)
        } else if storage_path(
            root,
            &item_journal(&paths["execution"], approved_sha256, position),
        )?
        .is_file()
        {
            checked_journal(
                root,
                &item_journal(&paths["execution"], approved_sha256, position),
                &request,
                approved_sha256,
                decision,
            )?;
            let journal = item_journal(&paths["execution"], approved_sha256, position);
            let status = if storage_path(
                root,
                &work_operations::derivation::publication::completion_marker_path(&journal),
            )?
            .is_file()
            {
                "published"
            } else {
                "recovery_required"
            };
            (status, String::new(), Value::Null)
        } else {
            match approved_candidate(decision) {
                Ok(candidate) => (
                    "ready",
                    unified_diff(&item.path, Some(&sources[&item.path]), Some(&candidate)),
                    Value::Null,
                ),
                Err(problem) => ("blocked", String::new(), json!(problem.reason_code)),
            }
        };
        items.push(json!({"id":item.id,"path":item.path,"status":status,
            "unified_diff":diff,"error":error}));
    }
    let ready = request.executable()
        && items
            .iter()
            .all(|item| item["status"] != "blocked" && item["status"] != "recovery_required");
    let relationship_error = if ready {
        match approved_candidates(&request).and_then(|candidates| {
            crate::specification::migration_reconciliation_publication::validate_expected(
                root,
                skill_root,
                configs,
                &request.requirement_id,
                &candidates,
            )
        }) {
            Ok(()) => Value::Null,
            Err(problem) => json!(problem.reason_code),
        }
    } else {
        Value::Null
    };
    let ready = ready && relationship_error.is_null();
    Ok(json!({"schema":"work-artifact-migration-preview",
        "status":if ready {"ready"} else {"blocked"},
        "request_sha256":approved_sha256,"items":items,
        "relationship_error":relationship_error}))
}

/// Candidate full apply/recovery scope uses the fingerprint-verified saved request.
pub fn migrate_with_runtime(
    root: &Path,
    skill_root: &Path,
    configs: &[crate::skill_catalog::SkillRootConfig],
    relative: &str,
    approved_sha256: &str,
    recovery: bool,
) -> Result<Value, WorkError> {
    let request = read_request(root, relative, approved_sha256)?;
    let context = work_feature::ports::RequirementWriterContext {
        canonical_project_root: root.canonicalize().map_err(|_| {
            fail(
                "migration_project_root",
                "The project root could not be resolved.",
            )
        })?,
        requirement_id: request.requirement_id.parse().map_err(|_| {
            fail(
                "migration_requirement_id",
                "The approved requirement is invalid.",
            )
        })?,
    };
    crate::writer_lock::require_no_legacy_locks(
        &context,
        Some(&resolved_artifact_paths(root, &context.requirement_id)?["execution"]),
    )?;
    work_feature::ports::with_runtime_writer(
        &LocalWriterLock,
        &context,
        work_model::runtime::LockClass::Execution,
        |owner| {
            if recovery {
                recover_scoped(
                    root,
                    skill_root,
                    configs,
                    relative,
                    approved_sha256,
                    Some(owner),
                )
            } else {
                execute_scoped(
                    root,
                    skill_root,
                    configs,
                    relative,
                    approved_sha256,
                    Some(owner),
                )
            }
        },
    )
}

pub fn execute(
    root: &Path,
    skill_root: &Path,
    configs: &[crate::skill_catalog::SkillRootConfig],
    relative: &str,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    migrate_with_runtime(root, skill_root, configs, relative, approved_sha256, false)
}

fn retained_artifact_context(
    root: &Path,
    request: &ArtifactMigrationRequest,
) -> Result<work_feature::ports::RequirementWriterContext, WorkError> {
    Ok(work_feature::ports::RequirementWriterContext {
        canonical_project_root: root
            .canonicalize()
            .map_err(|_| fail("journal_owner_identity", "The project root is invalid."))?,
        requirement_id: request.requirement_id.parse().map_err(|_| {
            fail(
                "migration_requirement_id",
                "The approved requirement is invalid.",
            )
        })?,
    })
}

fn retained_artifact_history(
    context: &work_feature::ports::RequirementWriterContext,
    request: &ArtifactMigrationRequest,
    approved: &str,
    execution: &str,
) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
    let mut history = crate::specification::storage::prepare_retained_batch_history(
        &context.canonical_project_root,
        execution,
        approved,
    )?;
    for decision in &request.decisions {
        if decision.action != ArtifactMigrationAction::Skip
            && history.contains_key(&decision.item.path)
        {
            history.insert(decision.item.path.clone(), decision.item.raw.clone());
        }
    }
    Ok(history)
}

fn retained_artifact_batch(
    context: &work_feature::ports::RequirementWriterContext,
    request: &ArtifactMigrationRequest,
    approved: &str,
    execution: &str,
    history: &BTreeMap<String, Vec<u8>>,
) -> Result<crate::specification::storage::RetainedJournalBatch, WorkError> {
    let journals = request
        .decisions
        .iter()
        .enumerate()
        .filter(|(_, row)| row.action != ArtifactMigrationAction::Skip)
        .map(|(position, row)| {
            Ok((
                item_journal(execution, approved, position),
                transaction_for_file(
                    request,
                    approved,
                    row,
                    &row.item.raw,
                    &approved_candidate(row)?,
                    history,
                    execution,
                )?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, WorkError>>()?;
    crate::specification::storage::prepare_retained_journal_batch(
        context, execution, approved, &journals,
    )
}

fn execute_scoped(
    root: &Path,
    skill_root: &Path,
    configs: &[crate::skill_catalog::SkillRootConfig],
    relative: &str,
    approved_sha256: &str,
    runtime_owner: Option<&work_model::runtime::RuntimeOwner>,
) -> Result<Value, WorkError> {
    execute_scoped_with_fault(
        root,
        skill_root,
        configs,
        relative,
        approved_sha256,
        runtime_owner,
        &mut |_, _| Ok(()),
    )
}

fn execute_scoped_with_fault(
    root: &Path,
    skill_root: &Path,
    configs: &[crate::skill_catalog::SkillRootConfig],
    relative: &str,
    approved_sha256: &str,
    runtime_owner: Option<&work_model::runtime::RuntimeOwner>,
    after_stage: &mut impl FnMut(
        usize,
        crate::transaction_storage::JournalRuntimeStage,
    ) -> Result<(), WorkError>,
) -> Result<Value, WorkError> {
    let request = read_request(root, relative, approved_sha256)?;
    if !request.executable() {
        return Ok(
            json!({"schema":"work-artifact-migration-result","status":"blocked",
            "request_sha256":approved_sha256,"items":[],"reconciliation":"blocked"}),
        );
    }
    let id: RequirementId = request.requirement_id.parse().map_err(|_| {
        fail(
            "migration_requirement_id",
            "The request requirement ID is invalid.",
        )
    })?;
    let paths = resolved_artifact_paths(root, &id)?;
    let execution = &paths["execution"];
    let retained_context = { Some(retained_artifact_context(root, &request)?) };
    let directory = storage_path(root, execution)?;
    fs::create_dir_all(&directory).map_err(|_| {
        fail(
            "migration_execution_directory",
            "Execution directory cannot be created.",
        )
    })?;
    checked_sources(root, &request, execution, approved_sha256)?;
    let candidates = approved_candidates(&request)?;
    crate::specification::migration_reconciliation_publication::validate_expected(
        root,
        skill_root,
        configs,
        &request.requirement_id,
        &candidates,
    )?;
    let history = {
        let context = retained_context.as_ref().expect("current retained context");

        retained_artifact_history(context, &request, approved_sha256, execution)?
    };
    let batch = retained_context
        .as_ref()
        .map(|context| {
            retained_artifact_batch(context, &request, approved_sha256, execution, &history)
        })
        .transpose()?;
    {
        let context = retained_context.as_ref().expect("current retained context");

        for (position, decision) in request.decisions.iter().enumerate() {
            if decision.action == ArtifactMigrationAction::Skip {
                continue;
            }
            let journal = item_journal(execution, approved_sha256, position);
            let transaction = transaction_for_file(
                &request,
                approved_sha256,
                decision,
                &decision.item.raw,
                &approved_candidate(decision)?,
                &history,
                execution,
            )?;
            crate::specification::storage::retained_journal_history_with_batch_owner(
                &crate::specification::storage::RetainedJournalRuntimeInput {
                    context,
                    execution,
                    relative: &journal,
                    prepared_journal: &transaction,
                    recover: true,
                },
                runtime_owner.ok_or_else(|| {
                    fail("journal_owner_identity", "The Native owner is required.")
                })?,
                batch.as_ref(),
            )?;
        }
    }
    let mut statuses = Vec::new();
    for (position, decision) in request.decisions.iter().enumerate() {
        if decision.action == ArtifactMigrationAction::Skip {
            statuses
                .push(json!({"id":decision.item.id,"path":decision.item.path,"status":"skipped"}));
            continue;
        }
        let candidate = match approved_candidate(decision) {
            Ok(raw) => raw,
            Err(problem) => {
                statuses.push(json!({"id":decision.item.id,"path":decision.item.path,
                    "status":"failed","code":problem.reason_code}));
                continue;
            }
        };
        let journal = item_journal(execution, approved_sha256, position);
        let outcome = {
            let context = retained_context.as_ref().expect("current retained context");

            let transaction = transaction_for_file(
                &request,
                approved_sha256,
                decision,
                &decision.item.raw,
                &candidate,
                &history,
                execution,
            )?;
            let recovering = storage_path(root, &journal)?.is_file();
            crate::specification::storage::publish_retained_journal_with_batch_owner(
                &crate::specification::storage::RetainedJournalRuntimeInput {
                    context,
                    execution,
                    relative: &journal,
                    prepared_journal: &transaction,
                    recover: recovering,
                },
                runtime_owner.ok_or_else(|| {
                    fail("journal_owner_identity", "The Native owner is required.")
                })?,
                batch.as_ref(),
                || Ok(()),
                |stage| after_stage(position, stage),
            )
        };
        match outcome {
            Ok(publication) => {
                statuses.push(json!({"id":decision.item.id,"path":decision.item.path,
                "status":"published","journal":journal,"publication_status":publication["status"]}))
            }
            Err(problem) => statuses.push(json!({"id":decision.item.id,"path":decision.item.path,
                "status":"failed","journal":journal,"code":problem.reason_code})),
        }
    }
    let complete = statuses
        .iter()
        .all(|row| row["status"] == "published" || row["status"] == "skipped");
    let reconciliation = if complete {
        match crate::specification::migration_reconciliation_publication::reconcile_scoped(
            root,
            skill_root,
            configs,
            &request.requirement_id,
            approved_sha256,
            runtime_owner,
        ) {
            Ok(result) => result,
            Err(problem) => json!({"status":"blocked","code":problem.reason_code}),
        }
    } else {
        json!({"status":"blocked","code":"migration_items_incomplete"})
    };
    let valid = complete && reconciliation["status"] == "valid";
    Ok(json!({"schema":"work-artifact-migration-result",
        "status":if valid {"completed"} else {"incomplete"},
        "request_sha256":approved_sha256,"items":statuses,"reconciliation":reconciliation}))
}

pub fn recover(
    root: &Path,
    skill_root: &Path,
    configs: &[crate::skill_catalog::SkillRootConfig],
    relative: &str,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    migrate_with_runtime(root, skill_root, configs, relative, approved_sha256, true)
}

fn recover_scoped(
    root: &Path,
    skill_root: &Path,
    configs: &[crate::skill_catalog::SkillRootConfig],
    relative: &str,
    approved_sha256: &str,
    runtime_owner: Option<&work_model::runtime::RuntimeOwner>,
) -> Result<Value, WorkError> {
    let request = read_request(root, relative, approved_sha256)?;
    let id: RequirementId = request.requirement_id.parse().map_err(|_| {
        fail(
            "migration_requirement_id",
            "The request requirement ID is invalid.",
        )
    })?;
    let paths = resolved_artifact_paths(root, &id)?;
    let execution = &paths["execution"];
    let retained_context = { Some(retained_artifact_context(root, &request)?) };
    if !request.executable() {
        return Err(fail(
            "migration_decisions",
            "Recovery requires an executable approved request.",
        ));
    }
    checked_sources(root, &request, execution, approved_sha256)?;
    let candidates = approved_candidates(&request)?;
    crate::specification::migration_reconciliation_publication::validate_expected(
        root,
        skill_root,
        configs,
        &request.requirement_id,
        &candidates,
    )?;
    let retained_history = retained_context
        .as_ref()
        .map(|context| retained_artifact_history(context, &request, approved_sha256, execution))
        .transpose()?;
    let batch = retained_context
        .as_ref()
        .map(|context| {
            retained_artifact_batch(
                context,
                &request,
                approved_sha256,
                execution,
                retained_history.as_ref().expect("retained history"),
            )
        })
        .transpose()?;
    let mut statuses = Vec::new();
    for (position, decision) in request.decisions.iter().enumerate() {
        if decision.action == ArtifactMigrationAction::Skip {
            statuses.push(json!({"id":decision.item.id,"status":"skipped"}));
            continue;
        }
        let journal = item_journal(execution, approved_sha256, position);
        let journal_path = storage_path(root, &journal)?;
        let staged = {
            let context = retained_context.as_ref().expect("current retained context");

            crate::execution::storage::LocalExecutionStorage {
                project_root: context.canonical_project_root.clone(),
            }
            .retained_requirement_inventory(context, execution)?
            .iter()
            .any(|item| item.manifest.business_identity["journal_path"] == journal)
        };
        if !journal_path.is_file() && !staged {
            statuses.push(json!({"id":decision.item.id,"status":"not_started"}));
            continue;
        }
        let publication = {
            let context = retained_context.as_ref().expect("current retained context");

            let transaction = transaction_for_file(
                &request,
                approved_sha256,
                decision,
                &decision.item.raw,
                &approved_candidate(decision)?,
                retained_history.as_ref().expect("retained history"),
                execution,
            )?;
            crate::specification::storage::publish_retained_journal_with_batch_owner(
                &crate::specification::storage::RetainedJournalRuntimeInput {
                    context,
                    execution,
                    relative: &journal,
                    prepared_journal: &transaction,
                    recover: true,
                },
                runtime_owner.ok_or_else(|| {
                    fail("journal_owner_identity", "The Native owner is required.")
                })?,
                batch.as_ref(),
                || Ok(()),
                |_| Ok(()),
            )
        };
        match publication {
            Ok(publication) => statuses.push(json!({"id":decision.item.id,"status":"published",
                "journal":journal,"publication_status":publication["status"]})),
            Err(problem) => statuses.push(json!({"id":decision.item.id,"status":"failed",
                "journal":journal,"code":problem.reason_code})),
        }
    }
    let complete = statuses
        .iter()
        .all(|row| row["status"] == "published" || row["status"] == "skipped");
    let reconciliation = if complete {
        match crate::specification::migration_reconciliation_publication::reconcile_scoped(
            root,
            skill_root,
            configs,
            &request.requirement_id,
            approved_sha256,
            runtime_owner,
        ) {
            Ok(result) => result,
            Err(problem) => json!({"status":"blocked","code":problem.reason_code}),
        }
    } else {
        json!({"status":"blocked","code":"migration_items_incomplete"})
    };
    let valid = complete && reconciliation["status"] == "valid";
    Ok(json!({"schema":"work-artifact-migration-result",
        "status":if valid {"completed"} else {"incomplete"},
        "request_sha256":approved_sha256,"items":statuses,"reconciliation":reconciliation}))
}

pub fn verify(
    root: &Path,
    skill_root: &Path,
    configs: &[crate::skill_catalog::SkillRootConfig],
    relative: &str,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    let request = read_request(root, relative, approved_sha256)?;
    if !request.executable() {
        return Err(fail(
            "migration_verify_request_invalid",
            "The artifact Migration request is not executable.",
        ));
    }
    let id: RequirementId = request.requirement_id.parse().map_err(|_| {
        fail(
            "migration_requirement_id",
            "The request requirement ID is invalid.",
        )
    })?;
    let paths = resolved_artifact_paths(root, &id)?;
    let execution = &paths["execution"];
    let mut results = Vec::new();
    for (position, decision) in request.decisions.iter().enumerate() {
        if decision.action == ArtifactMigrationAction::Skip {
            continue;
        }
        let journal = item_journal(execution, approved_sha256, position);
        checked_journal(root, &journal, &request, approved_sha256, decision)?;
        results.push(
            crate::specification::migration_verification::verify_published_transaction(
                root, &journal, None,
            )?,
        );
    }
    let reconciliation = work_operations::derivation::publication::journal_path(
        execution,
        work_operations::derivation::publication::JournalKind::SpecificationMigrationReconcile(
            approved_sha256,
        ),
    );
    if storage_path(root, &reconciliation)?.is_file() {
        let expected = json!({"request_sha256":approved_sha256,"phase":"reconciliation"});
        results.push(
            crate::specification::migration_verification::verify_published_transaction(
                root,
                &reconciliation,
                Some(&expected),
            )?,
        );
    }
    let chain = crate::specification::migration_reconciliation_publication::verify_final_chain(
        root,
        skill_root,
        configs,
        &request.requirement_id,
    )?;
    Ok(
        json!({"schema":"work-spec-migration-verification","status":"valid",
        "mode":"artifact","fingerprint":approved_sha256,
        "request_path":relative,"results":results,"final_chain":chain}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn copy_current_chain(root: &Path) {
        let fixture = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/cases/specification/update/collection-summary/project"
        ));
        for path in [
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(path);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(path), destination).unwrap();
        }
        crate::fixture_support::copy_fixture_sources(fixture, root).unwrap();
    }

    fn reviewed_content(path: &str) -> Value {
        serde_json::from_slice(
            &fs::read(
                Path::new(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/fixtures/cases/specification/update/collection-summary/project"
                ))
                .join(path),
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn layout_analysis_binds_custom_routes_and_refuses_legacy_approval_before_any_write() {
        let root = std::env::temp_dir().join(format!(
            "work-layout-analysis-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        copy_current_chain(&root);
        let execution = "自訂 空白/執行/example";
        let task_path = "outputs/work/tasks/example/index.json";
        let mut index: Value =
            serde_json::from_slice(&fs::read(root.join(task_path)).unwrap()).unwrap();
        index["artifacts"]["execution"] = json!(execution);
        let raw = crate::fixture_support::render_task_index(&index).unwrap();
        fs::write(root.join(task_path), &raw).unwrap();
        let old = root.join(format!(
            "{execution}/.work-spec-migration-ABCDEF123456.json"
        ));
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        let original = b"{ interrupted legacy journal\r\n";
        fs::write(&old, original).unwrap();
        let analysis = analyze(&root, "example", &["execute".into()]).unwrap();
        let report = analysis["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["code"] == "legacy_layout_review_required")
            .unwrap();
        assert_eq!(report["inventory"]["execution"], execution);
        assert_eq!(
            report["inventory"]["mappings"][0]["candidate_path"],
            format!("{execution}/journals/specification-migration/ABCDEF123456/journal.json")
        );
        assert_eq!(
            report["inventory"]["files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|file| file["path"]
                    == format!("{execution}/.work-spec-migration-ABCDEF123456.json"))
                .unwrap()["raw"],
            json!(original.as_slice())
        );
        assert_eq!(
            prepare_request(&root, &analysis, &json!([]))
                .unwrap_err()
                .reason_code,
            "migration_reconstruction_required"
        );
        assert_eq!(fs::read(&old).unwrap(), original);
        assert_eq!(fs::read(root.join(task_path)).unwrap(), raw);
        assert!(!root.join("outputs/work/migrations").exists());
        assert!(!root.join(format!("{execution}/journals")).exists());
    }

    #[test]
    fn raw_evidence_requires_exact_bytes_before_any_preparation_write() {
        let root = std::env::temp_dir().join(format!(
            "work-raw-evidence-check-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("outputs/work/tasks/example/index.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let raw = b"%PDF-1.7\r\n\xff\x00";
        fs::write(&path, raw).unwrap();
        let analysis = analyze(&root, "example", &["task".into()]).unwrap();
        assert_eq!(analysis["items"][0]["raw"], json!(raw.as_slice()));
        assert_eq!(analysis["items"][0]["resolution_status"], "needs_review");
        let mut changed = analysis.clone();
        changed["items"][0]["raw"][0] = json!(0);
        assert_eq!(
            prepare_request(&root, &changed, &json!([]))
                .unwrap_err()
                .reason_code,
            "migration_raw_evidence_invalid"
        );
        assert_eq!(fs::read(path).unwrap(), raw);
        assert!(!root.join("outputs/work/migrations").exists());
        assert!(!root.join("outputs/work/executions").exists());
    }

    #[test]
    fn task_analysis_preserves_damaged_source() {
        let root = std::env::temp_dir().join(format!(
            "work-artifact-analysis-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("outputs/work/tasks/example/index.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"broken").unwrap();
        let result = analyze(&root, "example", &["task".into()]).unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
        assert_eq!(
            result["items"][0]["source_sha256"],
            work_operations::derivation::fingerprint::raw(b"broken")
        );
        assert_eq!(fs::read(path).unwrap(), b"broken");
    }

    #[test]
    fn final_decisions_persist_without_changing_formal_task() {
        let root = std::env::temp_dir().join(format!(
            "work-artifact-prepare-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        copy_current_chain(&root);
        let source = root.join("outputs/work/tasks/example/index.json");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        let fixture = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/cases/specification/update/collection-summary/project/outputs/work/tasks/example/index.json"
        ));
        let mut legacy: Value = serde_json::from_slice(&fs::read(fixture).unwrap()).unwrap();
        legacy["schema"] = json!("work-task-index/v0");
        let original = serde_json::to_vec_pretty(&legacy).unwrap();
        fs::write(&source, &original).unwrap();
        let analysis = analyze(&root, "example", &["task".into()]).unwrap();
        assert_eq!(analysis["items"].as_array().unwrap().len(), 1);
        let choice = json!([{"id":analysis["items"][0]["id"],"action":"modify","content":reviewed_content("outputs/work/tasks/example/index.json")}]);
        let prepared = prepare_request(&root, &analysis, &choice).unwrap();
        assert_eq!(prepared["executable"], true);
        assert_eq!(fs::read(&source).unwrap(), original);
        assert_eq!(
            prepare_request(&root, &analysis, &choice).unwrap(),
            prepared
        );
        let skip = json!([{"id":analysis["items"][0]["id"],"action":"skip","reason":"Deferred"}]);
        let skipped = prepare_request(&root, &analysis, &skip).unwrap();
        assert_eq!(skipped["executable"], false);
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let skill_root = repo.join("../skills/work");
        assert_eq!(
            execute(
                &root,
                &skill_root,
                &[],
                skipped["request_path"].as_str().unwrap(),
                skipped["request_sha256"].as_str().unwrap()
            )
            .unwrap()["status"],
            "blocked"
        );
        let invalid_modify = json!([{"id":analysis["items"][0]["id"],
            "action":"modify","content":{"schema":"work-task-index"}}]);
        assert_eq!(
            prepare_request(&root, &analysis, &invalid_modify)
                .unwrap_err()
                .reason_code,
            "migration_candidate_invalid"
        );
        let mut modified = reviewed_content("outputs/work/tasks/example/index.json");
        modified["title"] = json!("Reviewed Task");
        let modify = json!([{"id":analysis["items"][0]["id"],
            "action":"modify","content":modified}]);
        let modified_request = prepare_request(&root, &analysis, &modify).unwrap();
        assert_eq!(modified_request["executable"], true);
        assert_eq!(
            modified_request["request"]["decisions"][0]["content"],
            modified
        );
        assert_ne!(
            modified_request["request_sha256"],
            prepared["request_sha256"]
        );
        let mut invalid_relationship = modified.clone();
        invalid_relationship["source"]["manifest"]["content"]["sha256"] = json!("0".repeat(64));
        let relationship_choice = json!([{"id":analysis["items"][0]["id"],
            "action":"modify","content":invalid_relationship}]);
        let relationship_request = prepare_request(&root, &analysis, &relationship_choice).unwrap();
        let relationship_path = relationship_request["request_path"].as_str().unwrap();
        let relationship_sha = relationship_request["request_sha256"].as_str().unwrap();
        let relationship_preview =
            preview(&root, &skill_root, &[], relationship_path, relationship_sha).unwrap();
        assert_eq!(relationship_preview["status"], "blocked");
        assert!(relationship_preview["relationship_error"].is_string());
        assert!(
            migrate_with_runtime(
                &root,
                &skill_root,
                &[],
                relationship_path,
                relationship_sha,
                false
            )
            .is_err()
        );
        assert_eq!(fs::read(&source).unwrap(), original);
        assert_eq!(
            prepare_request(&root, &analysis, &json!([]))
                .unwrap_err()
                .reason_code,
            "migration_decisions"
        );
        let abort = json!([{"id":analysis["items"][0]["id"],"action":"abort"}]);
        assert_eq!(
            prepare_request(&root, &analysis, &abort)
                .unwrap_err()
                .reason_code,
            "migration_aborted"
        );
        assert_eq!(fs::read(&source).unwrap(), original);
        let previewed = preview(
            &root,
            &skill_root,
            &[],
            prepared["request_path"].as_str().unwrap(),
            prepared["request_sha256"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(previewed["status"], "ready");
        assert!(
            !previewed["items"][0]["unified_diff"]
                .as_str()
                .unwrap()
                .is_empty()
        );
        assert_eq!(previewed["request_sha256"], prepared["request_sha256"]);
        assert_eq!(
            preview(
                &root,
                &skill_root,
                &[],
                prepared["request_path"].as_str().unwrap(),
                &"0".repeat(64)
            )
            .unwrap_err()
            .reason_code,
            "migration_approval_changed"
        );
        {
            use work_feature::ports::{RuntimeWriterGuard, RuntimeWriterLock};
            let saved = read_request(
                &root,
                prepared["request_path"].as_str().unwrap(),
                prepared["request_sha256"].as_str().unwrap(),
            )
            .unwrap();
            let context = work_feature::ports::RequirementWriterContext {
                canonical_project_root: root.canonicalize().unwrap(),
                requirement_id: saved.requirement_id.parse().unwrap(),
            };
            let held = LocalWriterLock
                .acquire_runtime(&context, work_model::runtime::LockClass::Execution)
                .unwrap();
            assert!(
                migrate_with_runtime(
                    &root,
                    &skill_root,
                    &[],
                    prepared["request_path"].as_str().unwrap(),
                    prepared["request_sha256"].as_str().unwrap(),
                    false
                )
                .is_err()
            );
            held.release().unwrap();
            fs::write(&source, b"unexpected source drift").unwrap();
            assert!(
                migrate_with_runtime(
                    &root,
                    &skill_root,
                    &[],
                    prepared["request_path"].as_str().unwrap(),
                    prepared["request_sha256"].as_str().unwrap(),
                    true
                )
                .is_err()
            );
            assert_eq!(fs::read(&source).unwrap(), b"unexpected source drift");
            fs::write(&source, &original).unwrap();
            LocalWriterLock
                .require_runtime_idle(&context, work_model::runtime::LockClass::Execution)
                .unwrap();
        }
        let published = migrate_with_runtime(
            &root,
            &skill_root,
            &[],
            prepared["request_path"].as_str().unwrap(),
            prepared["request_sha256"].as_str().unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(published["status"], "completed", "{published:?}");
        assert_eq!(published["items"][0]["status"], "published");
        assert_eq!(published["reconciliation"]["status"], "valid");
        let repeated = migrate_with_runtime(
            &root,
            &skill_root,
            &[],
            prepared["request_path"].as_str().unwrap(),
            prepared["request_sha256"].as_str().unwrap(),
            false,
        )
        .unwrap();
        assert_eq!(repeated["status"], "completed");
        assert_eq!(
            repeated["items"][0]["publication_status"],
            "already_published"
        );
        assert_eq!(
            preview(
                &root,
                &skill_root,
                &[],
                prepared["request_path"].as_str().unwrap(),
                prepared["request_sha256"].as_str().unwrap()
            )
            .unwrap()["items"][0]["status"],
            "published"
        );
        let recovered = migrate_with_runtime(
            &root,
            &skill_root,
            &[],
            prepared["request_path"].as_str().unwrap(),
            prepared["request_sha256"].as_str().unwrap(),
            true,
        )
        .unwrap();
        assert_eq!(
            recovered["items"][0]["publication_status"],
            "already_published"
        );
        assert_eq!(
            fs::read(&source).unwrap(),
            work_feature::specification::artifact_migration::candidate_bytes(
                "task_index",
                &reviewed_content("outputs/work/tasks/example/index.json")
            )
            .unwrap()
        );
        let saved = fs::read(root.join(prepared["request_path"].as_str().unwrap())).unwrap();
        assert_eq!(sha256_hex(&saved), prepared["request_sha256"]);
        assert_eq!(
            prepare_request(&root, &analysis, &choice)
                .unwrap_err()
                .reason_code,
            "migration_source_changed"
        );
    }

    #[test]
    fn mixed_artifacts_publish_from_one_saved_request() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/cases/specification/update/collection-summary/project");
        let root = std::env::temp_dir().join(format!(
            "work-artifact-mixed-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let paths = [
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ];
        for relative in paths {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            let mut value: Value =
                serde_json::from_slice(&fs::read(fixture.join(relative)).unwrap()).unwrap();
            value["schema"] = json!("legacy/v0");
            fs::write(destination, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        }
        crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        let analysis = analyze(&root, "example", &[]).unwrap();
        assert_eq!(analysis["items"].as_array().unwrap().len(), 3);
        let choices = Value::Array(
            analysis["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| json!({"id":item["id"],"action":"modify","content":reviewed_content(item["path"].as_str().unwrap())}))
                .collect(),
        );
        let prepared = prepare_request(&root, &analysis, &choices).unwrap();
        let request_path = prepared["request_path"].as_str().unwrap();
        let approved = prepared["request_sha256"].as_str().unwrap();
        let plan_path = root.join(paths[0]);
        let plan_bytes = fs::read(&plan_path).unwrap();
        fs::write(&plan_path, b"source drift").unwrap();
        assert_eq!(
            preview(
                &root,
                &repo.join("../skills/work"),
                &[],
                request_path,
                approved
            )
            .unwrap_err()
            .reason_code,
            "migration_source_changed"
        );
        assert_eq!(
            execute(
                &root,
                &repo.join("../skills/work"),
                &[],
                request_path,
                approved
            )
            .unwrap_err()
            .reason_code,
            "migration_source_changed"
        );
        fs::write(&plan_path, plan_bytes).unwrap();
        let result = execute(
            &root,
            &repo.join("../skills/work"),
            &[],
            request_path,
            approved,
        )
        .unwrap();
        assert_eq!(result["status"], "completed", "{result:?}");
        assert_eq!(result["items"].as_array().unwrap().len(), 3);
        assert_eq!(result["reconciliation"]["status"], "valid");
        for relative in paths {
            assert_eq!(
                fs::read(root.join(relative)).unwrap(),
                fs::read(fixture.join(relative)).unwrap()
            );
        }
    }

    #[test]
    fn failed_item_preserves_prior_publication_and_allows_later_item() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/cases/specification/update/collection-summary/project");
        let root = std::env::temp_dir().join(format!(
            "work-artifact-partial-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let paths = [
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ];
        let mut originals = Vec::new();
        for (position, relative) in paths.iter().enumerate() {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            let mut value: Value =
                serde_json::from_slice(&fs::read(fixture.join(relative)).unwrap()).unwrap();
            if position < 3 {
                value["schema"] = json!("legacy/v0");
            }
            let raw = serde_json::to_vec_pretty(&value).unwrap();
            fs::write(destination, &raw).unwrap();
            originals.push(raw);
        }
        crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        let analysis = analyze(&root, "example", &[]).unwrap();
        assert_eq!(analysis["items"].as_array().unwrap().len(), 3);
        let choices = Value::Array(
            analysis["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| json!({"id":item["id"],"action":"modify","content":reviewed_content(item["path"].as_str().unwrap())}))
                .collect(),
        );
        let prepared = prepare_request(&root, &analysis, &choices).unwrap();
        let approved = prepared["request_sha256"].as_str().unwrap();
        let journal = item_journal("outputs/work/executions/example", approved, 1);
        let journal_path = root.join(&journal);
        fs::create_dir_all(journal_path.parent().unwrap()).unwrap();
        fs::write(&journal_path, b"invalid journal").unwrap();
        let marker = work_operations::derivation::publication::completion_marker_path(&journal);
        fs::write(
            root.join(&marker),
            work_operations::derivation::publication::completion_marker(b"invalid journal"),
        )
        .unwrap();
        let result = {
            assert!(
                execute(
                    &root,
                    &repo.join("../skills/work"),
                    &[],
                    prepared["request_path"].as_str().unwrap(),
                    approved
                )
                .is_err()
            );
            for (position, path) in paths.iter().enumerate() {
                assert_eq!(fs::read(root.join(path)).unwrap(), originals[position]);
            }
            assert_eq!(fs::read(&journal_path).unwrap(), b"invalid journal");
            fs::remove_file(&journal_path).unwrap();
            fs::remove_file(root.join(&marker)).unwrap();
            fs::remove_dir(journal_path.parent().unwrap()).unwrap();
            fs::remove_dir(journal_path.parent().unwrap().parent().unwrap()).unwrap();
            fs::remove_dir(
                journal_path
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap(),
            )
            .unwrap();
            let context = work_feature::ports::RequirementWriterContext {
                canonical_project_root: root.canonicalize().unwrap(),
                requirement_id: "example".parse().unwrap(),
            };
            work_feature::ports::with_runtime_writer(&LocalWriterLock,&context,work_model::runtime::LockClass::Execution,
                |owner|execute_scoped_with_fault(&root,&repo.join("../skills/work"),&[],prepared["request_path"].as_str().unwrap(),approved,Some(owner),&mut |position,stage| {
                    if position==1 && stage==crate::transaction_storage::JournalRuntimeStage::JournalInitialized {
                        Err(fail("injected_item_fault","injected"))
                    } else {Ok(())}
                })).unwrap()
        };
        assert_eq!(result["status"], "incomplete");
        assert_eq!(result["items"][0]["status"], "published");
        assert_eq!(result["items"][1]["status"], "failed");
        assert_eq!(result["items"][2]["status"], "published");
        assert_eq!(result["reconciliation"]["status"], "blocked");
        assert_ne!(fs::read(root.join(paths[0])).unwrap(), originals[0]);
        assert_eq!(fs::read(root.join(paths[1])).unwrap(), originals[1]);
        assert_ne!(fs::read(root.join(paths[2])).unwrap(), originals[2]);
        assert!(
            crate::specification::migration_reconciliation_publication::validate_expected(
                &root,
                &repo.join("../skills/work"),
                &[],
                "example",
                &BTreeMap::new(),
            )
            .is_err()
        );
        {
            let execution = "outputs/work/executions/example";
            let published_before = paths
                .iter()
                .map(|path| fs::read(root.join(path)).unwrap())
                .collect::<Vec<_>>();
            let unknown = item_journal(execution, approved, 998);
            let unknown_path = root.join(&unknown);
            fs::create_dir_all(unknown_path.parent().unwrap()).unwrap();
            let known = item_journal(execution, approved, 0);
            fs::write(&unknown_path, fs::read(root.join(&known)).unwrap()).unwrap();
            let unknown_marker =
                work_operations::derivation::publication::completion_marker_path(&unknown);
            let known_marker =
                work_operations::derivation::publication::completion_marker_path(&known);
            fs::write(
                root.join(&unknown_marker),
                fs::read(root.join(&known_marker)).unwrap(),
            )
            .unwrap();
            let rejected = recover(
                &root,
                &repo.join("../skills/work"),
                &[],
                prepared["request_path"].as_str().unwrap(),
                approved,
            )
            .unwrap();
            assert_eq!(rejected["status"], "incomplete");
            assert!(
                rejected["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|item| item["status"] == "failed")
            );
            for (position, path) in paths.iter().enumerate() {
                assert_eq!(
                    fs::read(root.join(path)).unwrap(),
                    published_before[position]
                );
            }
            fs::remove_file(&unknown_path).unwrap();
            fs::remove_file(root.join(&unknown_marker)).unwrap();
            fs::remove_dir(unknown_path.parent().unwrap()).unwrap();
            let pending_raw = fs::read(&journal_path).unwrap();
            fs::write(&journal_path, b"foreign partial journal").unwrap();
            let rejected = recover(
                &root,
                &repo.join("../skills/work"),
                &[],
                prepared["request_path"].as_str().unwrap(),
                approved,
            )
            .unwrap();
            assert_eq!(rejected["status"], "incomplete");
            assert!(
                rejected["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|item| item["status"] == "failed")
            );
            for (position, path) in paths.iter().enumerate() {
                assert_eq!(
                    fs::read(root.join(path)).unwrap(),
                    published_before[position]
                );
            }
            assert_eq!(fs::read(&journal_path).unwrap(), b"foreign partial journal");
            fs::write(&journal_path, pending_raw).unwrap();
            let recovered = recover(
                &root,
                &repo.join("../skills/work"),
                &[],
                prepared["request_path"].as_str().unwrap(),
                approved,
            )
            .unwrap();
            assert_eq!(recovered["status"], "completed", "{recovered}");
            for position in [0, 2] {
                assert_eq!(
                    fs::read(root.join(paths[position])).unwrap(),
                    published_before[position]
                );
            }
            let context = work_feature::ports::RequirementWriterContext {
                canonical_project_root: root.canonicalize().unwrap(),
                requirement_id: "example".parse().unwrap(),
            };
            assert!(
                crate::execution::storage::LocalExecutionStorage {
                    project_root: root.clone()
                }
                .retained_requirement_inventory(&context, execution)
                .unwrap()
                .is_empty()
            );
            let final_raw = paths
                .iter()
                .map(|path| fs::read(root.join(path)).unwrap())
                .collect::<Vec<_>>();
            let again = recover(
                &root,
                &repo.join("../skills/work"),
                &[],
                prepared["request_path"].as_str().unwrap(),
                approved,
            )
            .unwrap();
            assert_eq!(again["status"], "completed");
            for (position, path) in paths.iter().enumerate() {
                assert_eq!(fs::read(root.join(path)).unwrap(), final_raw[position]);
            }
        }
    }

    #[test]
    fn prepared_journal_recovers_without_rebuilding_candidate() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join(
            "crates/work-infrastructure/fixtures/cases/specification/update/collection-summary/project/outputs/work/tasks/example/index.json",
        );
        let root = std::env::temp_dir().join(format!(
            "work-artifact-interrupted-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = "outputs/work/tasks/example/index.json";
        copy_current_chain(&root);
        let source = root.join(path);
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        let mut value: Value = serde_json::from_slice(&fs::read(fixture).unwrap()).unwrap();
        value["schema"] = json!("legacy/v0");
        let before = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&source, &before).unwrap();
        let analysis = analyze(&root, "example", &["task".into()]).unwrap();
        let choices = json!([{"id":analysis["items"][0]["id"],"action":"modify","content":reviewed_content("outputs/work/tasks/example/index.json")}]);
        let prepared = prepare_request(&root, &analysis, &choices).unwrap();
        let request: ArtifactMigrationRequest =
            serde_json::from_value(prepared["request"].clone()).unwrap();
        let approved = prepared["request_sha256"].as_str().unwrap();
        let candidate = approved_candidate(&request.decisions[0]).unwrap();
        let transaction = transaction_for_file(
            &request,
            approved,
            &request.decisions[0],
            &before,
            &candidate,
            &BTreeMap::new(),
            "outputs/work/executions/example",
        )
        .unwrap();
        let journal = item_journal("outputs/work/executions/example", approved, 0);
        fs::create_dir_all(root.join("outputs/work/executions/example")).unwrap();
        {
            let context = retained_artifact_context(&root, &request).unwrap();
            let failure=work_feature::ports::with_runtime_writer(&LocalWriterLock,&context,work_model::runtime::LockClass::Execution,
                |owner|crate::specification::storage::publish_retained_journal_with_owner(
                    &crate::specification::storage::RetainedJournalRuntimeInput {context:&context,execution:"outputs/work/executions/example",relative:&journal,prepared_journal:&transaction,recover:false},owner,||Ok(()),|stage| {
                        if stage==crate::transaction_storage::JournalRuntimeStage::JournalInitialized {Err(fail("injected_item_fault","injected"))} else {Ok(())}
                    })).unwrap_err();
            assert_eq!(failure.reason_code, "injected_item_fault");
        }
        assert_eq!(fs::read(&source).unwrap(), before);
        let source_evidence =
            work_feature::task::source::evidence_paths(&reviewed_content(path)).unwrap();
        let source_content = source_evidence
            .iter()
            .find(|path| path.ends_with("/source.txt"))
            .unwrap();
        let evidence_path = root.join(source_content);
        let original_evidence = fs::read(&evidence_path).unwrap();
        let journal_before = fs::read(root.join(&journal)).unwrap();
        let mut changed_evidence = original_evidence.clone();
        changed_evidence[0] ^= 1;
        fs::write(&evidence_path, &changed_evidence).unwrap();
        assert_eq!(
            recover(
                &root,
                &repo.join("../skills/work"),
                &[],
                prepared["request_path"].as_str().unwrap(),
                approved,
            )
            .unwrap_err()
            .reason_code,
            "source_hash_mismatch"
        );
        assert_eq!(fs::read(&source).unwrap(), before);
        assert_eq!(fs::read(root.join(&journal)).unwrap(), journal_before);
        assert_eq!(fs::read(&evidence_path).unwrap(), changed_evidence);
        fs::write(&evidence_path, original_evidence).unwrap();
        let recovered = recover(
            &root,
            &repo.join("../skills/work"),
            &[],
            prepared["request_path"].as_str().unwrap(),
            approved,
        )
        .unwrap();
        assert_eq!(recovered["status"], "completed");
        assert_eq!(fs::read(&source).unwrap(), candidate);
    }

    #[test]
    fn missing_item_and_broken_execution_binding_route_to_reconstruction() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/cases/specification/update/collection-summary/project");
        let root = std::env::temp_dir().join(format!(
            "work-artifact-relations-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for relative in [
            "outputs/work/tasks/example/index.json",
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let execution = root.join("outputs/work/executions/example/index.json");
        let mut value: Value = serde_json::from_slice(&fs::read(&execution).unwrap()).unwrap();
        value["task_spec_id"] = json!("TASK-SPEC-WRONG");
        fs::write(&execution, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        let analysis = analyze(&root, "example", &[]).unwrap();
        let diagnostics = analysis["diagnostics"].as_array().unwrap();
        assert!(
            diagnostics
                .iter()
                .any(|row| row["code"] == "missing_task_item")
        );
        assert!(
            diagnostics
                .iter()
                .any(|row| row["code"] == "execution_spec_mismatch")
        );
        assert!(
            diagnostics
                .iter()
                .all(|row| row["next_command"] == "migration semantic-prepare"
                    && row["mode"] == "reconstruction")
        );
        assert_eq!(
            prepare_request(&root, &analysis, &json!([]))
                .unwrap_err()
                .reason_code,
            "migration_reconstruction_required"
        );
        let invalid_journal = {
            root.join("outputs/work/executions/example/journals/specification-update/SPEC-UPDATE-INVALID/journal.json")
        };
        fs::create_dir_all(invalid_journal.parent().unwrap()).unwrap();
        fs::write(&invalid_journal, b"{").unwrap();
        let with_transaction = analyze(&root, "example", &[]).unwrap();
        assert!(
            with_transaction["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["code"] == "invalid_transaction" && row["mode"] == "blocked")
        );
    }

    #[test]
    fn task_only_and_execute_only_keep_other_artifacts_unchanged() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/cases/specification/update/collection-summary/project");
        let paths = [
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ];
        for (selection, changed) in [("task", vec![0, 1]), ("execute", vec![2])] {
            let root = std::env::temp_dir().join(format!(
                "work-artifact-{selection}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            for (position, relative) in paths.iter().enumerate() {
                let destination = root.join(relative);
                fs::create_dir_all(destination.parent().unwrap()).unwrap();
                let mut value: Value =
                    serde_json::from_slice(&fs::read(fixture.join(relative)).unwrap()).unwrap();
                if changed.contains(&position) {
                    value["schema"] = json!("legacy/v0");
                }
                fs::write(
                    destination,
                    if changed.contains(&position) {
                        serde_json::to_vec_pretty(&value).unwrap()
                    } else {
                        fs::read(fixture.join(relative)).unwrap()
                    },
                )
                .unwrap();
            }
            crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
            let analysis = analyze(&root, "example", &[selection.into()]).unwrap();
            assert_eq!(analysis["items"].as_array().unwrap().len(), changed.len());
            let choices = Value::Array(
                analysis["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| json!({"id":item["id"],"action":"modify","content":reviewed_content(item["path"].as_str().unwrap())}))
                    .collect(),
            );
            let prepared = prepare_request(&root, &analysis, &choices).unwrap();
            let result = execute(
                &root,
                &repo.join("../skills/work"),
                &[],
                prepared["request_path"].as_str().unwrap(),
                prepared["request_sha256"].as_str().unwrap(),
            )
            .unwrap();
            assert_eq!(result["status"], "completed", "{selection}: {result:?}");
            for relative in paths {
                assert_eq!(
                    fs::read(root.join(relative)).unwrap(),
                    fs::read(fixture.join(relative)).unwrap()
                );
            }
        }
    }
}
