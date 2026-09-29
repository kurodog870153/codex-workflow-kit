//! Artifact compatibility migration, separate from complete-set revision migration.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::plan::default_artifact_paths;
use work_feature::ports::ArtifactStore;
use work_feature::specification::artifact_migration::{analyze_artifact, validate_candidate};
use work_model::schema::PublicSchema;
use work_model::specification::{
    ArtifactMigrationAction, ArtifactMigrationAnalysis, ArtifactMigrationDecision,
    ArtifactMigrationItem, ArtifactMigrationRequest,
};
use work_operations::canonical::{canonical_json_sha256, parse_json_contract, sha256_hex};
use work_operations::identifiers::RequirementId;
use work_operations::specification::migration_diff::unified_diff;
use work_operations::specification::transaction::{
    approval_sha256, decode_snapshot, derived_transaction_id, encode_snapshot, render_transaction,
    validate_transaction,
};

use crate::files::LocalFiles;
use crate::specification::storage::{
    execution_history_fingerprints, publish_journal, require_no_spec_update, storage_path,
    write_journal,
};
use crate::writer_lock::LocalWriterLock;
use work_feature::ports::WriterLock;

fn fail(code: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, code, message, json!({}))
}

pub fn analyze(
    root: &Path,
    requirement: &str,
    selected_kinds: &[String],
) -> Result<Value, WorkError> {
    let id: RequirementId = requirement.parse().map_err(|_| {
        fail(
            "migration_requirement_id",
            "A valid requirement ID is required.",
        )
    })?;
    let paths = default_artifact_paths(&id)
        .into_iter()
        .map(|(kind, path)| (kind.to_owned(), path))
        .collect::<BTreeMap<_, _>>();
    let requested =
        |kind: &str| selected_kinds.is_empty() || selected_kinds.iter().any(|row| row == kind);
    if selected_kinds
        .iter()
        .any(|kind| !matches!(kind.as_str(), "plan" | "task" | "execute"))
    {
        return Err(fail("migration_selection", "Select plan, task or execute."));
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
    if requested("plan") {
        inspect("plan", &paths["plan"])?;
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
    let fingerprint =
        canonical_json_sha256(&json!({"requirement_id":requirement,"items":items}))
            .map_err(|_| fail("migration_fingerprint", "Analysis cannot be fingerprinted."))?;
    let result = json!({"schema":"work-artifact-migration-analysis/v1",
        "requirement_id":requirement,"items":items,"fingerprint":fingerprint});
    serde_json::from_value::<ArtifactMigrationAnalysis>(result.clone()).map_err(|_| {
        fail(
            "invalid_contract_value",
            "Analysis does not match its contract.",
        )
    })?;
    Ok(result)
}

pub fn prepare_request(root: &Path, analysis: &Value, choices: &Value) -> Result<Value, WorkError> {
    let reviewed: ArtifactMigrationAnalysis = serde_json::from_value(analysis.clone())
        .map_err(|_| fail("migration_analysis", "A valid analysis is required."))?;
    if reviewed.schema != PublicSchema::WorkArtifactMigrationAnalysisV1 || reviewed.items.is_empty()
    {
        return Err(fail(
            "migration_analysis",
            "Analysis has no migration items.",
        ));
    }
    let mut selected = BTreeSet::new();
    for item in &reviewed.items {
        selected.insert(
            match item.kind.as_str() {
                "plan" => "plan",
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
        schema: PublicSchema::WorkArtifactMigrationRequestV1,
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
    let hash = canonical_json_sha256(&value)
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
        json!({"schema":"work-artifact-migration-prepared/v1","request_path":relative,
        "request_sha256":sha256_hex(&raw),"executable":request.executable(),"request":value}),
    )
}

fn read_request(
    root: &Path,
    relative: &str,
    approved_sha256: &str,
) -> Result<ArtifactMigrationRequest, WorkError> {
    let path = storage_path(root, relative)?;
    let raw = LocalFiles.read_raw(&path)?;
    if sha256_hex(&raw) != approved_sha256 {
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
    let paths = default_artifact_paths(&id)
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let task_directory = paths["task"]
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .ok_or_else(|| fail("migration_request_path", "The TASK directory is invalid."))?;
    for decision in &request.decisions {
        let item = &decision.item;
        let valid_path = match item.kind.as_str() {
            "plan" => item.path == paths["plan"],
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
    let hash = canonical_json_sha256(&value).map_err(|_| {
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
        canonical_json_sha256(&json!({"requirement_id":request.requirement_id,"items":items}))
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
    history: &BTreeMap<String, String>,
) -> Result<Value, WorkError> {
    let path = &decision.item.path;
    let files = json!([{"phase":10,"path":path,"operation":"replace",
        "before":encode_snapshot(before),"after":encode_snapshot(after)}]);
    let source_sha256 = BTreeMap::from([(path.clone(), sha256_hex(before))]);
    let candidate_sha256 = BTreeMap::from([(path.clone(), sha256_hex(after))]);
    let metadata = json!({"request":{"request_sha256":request_sha256,
        "analysis_fingerprint":request.analysis_fingerprint,"item_id":decision.item.id},
        "artifacts":{},"affected_task_ids":[],"history_sha256":history,
        "source_sha256":source_sha256,"candidate_sha256":candidate_sha256});
    let approval = approval_sha256(&files, &metadata);
    let id = derived_transaction_id("MIGRATION", &approval).map_err(|_| {
        fail(
            "migration_transaction",
            "The transaction ID cannot be derived.",
        )
    })?;
    let transaction = json!({"schema":"work-spec-transaction/v1","transaction_id":id,
        "approval_sha256":approval,"state":"prepared","published_count":0,
        "metadata":metadata,"files":files});
    validate_transaction(&transaction)
        .map_err(|_| fail("migration_transaction", "The transaction is invalid."))?;
    Ok(transaction)
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
        if sha256_hex(&raw) != item.source_sha256 {
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
    format!(
        "{execution}/.work-spec-migration-{}-{:03}.json",
        approved_sha256[..12].to_ascii_uppercase(),
        position + 1
    )
}

fn checked_journal(
    root: &Path,
    journal: &str,
    request: &ArtifactMigrationRequest,
    approved_sha256: &str,
    decision: &ArtifactMigrationDecision,
) -> Result<(), WorkError> {
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
        || sha256_hex(&before) != decision.item.source_sha256
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
    let paths = default_artifact_paths(&id)
        .into_iter()
        .collect::<BTreeMap<_, _>>();
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
            let status = if storage_path(root, &format!("{journal}.done"))?.is_file() {
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
            crate::specification::artifact_reconciliation::validate_expected(
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
    Ok(json!({"schema":"work-artifact-migration-preview/v1",
        "status":if ready {"ready"} else {"blocked"},
        "request_sha256":approved_sha256,"items":items,
        "relationship_error":relationship_error}))
}

pub fn execute(
    root: &Path,
    skill_root: &Path,
    configs: &[crate::skill_catalog::SkillRootConfig],
    relative: &str,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    let request = read_request(root, relative, approved_sha256)?;
    if !request.executable() {
        return Ok(
            json!({"schema":"work-artifact-migration-result/v1","status":"blocked",
            "request_sha256":approved_sha256,"items":[],"reconciliation":"blocked"}),
        );
    }
    let id: RequirementId = request.requirement_id.parse().map_err(|_| {
        fail(
            "migration_requirement_id",
            "The request requirement ID is invalid.",
        )
    })?;
    let paths = default_artifact_paths(&id)
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let execution = &paths["execution"];
    let directory = storage_path(root, execution)?;
    fs::create_dir_all(&directory).map_err(|_| {
        fail(
            "migration_execution_directory",
            "Execution directory cannot be created.",
        )
    })?;
    require_no_spec_update(root, execution, None)?;
    let lock = storage_path(root, &format!("{execution}/.work-state-writer.lock"))?;
    let _guard = LocalWriterLock.acquire(&lock)?;
    require_no_spec_update(root, execution, None)?;
    let sources = checked_sources(root, &request, execution, approved_sha256)?;
    let candidates = approved_candidates(&request)?;
    crate::specification::artifact_reconciliation::validate_expected(
        root,
        skill_root,
        configs,
        &request.requirement_id,
        &candidates,
    )?;
    let history = execution_history_fingerprints(root, execution)?;
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
        let marker = format!("{journal}.done");
        let outcome = if storage_path(root, &journal)?.is_file() {
            checked_journal(root, &journal, &request, approved_sha256, decision)
                .and_then(|_| publish_journal(root, &journal, &marker))
        } else {
            let transaction = transaction_for_file(
                &request,
                approved_sha256,
                decision,
                &sources[&decision.item.path],
                &candidate,
                &history,
            )?;
            write_journal(root, &journal, &transaction)
                .and_then(|_| publish_journal(root, &journal, &marker))
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
        match crate::specification::artifact_reconciliation::reconcile(
            root,
            skill_root,
            configs,
            &request.requirement_id,
            approved_sha256,
        ) {
            Ok(result) => result,
            Err(problem) => json!({"status":"blocked","code":problem.reason_code}),
        }
    } else {
        json!({"status":"blocked","code":"migration_items_incomplete"})
    };
    let valid = complete && reconciliation["status"] == "valid";
    Ok(json!({"schema":"work-artifact-migration-result/v1",
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
    let request = read_request(root, relative, approved_sha256)?;
    let id: RequirementId = request.requirement_id.parse().map_err(|_| {
        fail(
            "migration_requirement_id",
            "The request requirement ID is invalid.",
        )
    })?;
    let paths = default_artifact_paths(&id)
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let execution = &paths["execution"];
    let lock = storage_path(root, &format!("{execution}/.work-state-writer.lock"))?;
    let _guard = LocalWriterLock.acquire(&lock)?;
    let mut statuses = Vec::new();
    for (position, decision) in request.decisions.iter().enumerate() {
        if decision.action == ArtifactMigrationAction::Skip {
            statuses.push(json!({"id":decision.item.id,"status":"skipped"}));
            continue;
        }
        let journal = item_journal(execution, approved_sha256, position);
        let marker = format!("{journal}.done");
        let journal_path = storage_path(root, &journal)?;
        if !journal_path.is_file() {
            statuses.push(json!({"id":decision.item.id,"status":"not_started"}));
            continue;
        }
        checked_journal(root, &journal, &request, approved_sha256, decision)?;
        match publish_journal(root, &journal, &marker) {
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
        match crate::specification::artifact_reconciliation::reconcile(
            root,
            skill_root,
            configs,
            &request.requirement_id,
            approved_sha256,
        ) {
            Ok(result) => result,
            Err(problem) => json!({"status":"blocked","code":problem.reason_code}),
        }
    } else {
        json!({"status":"blocked","code":"migration_items_incomplete"})
    };
    let valid = complete && reconciliation["status"] == "valid";
    Ok(json!({"schema":"work-artifact-migration-result/v1",
        "status":if valid {"completed"} else {"incomplete"},
        "request_sha256":approved_sha256,"items":statuses,"reconciliation":reconciliation}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn plan_only_analysis_preserves_damaged_source() {
        let root = std::env::temp_dir().join(format!(
            "work-artifact-analysis-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("outputs/work/plans/example.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"broken").unwrap();
        let result = analyze(&root, "example", &["plan".into()]).unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
        assert_eq!(
            result["items"][0]["source_sha256"],
            work_operations::canonical::sha256_hex(b"broken")
        );
        assert_eq!(fs::read(path).unwrap(), b"broken");
    }

    #[test]
    fn final_decisions_persist_without_changing_formal_plan() {
        let root = std::env::temp_dir().join(format!(
            "work-artifact-prepare-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("outputs/work/plans/example.json");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        let fixture = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/specification-migration/outputs/work/plans/example.json"
        ));
        let mut legacy: Value = serde_json::from_slice(&fs::read(fixture).unwrap()).unwrap();
        legacy["schema"] = json!("work-plan/v0");
        let original = serde_json::to_vec_pretty(&legacy).unwrap();
        fs::write(&source, &original).unwrap();
        let analysis = analyze(&root, "example", &["plan".into()]).unwrap();
        assert_eq!(analysis["items"].as_array().unwrap().len(), 1);
        let choice = json!([{"id":analysis["items"][0]["id"],"action":"apply"}]);
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
            "action":"modify","content":{"schema":"work-plan/v1"}}]);
        assert_eq!(
            prepare_request(&root, &analysis, &invalid_modify)
                .unwrap_err()
                .reason_code,
            "migration_candidate_invalid"
        );
        let mut modified = analysis["items"][0]["proposed_content"].clone();
        modified["title"] = json!("Reviewed Plan");
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
        invalid_relationship["hierarchy_selection"]["selection_sha256"] = json!("0".repeat(64));
        let relationship_choice = json!([{"id":analysis["items"][0]["id"],
            "action":"modify","content":invalid_relationship}]);
        let relationship_request = prepare_request(&root, &analysis, &relationship_choice).unwrap();
        let relationship_path = relationship_request["request_path"].as_str().unwrap();
        let relationship_sha = relationship_request["request_sha256"].as_str().unwrap();
        let relationship_preview =
            preview(&root, &skill_root, &[], relationship_path, relationship_sha).unwrap();
        assert_eq!(relationship_preview["status"], "blocked");
        assert!(relationship_preview["relationship_error"].is_string());
        assert!(execute(&root, &skill_root, &[], relationship_path, relationship_sha).is_err());
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
        let published = execute(
            &root,
            &skill_root,
            &[],
            prepared["request_path"].as_str().unwrap(),
            prepared["request_sha256"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(published["status"], "completed", "{published:?}");
        assert_eq!(published["items"][0]["status"], "published");
        assert_eq!(published["reconciliation"]["status"], "valid");
        let repeated = execute(
            &root,
            &skill_root,
            &[],
            prepared["request_path"].as_str().unwrap(),
            prepared["request_sha256"].as_str().unwrap(),
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
        let recovered = recover(
            &root,
            &skill_root,
            &[],
            prepared["request_path"].as_str().unwrap(),
            prepared["request_sha256"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            recovered["items"][0]["publication_status"],
            "already_published"
        );
        assert_eq!(
            fs::read(&source).unwrap(),
            work_feature::specification::artifact_migration::candidate_bytes(
                "plan",
                &analysis["items"][0]["proposed_content"]
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
        let fixture = repo.join("crates/work-infrastructure/fixtures/specification-migration");
        let root = std::env::temp_dir().join(format!(
            "work-artifact-mixed-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let paths = [
            "outputs/work/plans/example.json",
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
        let analysis = analyze(&root, "example", &[]).unwrap();
        assert_eq!(analysis["items"].as_array().unwrap().len(), 4);
        let choices = Value::Array(
            analysis["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| json!({"id":item["id"],"action":"apply"}))
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
        assert_eq!(result["items"].as_array().unwrap().len(), 4);
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
        let fixture = repo.join("crates/work-infrastructure/fixtures/specification-migration");
        let root = std::env::temp_dir().join(format!(
            "work-artifact-partial-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let paths = [
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
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
        let analysis = analyze(&root, "example", &[]).unwrap();
        assert_eq!(analysis["items"].as_array().unwrap().len(), 3);
        let choices = Value::Array(
            analysis["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| json!({"id":item["id"],"action":"apply"}))
                .collect(),
        );
        let prepared = prepare_request(&root, &analysis, &choices).unwrap();
        let approved = prepared["request_sha256"].as_str().unwrap();
        let journal = item_journal("outputs/work/executions/example", approved, 1);
        let journal_path = root.join(journal);
        fs::create_dir_all(journal_path.parent().unwrap()).unwrap();
        fs::write(&journal_path, b"invalid journal").unwrap();
        fs::write(
            root.join(format!(
                "{}.done",
                journal_path.strip_prefix(&root).unwrap().display()
            )),
            work_operations::specification::transaction::completion_marker(b"invalid journal"),
        )
        .unwrap();
        let result = execute(
            &root,
            &repo.join("../skills/work"),
            &[],
            prepared["request_path"].as_str().unwrap(),
            approved,
        )
        .unwrap();
        assert_eq!(result["status"], "incomplete");
        assert_eq!(result["items"][0]["status"], "published");
        assert_eq!(result["items"][1]["status"], "failed");
        assert_eq!(result["items"][2]["status"], "published");
        assert_eq!(result["reconciliation"]["status"], "blocked");
        assert_ne!(fs::read(root.join(paths[0])).unwrap(), originals[0]);
        assert_eq!(fs::read(root.join(paths[1])).unwrap(), originals[1]);
        assert_ne!(fs::read(root.join(paths[2])).unwrap(), originals[2]);
        assert!(
            crate::specification::artifact_reconciliation::validate_expected(
                &root,
                &repo.join("../skills/work"),
                &[],
                "example",
                &BTreeMap::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn prepared_journal_recovers_without_rebuilding_candidate() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join(
            "crates/work-infrastructure/fixtures/specification-migration/outputs/work/plans/example.json",
        );
        let root = std::env::temp_dir().join(format!(
            "work-artifact-interrupted-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = "outputs/work/plans/example.json";
        let source = root.join(path);
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        let mut value: Value = serde_json::from_slice(&fs::read(fixture).unwrap()).unwrap();
        value["schema"] = json!("legacy/v0");
        let before = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&source, &before).unwrap();
        let analysis = analyze(&root, "example", &["plan".into()]).unwrap();
        let choices = json!([{"id":analysis["items"][0]["id"],"action":"apply"}]);
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
        )
        .unwrap();
        let journal = item_journal("outputs/work/executions/example", approved, 0);
        fs::create_dir_all(root.join("outputs/work/executions/example")).unwrap();
        write_journal(&root, &journal, &transaction).unwrap();
        assert_eq!(fs::read(&source).unwrap(), before);
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
    fn task_only_and_execute_only_keep_other_artifacts_unchanged() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/specification-migration");
        let paths = [
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ];
        for (selection, changed) in [("task", vec![1, 2]), ("execute", vec![3])] {
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
            let analysis = analyze(&root, "example", &[selection.into()]).unwrap();
            assert_eq!(analysis["items"].as_array().unwrap().len(), changed.len());
            let choices = Value::Array(
                analysis["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| json!({"id":item["id"],"action":"apply"}))
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
