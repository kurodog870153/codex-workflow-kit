//! Whole-set file preparation and lifecycle decisions over explicit storage capabilities.
use super::ExecutionWriterContext;
use crate::error::{ExitCode, WorkError};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use work_model::execution::file_transaction::*;
use work_model::runtime::{RuntimeBytes, RuntimeTarget};
use work_model::schema::PublicSchema;
use work_operations::derivation::{file_transaction as derive, fingerprint};

pub fn error(reason: &str) -> WorkError {
    WorkError::new(
        ExitCode::ArtifactIntegrity,
        reason,
        "Project file publication is not verified. Preserve complete evidence and return to preparation or authorized recovery.",
        json!({"recovery_required":true}),
    )
}

pub trait FileTransactionRepository {
    fn check_resources(&self, paths: &[String]) -> Result<(), WorkError>;
    fn execute_selection(&self, task: &Value, attempt: &Value) -> Result<Value, WorkError>;
    fn read_source(&self, path: &str) -> Result<Vec<u8>, WorkError>;
    fn inspect(&self, path: &str) -> Result<Option<FileState>, WorkError>;
    fn check_prepare(&self, context: &ExecutionWriterContext) -> Result<(), WorkError>;
    fn apply(
        &self,
        context: &ExecutionWriterContext,
        preview: &FileTransactionPreview,
        evidence: &str,
    ) -> Result<FileTransactionResult, WorkError>;
    fn recovery_preview(
        &self,
        context: &ExecutionWriterContext,
        identity: &str,
    ) -> Result<FileRecoveryPreview, WorkError>;
    fn restore(
        &self,
        context: &ExecutionWriterContext,
        preview: &FileRecoveryPreview,
        evidence: &str,
    ) -> Result<FileTransactionResult, WorkError>;
}

pub fn prepare(
    repository: &impl FileTransactionRepository,
    context: &ExecutionWriterContext,
    attempt_id: &str,
    staged_files: &BTreeMap<String, String>,
) -> Result<FileTransactionPreview, WorkError> {
    repository.check_prepare(context)?;
    let target = context.target();
    let task_context = context.task_context();
    let index_path = format!("{}/index.json", target.execution_dir);
    let attempt_path = format!(
        "{}/{}/{}/attempt.json",
        target.execution_dir, target.task_id, attempt_id
    );
    let index_raw = repository.read_source(&index_path)?;
    let attempt_raw = repository.read_source(&attempt_path)?;
    let parse = |bytes: &[u8]| {
        work_operations::canonical::parse_json_contract(bytes)
            .map_err(|_| error("file_transaction_source_invalid"))
    };
    let index = parse(&index_raw)?;
    let attempt = parse(&attempt_raw)?;
    context.check_execution_index(&index, &index_raw)?;
    work_operations::execution::attempt::validate_attempt_bytes(&attempt, &attempt_raw)
        .map_err(|e| error(e.reason_code))?;
    work_operations::execution::validate_execution_identity(
        &task_context.collection,
        &task_context.validation,
        &index,
        &attempt,
        target.task_id,
    )
    .map_err(|e| error(e.reason_code))?;
    if attempt["status"] != "in_progress"
        || index["lock"]["task_id"] != target.task_id
        || index["lock"]["attempt_id"] != attempt_id
        || !index["lock"]["record_id"].is_null()
    {
        return Err(error("file_transaction_attempt_not_idle"));
    }
    let task = task_context.contract["tasks"]
        .as_array()
        .and_then(|tasks| tasks.iter().find(|t| t["id"] == target.task_id))
        .ok_or_else(|| error("file_transaction_task_missing"))?;
    verify_execute(repository, task, &attempt)?;
    for dep in task["dependencies"].as_array().into_iter().flatten() {
        if !index["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|t| t["id"] == *dep && t["status"] == "completed")
        {
            return Err(error("file_transaction_dependency_incomplete"));
        }
    }
    let mut declared = BTreeMap::new();
    for file in task["files"].as_array().into_iter().flatten() {
        match file["action"].as_str() {
            Some("create" | "modify") => {
                declared.insert(
                    file["path"].as_str().expect("validated path").to_owned(),
                    file["action"].as_str().unwrap().to_owned(),
                );
            }
            Some("move") => {
                declared.insert(
                    file["source"].as_str().expect("validated path").to_owned(),
                    "remove".into(),
                );
                declared.insert(
                    file["destination"]
                        .as_str()
                        .expect("validated path")
                        .to_owned(),
                    "create".into(),
                );
            }
            _ => return Err(error("file_transaction_action_unsupported")),
        }
    }
    if declared.is_empty()
        || declared
            .iter()
            .filter(|(_, a)| a.as_str() != "remove")
            .map(|(p, _)| p)
            .collect::<BTreeSet<_>>()
            != staged_files.keys().collect()
    {
        return Err(error("file_transaction_incomplete_set"));
    }
    repository.check_resources(
        &declared
            .keys()
            .chain(staged_files.values())
            .cloned()
            .collect::<Vec<_>>(),
    )?;
    let trace: work_model::task::discussion_trace::DiscussionTrace =
        serde_json::from_value(task_context.index["discussion"].clone())
            .map_err(|_| error("file_transaction_planning_review_required"))?;
    let review_path = format!(
        "outputs/work/discussions/{}/history/{}/session.json",
        context.writer().requirement_id.as_str(),
        trace.revision
    );
    let review_raw = repository.read_source(&review_path)?;
    let session: work_model::discussion::DiscussionSession = serde_json::from_slice(&review_raw)
        .map_err(|_| error("file_transaction_planning_review_required"))?;
    if session.commit.content_sha256 != trace.content_sha256
        || session.revision != trace.revision
        || session.requirement_id != context.writer().requirement_id.as_str()
    {
        return Err(error("file_transaction_review_binding_mismatch"));
    }
    work_operations::discussion::ready_to_generate(&session).map_err(|e| error(e.0))?;
    let reviewed =
        work_operations::discussion::assembly::task_records(&session).map_err(|e| error(e.0))?;
    if !work_operations::task::file_dependencies::matches_reviewed_ancestry(
        &task_context.collection,
        &reviewed,
        context.target().task_id,
    ) || !work_operations::task::file_dependencies::matches_reviewed_source(
        &session,
        &task_context.collection,
    ) {
        return Err(error("file_transaction_review_binding_mismatch"));
    }
    work_operations::execution::authorization::require_modified_files(
        &attempt,
        &declared.keys().cloned().collect::<Vec<_>>(),
        None,
    )
    .map_err(|e| error(e.reason_code))?;
    let mut targets = Vec::new();
    let mut before_metadata = BTreeMap::new();
    let mut after_metadata = BTreeMap::new();
    let mut source_sha256: BTreeMap<_, _> = task_context
        .sources
        .iter()
        .map(|(p, b)| (p.clone(), fingerprint::raw(b)))
        .collect();
    source_sha256.insert(review_path.clone(), fingerprint::raw(&review_raw));
    for path in crate::task::source::evidence_paths(&task_context.collection)? {
        source_sha256.insert(
            path.clone(),
            fingerprint::raw(&repository.read_source(&path)?),
        );
    }
    for path in
        work_operations::execution::file_transaction::input_paths(&task_context.contract, task)
            .map_err(|e| error(e.reason_code))?
    {
        source_sha256.insert(
            path.clone(),
            fingerprint::raw(&repository.read_source(&path)?),
        );
    }
    source_sha256.insert(index_path, fingerprint::raw(&index_raw));
    source_sha256.insert(attempt_path, fingerprint::raw(&attempt_raw));
    let mut identities = BTreeSet::new();
    for (path, action) in declared {
        if path.starts_with("outputs/work/")
            || path.starts_with(".git/")
            || !work_operations::derivation::identity::runtime_relative_path(&path)
            || !identities.insert(work_operations::canonical::portable_path_identity(&path))
        {
            return Err(error("file_transaction_target_unsupported"));
        }
        let before = repository.inspect(&path)?;
        if (action == "create") == before.is_some() {
            return Err(error("file_transaction_precondition"));
        }
        let after = if action == "remove" {
            None
        } else {
            let staged = &staged_files[&path];
            if declared_path_overlap(staged, &identities)
                || !staged.starts_with("outputs/work/transactions/")
            {
                return Err(error("file_transaction_staging_scope"));
            }
            let state = repository
                .inspect(staged)?
                .ok_or_else(|| error("file_transaction_staging_missing"))?;
            source_sha256.insert(staged.clone(), fingerprint::raw(&state.bytes));
            Some(state)
        };
        before_metadata.insert(path.clone(), before.as_ref().map(|s| s.metadata.clone()));
        after_metadata.insert(path.clone(), after.as_ref().map(|s| s.metadata.clone()));
        let snapshot = |state: FileState| RuntimeBytes {
            sha256: fingerprint::raw(&state.bytes),
            bytes: state.bytes,
        };
        targets.push(RuntimeTarget {
            path,
            before: before.map(snapshot),
            after: after.map(snapshot),
        });
    }
    let binding = FileTransactionBinding {
        task_id: target.task_id.into(),
        attempt_id: attempt_id.into(),
        source_sha256,
        staged_files: staged_files.clone(),
        before_metadata,
        after_metadata,
    };
    for (p, h) in &binding.source_sha256 {
        if fingerprint::raw(&repository.read_source(p)?) != *h {
            return Err(error("file_transaction_source_drift"));
        }
    }
    derive::preview(
        &context.writer().canonical_project_root.to_string_lossy(),
        context.writer().requirement_id.as_str(),
        target.execution_dir,
        binding,
        targets,
    )
    .map_err(error)
}

pub fn verify_execute(
    repository: &impl FileTransactionRepository,
    task: &Value,
    attempt: &Value,
) -> Result<(), WorkError> {
    let selection = repository.execute_selection(task, attempt)?;
    if selection["selected_paths"] != task["instruction_selection"]["selected_paths"]
        || selection["resolved_paths"] != task["instruction_selection"]["resolved_paths"]
        || selection["instructions_sha256"] != attempt["execute_instructions_sha256"]
    {
        return Err(error("file_transaction_execute_instructions_stale"));
    }
    Ok(())
}

fn declared_path_overlap(path: &str, identities: &BTreeSet<String>) -> bool {
    identities.contains(&work_operations::canonical::portable_path_identity(path))
}

pub fn run(
    repository: &impl FileTransactionRepository,
    context: &ExecutionWriterContext,
    operation: &str,
    request: &Value,
) -> Result<Value, WorkError> {
    let request: FileTransactionRequest = serde_json::from_value(request.clone())
        .map_err(|_| error("file_transaction_request_invalid"))?;
    if request.schema != PublicSchema::WorkFileTransactionRequest {
        return Err(error("file_transaction_request_schema"));
    }
    let output = match (operation, request.command) {
        (
            "file-prepare",
            FileTransactionCommand::Prepare {
                attempt_id,
                staged_files,
            },
        ) => json!(prepare(repository, context, &attempt_id, &staged_files)?),
        (
            "file-apply",
            FileTransactionCommand::Apply {
                preview,
                approved_sha256,
                authorization_evidence,
            },
        ) => {
            if preview.schema != PublicSchema::WorkFileTransactionPreview
                || authorization_evidence.trim().is_empty()
                || approved_sha256 != preview.manifest.approval_sha256
            {
                return Err(error("file_transaction_approval_mismatch"));
            }
            let binding: FileTransactionBinding =
                serde_json::from_value(preview.manifest.business_identity.clone())
                    .map_err(|_| error("file_transaction_binding_invalid"))?;
            let current = prepare(
                repository,
                context,
                &binding.attempt_id,
                &binding.staged_files,
            )?;
            if current != *preview {
                return Err(error("file_transaction_approval_stale"));
            }
            json!(repository.apply(context, &current, &authorization_evidence)?)
        }
        (
            "file-recovery-prepare",
            FileTransactionCommand::RecoveryPrepare {
                transaction_identity,
            },
        ) => json!(repository.recovery_preview(context, &transaction_identity)?),
        (
            "file-restore",
            FileTransactionCommand::Restore {
                transaction_identity,
                approved_sha256,
                authorization_evidence,
            },
        ) => {
            if authorization_evidence.trim().is_empty() {
                return Err(error("file_transaction_recovery_authorization_required"));
            }
            let preview = repository.recovery_preview(context, &transaction_identity)?;
            if preview.approval_sha256 != approved_sha256 {
                return Err(error("file_transaction_recovery_approval_stale"));
            }
            json!(repository.restore(context, &preview, &authorization_evidence)?)
        }
        _ => return Err(error("file_transaction_command_mismatch")),
    };
    Ok(output)
}
