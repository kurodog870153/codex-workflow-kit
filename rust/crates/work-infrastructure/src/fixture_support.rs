//! Artifact builders used by process boundary integration tests.

use serde_json::Value;
pub use work_operations::derivation::graph::ArtifactNode;

/// Prepare a complete case in memory. Protected entries include historical raw
/// evidence and deliberate negative-test damage; callers explicitly select them.
/// No file is written, including when derivation or evidence validation fails.
pub fn stage_fixture_case<F>(
    original: &std::collections::BTreeMap<String, Vec<u8>>,
    protected: &std::collections::BTreeSet<String>,
    derive: F,
) -> Result<std::collections::BTreeMap<String, Vec<u8>>, String>
where
    F: FnOnce(&mut std::collections::BTreeMap<String, Vec<u8>>) -> Result<(), String>,
{
    for path in protected {
        if !original.contains_key(path) {
            return Err(format!("missing protected fixture: {path}"));
        }
    }
    let mut candidate = original.clone();
    derive(&mut candidate)?;
    for path in protected {
        if candidate.get(path) != original.get(path) {
            return Err(format!("protected fixture changed: {path}"));
        }
    }
    Ok(candidate)
}

/// Render current Task and Execute as one candidate using Operations' topology.
/// Inputs are cloned so failed binding cannot leave a partly updated case.
pub fn rebuild_fixture_bindings(
    index: &Value,
    items: &std::collections::BTreeMap<String, Vec<u8>>,
    execution: Option<&Value>,
    changed_roots: &std::collections::BTreeSet<ArtifactNode>,
) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    let mut index = index.clone();
    let mut execution = execution.cloned();
    let index_raw = work_operations::derivation::graph::reconcile_artifact_bindings(
        &mut index,
        items,
        execution.as_mut(),
        changed_roots,
    )
    .map_err(|issue| issue.reason_code().to_owned())?;
    let execution_raw = execution
        .as_ref()
        .map(render_execution_index)
        .transpose()
        .map_err(|issue| issue.to_string())?;
    Ok((index_raw, execution_raw))
}

pub fn render_task_index(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    work_operations::task::ordering::render_task(
        value,
        work_operations::task::ordering::TaskDocumentKind::Index,
    )
}

pub fn render_task_item(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    work_operations::task::ordering::render_task(
        value,
        work_operations::task::ordering::TaskDocumentKind::Item,
    )
}

pub fn build_initial_execution_index(
    collection: &Value,
    validation: &Value,
) -> Result<Value, String> {
    work_operations::execution::index::build_initial_execution_index(collection, validation)
        .map_err(|issue| issue.reason_code.to_owned())
}

pub fn render_execution_index(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    work_operations::execution::index::render_execution_index(value)
}

/// Build historical reservation evidence without executing or granting capabilities.
pub fn historical_record_reservation(
    task: &Value,
    attempt: &Value,
    index: &Value,
    task_id: &str,
    record_id: &str,
) -> Result<Value, String> {
    work_operations::execution::record_begin_candidate(
        task, attempt, index, task_id, record_id, None,
    )
    .map(|(candidate, _, _)| candidate)
    .map_err(|issue| issue.reason_code.to_owned())
}

pub fn selection_sha256(mode: &str, skills: &[Value]) -> String {
    work_operations::derivation::fingerprint::skill_selection(mode, skills)
}

pub fn structured_sha256(value: &Value) -> String {
    work_operations::derivation::fingerprint::structured(value).expect("fixture JSON serializes")
}

pub fn raw_sha256(raw: &[u8]) -> String {
    work_operations::derivation::fingerprint::raw(raw)
}

/// Explicit reviewed replacement used only by fixture and process tests.
pub fn source_confirmation(previous: &Value, source: &Value) -> Value {
    use std::collections::BTreeSet;
    let ids = |value: &Value| {
        value["acceptance_criteria"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap().to_owned())
            .collect::<BTreeSet<_>>()
    };
    let old = ids(&previous["source"]);
    let new = ids(source);
    let reviews: serde_json::Map<String, Value> = previous["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| {
            (
                task["id"].as_str().unwrap().to_owned(),
                serde_json::from_str(include_str!(
                    "../fixtures/shared/source-task/source-confirmation-review.json"
                ))
                .expect("fixed fixture JSON"),
            )
        })
        .collect();
    serde_json::json!({"previous_planning_sha256":work_operations::derivation::fingerprint::structured(previous).unwrap(),"new_source_sha256":work_operations::derivation::fingerprint::structured(source).unwrap(),"requirement_sha256":source["snapshot"]["content"]["sha256"],"complete_requirement_review":true,"retained_acceptance_ids":old.intersection(&new).collect::<Vec<_>>(),"removed_acceptance":old.difference(&new).map(|id|serde_json::json!({"id":id,"reason":"The reviewed replacement explicitly retires this requirement."})).collect::<Vec<_>>(),"added_acceptance_ids":new.difference(&old).collect::<Vec<_>>(),"task_reviews":reviews})
}

pub fn valid_sha256(value: &str) -> bool {
    work_operations::protocol::valid_sha256(value)
}

pub fn validate_contract_example(id: &str, value: &Value) -> Result<(), String> {
    use work_operations::execution::deviation;
    let execution = match id {
        "work-execution-deviation-proposal" => Some(deviation::validate_deviation_proposal(value)),
        "work-execution-deviation" => Some(deviation::validate_deviation_artifact(value)),
        "work-execution-deviation-preview" => Some(deviation::validate_deviation_preview(value)),
        "work-execution-deviation-record" => {
            Some(deviation::validate_deviation_record_response(value))
        }
        "work-execution-deviation-semantic-request" => {
            Some(deviation::validate_semantic_deviation_request(value))
        }
        _ => None,
    };
    if let Some(result) = execution {
        return result.map_err(|issue| issue.reason_code.to_owned());
    }
    match id {
        "work-spec-prepare-request" => {
            work_operations::specification::prepare::validate_prepare_request(value)
                .map_err(|issue| issue.reason_code.to_owned())
        }
        "work-spec-verification-request" => {
            work_operations::specification::verification::validate_request(value)
                .map_err(|issue| issue.reason_code.to_owned())
        }
        _ => Err(format!("unknown contract example: {id}")),
    }
}

/// Capture exact fixture requirements and separately bind confirmed Task choices.
pub fn capture_planning_context(
    root: &std::path::Path,
    requirement: &str,
    raw: &[u8],
    hierarchy: &Value,
    skills: &Value,
    acceptance: &Value,
) -> Result<Value, work_feature::error::WorkError> {
    use work_feature::ports::SourceSnapshotWriter;
    let id = requirement.parse().map_err(|_| {
        work_feature::error::WorkError::new(
            work_feature::error::ExitCode::Contract,
            "invalid_requirement_id",
            "Fixture requires a portable requirement ID.",
            serde_json::json!({}),
        )
    })?;
    let snapshot = crate::source_snapshot_storage::LocalSourceSnapshotStorage {
        project_root: root.to_path_buf(),
    }
    .capture(
        &id,
        &work_model::source::snapshot::SnapshotSource::UserText {},
        &"source.txt"
            .to_owned()
            .try_into()
            .expect("portable fixture filename"),
        raw,
        "2026-10-03T00:00:00Z",
    )?;
    Ok(
        serde_json::json!({"snapshot":snapshot.manifest,"artifacts":work_feature::artifact_paths::default_artifact_paths(&id),"hierarchy_selection":hierarchy,"skill_selection":skills,"acceptance_criteria":acceptance}),
    )
}

/// Copy the immutable fixture Snapshot triplets together with isolated test setup.
pub fn copy_fixture_sources(
    fixture: &std::path::Path,
    root: &std::path::Path,
) -> Result<(), std::io::Error> {
    if !fixture.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("missing fixture project: {}", fixture.display()),
        ));
    }
    let sources = fixture.join("outputs/work/sources");
    if !sources.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("missing fixture Source directory: {}", sources.display()),
        ));
    }
    let mut files = Vec::new();
    for requirement in std::fs::read_dir(&sources)? {
        let requirement = requirement?;
        if !requirement.file_type()?.is_dir() {
            continue;
        }
        for snapshot in std::fs::read_dir(requirement.path())? {
            let snapshot = snapshot?;
            if !snapshot.file_type()?.is_dir() {
                continue;
            }
            let manifest_path = snapshot.path().join("manifest.json");
            let raw = std::fs::read(&manifest_path)?;
            let manifest: work_model::source::snapshot::SourceSnapshot =
                serde_json::from_slice(&raw).map_err(std::io::Error::other)?;
            for name in [
                "manifest.json",
                "manifest.json.done",
                manifest.content.path.as_str(),
            ] {
                let path = snapshot.path().join(name);
                let relative = path
                    .strip_prefix(fixture)
                    .expect("fixture descendant")
                    .to_path_buf();
                files.push((relative, std::fs::read(path)?));
            }
        }
    }
    for (relative, raw) in files {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("Snapshot parent"))?;
        std::fs::write(path, raw)?;
    }
    Ok(())
}

pub fn decode_transaction_snapshot(value: &Value) -> Result<Vec<u8>, String> {
    work_operations::derivation::snapshot::decode_snapshot(value)
        .map_err(|issue| issue.reason_code.to_owned())
}

/// Derive actual record-finish evidence for public serialization regression tests.
pub fn record_finish_candidates(
    task: &Value,
    attempt: &Value,
    index: &Value,
    request: &Value,
) -> Result<(Value, Value), String> {
    let candidate = work_operations::execution::record_finish::build_record_finish_candidates(
        task,
        attempt,
        index,
        task["id"].as_str().ok_or("missing_task_id")?,
        request,
    )
    .map_err(|issue| issue.reason_code.to_owned())?;
    Ok((candidate.attempt, candidate.index))
}

/// Restore the retained Task baseline only inside an isolated test skill root.
pub fn restore_historical_task_instructions(root: &std::path::Path) -> std::io::Result<()> {
    std::fs::write(
        root.join("references/instruction-loading/formal-task-validation.md"),
        include_bytes!(
            "../fixtures/historical/instructions/task/issue-81-baseline/formal-task-validation.md"
        ),
    )?;
    for (relative, raw) in [
        ("execute/reserve-one-execution-record.md", include_bytes!("../fixtures/historical/instructions/task/issue-81-baseline/workflows/execute/reserve-one-execution-record.md").as_slice()),
        ("execute/record-one-execution-result.md", include_bytes!("../fixtures/historical/instructions/task/issue-81-baseline/workflows/execute/record-one-execution-result.md").as_slice()),
        ("task/initialize-and-save.md", include_bytes!("../fixtures/historical/instructions/task/issue-81-baseline/workflows/task/initialize-and-save.md").as_slice()),
        ("task/formalization-boundary.md", include_bytes!("../fixtures/historical/instructions/task/issue-81-baseline/workflows/task/formalization-boundary.md").as_slice()),
        ("execute/close-one-attempt.md", include_bytes!("../fixtures/historical/instructions/task/issue-81-baseline/workflows/execute/close-one-attempt.md").as_slice()),
        ("execute/execute-one-authorized-argv-cmd.md", include_bytes!("../fixtures/historical/instructions/task/issue-81-baseline/workflows/execute/execute-one-authorized-argv-cmd.md").as_slice()),
        ("execute/recover-one-execution-transaction.md", include_bytes!("../fixtures/historical/instructions/task/issue-81-baseline/workflows/execute/recover-one-execution-transaction.md").as_slice()),
    ] { std::fs::write(root.join("references/workflows").join(relative), raw)?; }
    std::fs::write(
        root.join("references/instructions/task/general/references/task-records.md"),
        include_bytes!(
            "../fixtures/historical/instructions/task/issue-81-baseline/task-records.md"
        ),
    )?;
    std::fs::write(
        root.join("references/instructions/task/general/references/external-operations.md"),
        include_bytes!(
            "../fixtures/historical/instructions/task/issue-81-baseline/external-operations.md"
        ),
    )
}

/// Restore the retained Execute baseline only inside an isolated test skill root.
pub fn restore_historical_execute_instructions(root: &std::path::Path) -> std::io::Result<()> {
    for (relative, raw) in [
        (
            "instructions.md",
            include_bytes!("../fixtures/historical/instructions/execute/handoff-baseline/instructions.md")
                .as_slice(),
        ),
        (
            "references/execution-records.md",
            include_bytes!(
                "../fixtures/historical/instructions/execute/handoff-baseline/references/execution-records.md"
            )
            .as_slice(),
        ),
        (
            "references/execution-recovery.md",
            include_bytes!(
                "../fixtures/historical/instructions/execute/handoff-baseline/references/execution-recovery.md"
            )
            .as_slice(),
        ),
    ] {
        let path = root
            .join("references/instructions/execute/general")
            .join(relative);
        std::fs::create_dir_all(path.parent().expect("baseline parent"))?;
        std::fs::write(path, raw)?;
    }
    Ok(())
}

/// Reproduce a historical instruction environment without changing artifact evidence.
pub fn historical_task_skill_root(
    current: &std::path::Path,
) -> std::io::Result<std::path::PathBuf> {
    // Callers only read this retained environment; drift tests copy it first.
    static ROOTS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<std::path::PathBuf, std::path::PathBuf>>,
    > = std::sync::OnceLock::new();
    let mut roots = ROOTS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| std::io::Error::other("fixture catalog lock poisoned"))?;
    if let Some(root) = roots.get(current) {
        return Ok(root.clone());
    }
    let root = historical_task_skill_root_uncached(current)?;
    roots.insert(current.to_path_buf(), root.clone());
    Ok(root)
}

fn historical_task_skill_root_uncached(
    current: &std::path::Path,
) -> std::io::Result<std::path::PathBuf> {
    fn copy(source: &std::path::Path, target: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(target)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            let path = target.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy(&entry.path(), &path)?;
            } else {
                std::fs::copy(entry.path(), path)?;
            }
        }
        Ok(())
    }
    let root = std::env::temp_dir().join(format!(
        "work-historical-execute-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    copy(&current.join("references"), &root.join("references"))?;
    restore_historical_task_instructions(&root)?;
    Ok(root)
}

pub fn historical_execute_skill_root(
    current: &std::path::Path,
) -> std::io::Result<std::path::PathBuf> {
    let root = historical_task_skill_root_uncached(current)?;
    restore_historical_execute_instructions(&root)?;
    Ok(root)
}

pub use work_operations::derivation::publication::{
    JournalKind, journal_path as fixture_journal_path,
};
/// Prepare current transaction bytes for isolated recovery tests, with no writes.
pub use work_operations::derivation::transaction::{
    PublicationOrder, TransactionInput, TransactionKind,
};

/// Derive complete isolated recovery evidence without writing or granting publication authority.
pub fn derive_fixture_journal_staging(
    root: &std::path::Path,
    requirement: &str,
    execution: &str,
    relative: &str,
    journal: &Value,
) -> Result<Option<work_operations::derivation::transaction::PreparedJournalStaging>, String> {
    let mut original = journal.clone();
    original["state"] = serde_json::json!("prepared");
    original["published_count"] = serde_json::json!(0);
    let kind = work_operations::specification::transaction::verified_retained_journal_kind(
        execution, relative, &original,
    )
    .map_err(|issue| issue.reason_code.to_owned())?;
    let canonical = root.canonicalize().map_err(|_| "fixture_root".to_owned())?;
    let requirement = requirement
        .parse()
        .map_err(|_| "fixture_requirement".to_owned())?;
    let result = work_operations::derivation::transaction::build_journal_staging(
        work_operations::derivation::transaction::JournalStagingInput {
            canonical_root: canonical
                .to_str()
                .ok_or_else(|| "fixture_root".to_owned())?,
            requirement: &requirement,
            execution_dir: execution,
            journal_path: relative,
            kind,
            journal: &original,
        },
    )
    .map_err(|issue| issue.reason_code.to_owned())?;
    Ok(Some(result))
}

pub fn derive_fixture_transaction(input: TransactionInput) -> Result<(Vec<u8>, String), String> {
    let derived = work_operations::derivation::transaction::TransactionDeriver::derive(input)
        .map_err(|issue| issue.reason_code.to_owned())?;
    let raw = work_operations::specification::transaction::render_transaction(&derived.journal)
        .map_err(|issue| issue.reason_code.to_owned())?;
    Ok((raw, derived.approval_sha256))
}

/// Bind an isolated current Session fixture through the same instruction and fingerprint derivations as production.
/// Immutable Source bytes are read/copied separately; this helper never writes files.
pub fn prepare_discussion_fixture(
    fixture: &std::path::Path,
    project_root: &std::path::Path,
    skill_root: &std::path::Path,
) -> Result<work_model::discussion::DiscussionSession, String> {
    use work_model::common::Nullable;
    let mut session: work_model::discussion::DiscussionSession = serde_json::from_slice(
        &std::fs::read(fixture.join("session.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    session.authorization.project_root = project_root
        .canonicalize()
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .into_owned();
    let repository = crate::hierarchy_catalog::LocalHierarchyCatalog {
        skill_root: skill_root.to_path_buf(),
    };
    if let Nullable::Value(source) = &session.context.confirmed_source {
        let hierarchy =
            serde_json::to_value(&source.hierarchy_selection).expect("Hierarchy serializes");
        for task in &mut session.tasks {
            if let Nullable::Value(selection) = &task.instruction_selection {
                let current = work_feature::instruction::select_task(
                    &repository,
                    &hierarchy,
                    &selection.selected_paths,
                    &selection.references,
                )
                .map_err(|error| error.reason_code)?;
                task.instructions_sha256 = Nullable::Value(current.instructions_sha256);
            }
        }
    }
    session.commit.content_sha256 = discussion_sha256(&session);
    work_operations::discussion::verify_integrity(&session).map_err(|issue| issue.0.to_owned())?;
    Ok(session)
}

pub fn discussion_sha256(session: &work_model::discussion::DiscussionSession) -> String {
    work_operations::derivation::fingerprint::discussion_session(session)
}

/// Structural serialization/recovery fixture only; this does not authorize native acceptance.
/// Runtime integration tests must instead supply an actual fixture file via user_confirmation_fixture.
pub fn user_validation_record_fixture(
    attempt: &Value,
    validation: &Value,
    record_id: &str,
    statement: &str,
) -> Value {
    use work_model::execution::acceptance::*;
    let document = UserConfirmationDocument {
        source: UserConfirmationOrigin::UserDocument,
        source_reference: format!("synthetic-serialization-user/{record_id}"),
        confirmer: validation["confirmer"]
            .as_str()
            .expect("designated fixture user")
            .into(),
        task_id: attempt["task_id"].as_str().expect("fixture Task").into(),
        attempt_id: attempt["attempt_id"]
            .as_str()
            .expect("fixture Attempt")
            .into(),
        validation_id: validation["id"].as_str().expect("fixture VAL").into(),
        record_id: record_id.into(),
        file_version: "a".repeat(64),
        outcome: work_model::execution::attempt::ValidationOutcome::Passed,
        statement: statement.into(),
    };
    let raw = serde_json::to_vec(&document).expect("fixture statement serializes");
    serde_json::json!({"id":record_id,"kind":"validation","source":"user_confirmation",
        "file_version":document.file_version,"outcome":"passed","evidence":statement,
        "confirmation_evidence":{"reference":{"path":"fixtures/user-statement.json","sha256":raw_sha256(&raw)},
            "document":document,"raw":raw}})
}
pub fn user_validation_finish_fixture(
    attempt: &Value,
    validation: &Value,
    record_id: &str,
    statement: &str,
) -> Value {
    let mut record = user_validation_record_fixture(attempt, validation, record_id, statement);
    record.as_object_mut().unwrap().remove("id");
    record.as_object_mut().unwrap().remove("kind");
    serde_json::json!({"schema":"work-record-finish-request","record":record})
}

/// Synthetic user document for integration fixtures, using the actual reserved context.
/// Production callers must supply the designated user's actual statement instead.
pub fn user_confirmation_fixture(
    root: &std::path::Path,
    context: &Value,
    outcome: work_model::execution::attempt::ValidationOutcome,
    statement: &str,
) -> Result<Value, String> {
    use work_model::execution::acceptance::*;
    let context: ValidationContext =
        serde_json::from_value(context.clone()).map_err(|e| e.to_string())?;
    if context.source != ValidationEvidenceSource::UserConfirmation {
        return Err("Fixture requires a reserved user confirmation".into());
    }
    let document = UserConfirmationDocument {
        source: UserConfirmationOrigin::UserDocument,
        source_reference: format!("synthetic-fixture-user/{}", context.record_id),
        confirmer: context.confirmer.ok_or("Missing designated fixture user")?,
        task_id: context.task_id,
        attempt_id: context.attempt_id,
        validation_id: context.validation_id,
        record_id: context.record_id.clone(),
        file_version: context.file_version,
        outcome,
        statement: statement.into(),
    };
    let raw = serde_json::to_vec(&document).map_err(|e| e.to_string())?;
    let path = format!(
        "user-confirmation-fixtures/{}-{}.json",
        document.attempt_id,
        context.record_id.replace('#', "-")
    );
    std::fs::create_dir_all(root.join("user-confirmation-fixtures")).map_err(|e| e.to_string())?;
    std::fs::write(root.join(&path), &raw).map_err(|e| e.to_string())?;
    Ok(
        serde_json::json!({"schema":"work-record-finish-request","record":{"outcome":outcome,
        "confirmation":{"path":path,"sha256":work_operations::derivation::fingerprint::raw(&raw)}}}),
    )
}

/// Isolated current planning -> formalization -> Attempt fixture for project-file integration tests.
/// The caller provides a newly allocated temporary project; existing projects are never accepted.
pub fn file_transaction_fixture(
    root: &std::path::Path,
    skill_root: &std::path::Path,
) -> Result<work_feature::execution::ExecutionWriterContext, String> {
    file_transaction_fixture_inner(root, skill_root, false, None, false, None)
}

#[cfg(any(target_os = "macos", windows))]
pub fn file_transaction_command_fixture(
    root: &std::path::Path,
    skill_root: &std::path::Path,
) -> Result<work_feature::execution::ExecutionWriterContext, String> {
    file_transaction_fixture_inner(root, skill_root, true, None, false, None)
}

pub fn file_transaction_command_scope_fixture(
    root: &std::path::Path,
    skill_root: &std::path::Path,
) -> Result<work_feature::execution::ExecutionWriterContext, String> {
    file_transaction_fixture_inner(root, skill_root, true, Some(true), false, None)
}

pub fn file_transaction_scope_fixture(
    root: &std::path::Path,
    skill_root: &std::path::Path,
    rollback: bool,
) -> Result<work_feature::execution::ExecutionWriterContext, String> {
    file_transaction_fixture_inner(root, skill_root, false, Some(rollback), false, None)
}

/// Complete current three-method acceptance fixture with actual native command execution.
pub fn mixed_validation_fixture(
    root: &std::path::Path,
    skill_root: &std::path::Path,
) -> Result<work_feature::execution::ExecutionWriterContext, String> {
    file_transaction_fixture_inner(root, skill_root, true, Some(true), true, None)
}

struct DevelopmentFixture<'a> {
    files: &'a std::collections::BTreeMap<String, Vec<u8>>,
    verification_argv: &'a [String],
    verification_only: bool,
}

/// A formally declared read-only Task with actual command evidence and no file changes.
pub fn input_verification_fixture(
    root: &std::path::Path,
    skill_root: &std::path::Path,
    files: &std::collections::BTreeMap<String, Vec<u8>>,
    verification_argv: &[String],
) -> Result<work_feature::execution::ExecutionWriterContext, String> {
    file_transaction_fixture_inner(
        root,
        skill_root,
        true,
        Some(true),
        false,
        Some(DevelopmentFixture {
            files,
            verification_argv,
            verification_only: true,
        }),
    )
}

/// Generic isolated local-development fixture: original project files, dirty user
/// contents, one dependent and one independent Task, and the ordinary argv runner.
pub fn development_iteration_fixture(
    root: &std::path::Path,
    skill_root: &std::path::Path,
    files: &std::collections::BTreeMap<String, Vec<u8>>,
    verification_argv: &[String],
) -> Result<work_feature::execution::ExecutionWriterContext, String> {
    file_transaction_fixture_inner(
        root,
        skill_root,
        true,
        Some(true),
        true,
        Some(DevelopmentFixture {
            files,
            verification_argv,
            verification_only: false,
        }),
    )
}

fn file_transaction_fixture_inner(
    root: &std::path::Path,
    skill_root: &std::path::Path,
    native_commands: bool,
    rollback: Option<bool>,
    mixed: bool,
    development: Option<DevelopmentFixture<'_>>,
) -> Result<work_feature::execution::ExecutionWriterContext, String> {
    file_transaction_fixture_configured(
        root,
        skill_root,
        native_commands,
        rollback,
        mixed,
        development,
        None,
    )
}

/// Seed an already authorized supplemental path in an isolated synthetic Attempt.
/// This is test setup, not the production deviation/approval workflow.
pub fn authorize_supplemental_file_fixture(
    root: &std::path::Path,
    task: &str,
    path: &str,
) -> Result<(), String> {
    let attempt_path = root.join(format!(
        "outputs/work/executions/example/{task}/ATTEMPT-001/attempt.json"
    ));
    let raw = std::fs::read(&attempt_path).map_err(|e| e.to_string())?;
    let mut attempt =
        work_operations::canonical::parse_json_contract(&raw).map_err(|e| format!("{e:?}"))?;
    work_operations::execution::attempt::validate_attempt_bytes(&attempt, &raw)
        .map_err(|e| e.reason_code.to_owned())?;
    let scope = work_operations::execution::authorization::scope::from_attempt(&attempt)
        .map_err(|e| e.reason_code.to_owned())?
        .ok_or("Fixture scope missing")?;
    if !root
        .join("outputs/work/transactions/example/file-test")
        .is_dir()
        || !work_operations::execution::authorization::scope::contains_path(&scope, path)
    {
        return Err("Fixture supplemental path outside scope".into());
    }
    attempt["authorization"]["modifiable_files"]
        .as_array_mut()
        .ok_or("Fixture files missing")?
        .push(serde_json::json!(path));
    attempt["authorization_sha256"] = serde_json::json!(
        work_operations::derivation::fingerprint::structured(&attempt["authorization"])
            .map_err(|e| format!("{e:?}"))?
    );
    let raw = work_operations::execution::attempt::render_attempt(&attempt)
        .map_err(|e| e.reason_code.to_owned())?;
    work_operations::execution::attempt::validate_attempt_bytes(&attempt, &raw)
        .map_err(|e| e.reason_code.to_owned())?;
    std::fs::write(attempt_path, raw).map_err(|e| e.to_string())
}

/// Two formally declared disjoint Tasks that may supplement their scope at execution time.
pub fn supplemental_files_fixture(
    root: &std::path::Path,
    skill_root: &std::path::Path,
    ordered: bool,
) -> Result<work_feature::execution::ExecutionWriterContext, String> {
    file_transaction_fixture_configured(
        root,
        skill_root,
        false,
        Some(true),
        false,
        None,
        Some(FileFixtureScenario::Supplemental(ordered)),
    )
}

enum FileFixtureScenario {
    MovedInput,
    Supplemental(bool),
    Swap,
    Replacement,
    Intermediate,
}

/// A declared input is also the source of a complete published move.
pub fn moved_input_fixture(
    root: &std::path::Path,
    skill_root: &std::path::Path,
) -> Result<work_feature::execution::ExecutionWriterContext, String> {
    file_transaction_fixture_configured(
        root,
        skill_root,
        true,
        Some(true),
        false,
        None,
        Some(FileFixtureScenario::MovedInput),
    )
}

/// Publish a whole-set plan through the ordinary Discussion and Execute APIs.
pub fn whole_file_set_fixture(
    root: &std::path::Path,
    skill_root: &std::path::Path,
    kind: &str,
) -> Result<work_feature::execution::ExecutionWriterContext, String> {
    let scenario = match kind {
        "swap" => FileFixtureScenario::Swap,
        "replacement" => FileFixtureScenario::Replacement,
        "intermediate" => FileFixtureScenario::Intermediate,
        _ => return Err("Unknown whole-set fixture".into()),
    };
    file_transaction_fixture_configured(
        root,
        skill_root,
        false,
        Some(true),
        false,
        None,
        Some(scenario),
    )
}

fn file_transaction_fixture_configured(
    root: &std::path::Path,
    skill_root: &std::path::Path,
    native_commands: bool,
    rollback: Option<bool>,
    mixed: bool,
    development: Option<DevelopmentFixture<'_>>,
    scenario: Option<FileFixtureScenario>,
) -> Result<work_feature::execution::ExecutionWriterContext, String> {
    use crate::discussion::assembly::{LocalDiscussionAssembly, PublicationRequest};
    use serde_json::json;
    use work_feature::execution::*;
    use work_model::common::Nullable;
    let fixture = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/cases/discussion/assembly/valid/input"
    ));
    let write = |p: &str, b: &[u8]| std::fs::write(root.join(p), b).map_err(|e| e.to_string());
    if root.join("outputs").exists() || root.join(".git").exists() {
        return Err("Fixture requires a new empty temporary project".into());
    }
    copy_fixture_sources(&fixture.join("../project"), root).map_err(|e| e.to_string())?;
    write("a-old.txt", b"original move\r\n")?;
    write("c-existing.txt", b"original modify\0bytes\n")?;
    if let Some(development) = &development {
        for (path, raw) in development.files {
            if !work_operations::derivation::identity::runtime_relative_path(path)
                || path.starts_with("outputs/")
                || path == ".git"
                || path.starts_with(".git/")
                || root.join(path).exists()
            {
                return Err(
                    "Development fixture requires unique new project-relative source files".into(),
                );
            }
            std::fs::create_dir_all(root.join(path).parent().ok_or("Missing project parent")?)
                .map_err(|e| e.to_string())?;
            write(path, raw)?;
        }
    }
    let mut session = prepare_discussion_fixture(fixture, root, skill_root)?;
    if matches!(scenario, Some(FileFixtureScenario::MovedInput)) {
        session.tasks[0].inputs.push(serde_json::from_value(json!({"key":"moved-input","kind":"project_state","source":"a-old.txt","precondition":"Existing source to move and verify"})).map_err(|e|e.to_string())?);
    }
    let second = serde_json::to_string(&session.tasks[0])
        .map_err(|e| e.to_string())?
        .replace("TASK-001", "TASK-002");
    let mut second: work_model::discussion::DiscussionTask =
        serde_json::from_str(&second).map_err(|e| e.to_string())?;
    second.dependencies = vec!["TASK-001".into()];
    if let Some(FileFixtureScenario::Supplemental(ordered)) = scenario {
        if !ordered {
            second.dependencies.clear();
        }
        second.inputs.clear();
        second.commands.clear();
        second.operations.clear();
        second.files = serde_json::from_value(
            serde_json::from_str(include_str!(
                "../fixtures/shared/source-task/independent-file.json"
            ))
            .expect("fixed fixture JSON"),
        )
        .map_err(|e| e.to_string())?;
        second.steps = serde_json::from_value(json!([{"key":"independent","action":"Verify the independent output",
            "references":[{"kind":"files","key":"independent"},{"kind":"validations","key":second.validations[0].key}]}])).map_err(|e|e.to_string())?;
    }
    session.tasks.push(second);
    session.next_task_number = 3;
    if let Some(development) = &development {
        let raw = serde_json::to_string(&session.tasks[1])
            .map_err(|e| e.to_string())?
            .replace("TASK-002", "TASK-003");
        let mut independent: work_model::discussion::DiscussionTask =
            serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        independent.dependencies.clear();
        independent.inputs.clear();
        independent.commands.clear();
        independent.operations.clear();
        independent.files = serde_json::from_value(
            serde_json::from_str(include_str!(
                "../fixtures/shared/source-task/independent-file.json"
            ))
            .expect("fixed fixture JSON"),
        )
        .map_err(|e| e.to_string())?;
        independent.steps=serde_json::from_value(json!([{"key":"independent","action":"Verify the independent output",
            "references":[{"kind":"files","key":"independent"},{"kind":"validations","key":independent.validations[0].key}]}])).map_err(|e|e.to_string())?;
        session.tasks.push(independent);
        session.next_task_number = 4;
        for (position, path) in development.files.keys().enumerate() {
            session.tasks[0].inputs.push(serde_json::from_value(json!({"key":format!("development-{}",position+1),"kind":"project_state","source":path,"precondition":"Existing local project source used by the declared verification command"})).map_err(|e|e.to_string())?);
            if let Some((parent, _)) = path.rsplit_once('/') {
                if !session.tasks[0]
                    .inputs
                    .iter()
                    .any(|input| input.source.as_deref() == Some(parent))
                {
                    session.tasks[0].inputs.push(serde_json::from_value(json!({"key":format!("directory-{}",position+1),"kind":"project_state","source":parent,"precondition":"Directory dependency range; file contents have separate evidence"})).map_err(|e|e.to_string())?);
                }
            }
        }
    }
    session.tasks[0].files = serde_json::from_value(
        serde_json::from_str(include_str!(
            "../fixtures/shared/source-task/default-files.json"
        ))
        .expect("fixed fixture JSON"),
    )
    .map_err(|e| e.to_string())?;
    match scenario {
        Some(FileFixtureScenario::Swap) => {
            session.tasks[0].files = serde_json::from_value(
                serde_json::from_str(include_str!(
                    "../fixtures/shared/source-task/swap-files.json"
                ))
                .expect("fixed fixture JSON"),
            )
            .map_err(|e| e.to_string())?;
        }
        Some(FileFixtureScenario::Replacement) => {
            session.tasks[0].files[2] = serde_json::from_value(
                json!({"key":"created","action":"create","path":"a-old.txt"}),
            )
            .map_err(|e| e.to_string())?;
        }
        Some(FileFixtureScenario::Intermediate) => {
            session.tasks[0].files = serde_json::from_value(
                serde_json::from_str(include_str!(
                    "../fixtures/shared/source-task/intermediate-files.json"
                ))
                .expect("fixed fixture JSON"),
            )
            .map_err(|e| e.to_string())?;
        }
        _ => {}
    }
    if native_commands {
        #[cfg(target_os = "macos")]
        let (image, flags, generate, verify) = (
            "/bin/sh".to_owned(),
            vec!["-c"],
            "cat c-existing.txt; mkdir -p tool-output; printf 'final move\\n' > tool-output/b-moved.txt; printf 'final modify\\n' > tool-output/c-existing.txt; printf 'new complete output\\n' > tool-output/d-new.txt",
            "cat c-existing.txt; test ! -e a-old.txt && test -f b-moved.txt && test -f d-new.txt && grep -q 'final modify' c-existing.txt",
        );
        #[cfg(windows)]
        let (image, flags, generate, verify) = (
            std::env::var("COMSPEC").map_err(|e| e.to_string())?,
            vec!["/d", "/c"],
            "type c-existing.txt & if not exist tool-output mkdir tool-output & echo final move>tool-output\\b-moved.txt & echo final modify>tool-output\\c-existing.txt & echo new complete output>tool-output\\d-new.txt",
            "type c-existing.txt & (if exist a-old.txt (exit /b 9) else if not exist b-moved.txt (exit /b 8) else if not exist d-new.txt (exit /b 8) else (findstr /c:\"final modify\" c-existing.txt >nul))",
        );
        #[cfg(not(any(target_os = "macos", windows)))]
        return Err("Native command fixture supports Windows and macOS only".into());
        #[cfg(any(target_os = "macos", windows))]
        {
            let argv = |script: &str| {
                std::iter::once(image.clone())
                    .chain(flags.iter().map(|flag| (*flag).to_owned()))
                    .chain([script.to_owned()])
                    .collect::<Vec<_>>()
            };
            session.tasks[0].commands = serde_json::from_value(json!([
                {"key":"generate","mode":"argv","argv":argv(generate)},
                {"key":"verify","mode":"argv","argv":argv(verify)}
            ]))
            .map_err(|e| e.to_string())?;
            let acceptance_ids = session.tasks[0].validations[0].acceptance_ids.clone();
            let verification: Value = serde_json::from_str(include_str!(
                "../fixtures/shared/execution/command-verification.json"
            ))
            .expect("fixed fixture JSON");
            session.tasks[0].validations = serde_json::from_value(json!([
                {"key":"result","kind":"automated","command_keys":["verify"],
                    "pass_condition":"Actual verification reads the complete published output",
                    "verification":verification,
                    "acceptance_ids":acceptance_ids}
            ]))
            .map_err(|e| e.to_string())?;
            for key in ["generate", "verify"] {
                session.tasks[0].steps[0].references.push(
                    work_model::task::candidate::CandidateReference {
                        kind: work_model::task::candidate::CandidateReferenceKind::Commands,
                        key: key.into(),
                    },
                );
            }
        }
    }
    if let Some(development) = &development {
        if development.verification_argv.is_empty() {
            return Err("Missing development verification argv".into());
        }
        session.tasks[0].commands[1].argv = Some(development.verification_argv.to_vec());
    }
    if matches!(scenario, Some(FileFixtureScenario::MovedInput)) {
        #[cfg(windows)]
        let argv = vec![
            std::env::var("COMSPEC").map_err(|e| e.to_string())?,
            "/d".into(),
            "/c".into(),
            "type c-existing.txt & findstr /c:\"final move\" b-moved.txt".into(),
        ];
        #[cfg(target_os = "macos")]
        let argv = vec![
            "/bin/sh".into(),
            "-c".into(),
            "cat c-existing.txt; grep -q 'final move' b-moved.txt".into(),
        ];
        #[cfg(any(windows, target_os = "macos"))]
        {
            session.tasks[0].commands[1].argv = Some(argv);
        }
    }
    if mixed {
        // The procedure has no explicit DSL: actual zero exit is the standard check.
        let mut validation =
            serde_json::to_value(&session.tasks[0].validations[0]).map_err(|e| e.to_string())?;
        validation.as_object_mut().unwrap().remove("verification");
        let acceptance_ids = validation["acceptance_ids"].clone();
        session.tasks[0].validations = serde_json::from_value(json!([
            validation,
            {"key":"review", "kind":"ai_review", "confirmer":"fixture-reviewer",
             "criteria":"Review the complete file result with exact concrete references", "acceptance_ids":acceptance_ids},
            {"key":"user", "kind":"manual", "confirmer":"fixture-user",
             "criteria":"Designated user confirms this exact output version", "acceptance_ids":acceptance_ids}
        ])).map_err(|e|e.to_string())?;
        for key in ["review", "user"] {
            session.tasks[0].steps[0].references.push(
                work_model::task::candidate::CandidateReference {
                    kind: work_model::task::candidate::CandidateReferenceKind::Validations,
                    key: key.into(),
                },
            );
        }
    }
    for key in ["moved", "modified", "created"] {
        session.tasks[0].steps[0].references.push(
            work_model::task::candidate::CandidateReference {
                kind: work_model::task::candidate::CandidateReferenceKind::Files,
                key: key.into(),
            },
        );
    }
    if let Nullable::Value(review) = &mut session.tasks[0].review {
        review.semantic_consistency_evidence =
            "One verified output requires coordinated create, modify and move".into();
    }
    if development
        .as_ref()
        .is_some_and(|fixture| fixture.verification_only)
    {
        let task = &mut session.tasks[0];
        task.files.clear();
        task.operations.clear();
        task.commands.remove(0);
        task.steps = serde_json::from_value(json!([{"key":"verify-inputs","action":"Verify declared inputs without modifying project files",
            "references":[{"kind":"commands","key":"verify"},{"kind":"validations","key":"result"}]}])).map_err(|e|e.to_string())?;
        let mut validation =
            serde_json::to_value(&task.validations[0]).map_err(|e| e.to_string())?;
        validation.as_object_mut().unwrap().remove("verification");
        task.validations[0] = serde_json::from_value(validation).map_err(|e| e.to_string())?;
    }
    session.commit.content_sha256 = discussion_sha256(&session);
    crate::discussion::storage::LocalDiscussionStorage {
        project_root: root.into(),
        files: crate::files::LocalFiles,
    }
    .initialize_with_runtime(&session, false)
    .map_err(|e| e.reason_code)?;
    let assembly = LocalDiscussionAssembly {
        project_root: root.into(),
        skill_root: skill_root.into(),
        skill_configs: vec![],
    };
    let mut metadata: Value = serde_json::from_str(include_str!(
        "../fixtures/shared/execution/publication-metadata.json"
    ))
    .expect("fixed fixture JSON");
    if native_commands {
        metadata["execution_defaults"] = json!({"os":std::env::consts::OS,"working_directory":".",
            "shell":if cfg!(windows) {"cmd"} else {"sh"}});
    }
    let preview = assembly
        .preview("example", &metadata)
        .map_err(|e| e.reason_code)?;
    assembly
        .publish(PublicationRequest {
            requirement_id: "example",
            expected_revision: session.revision,
            session_sha256: &session.commit.content_sha256,
            metadata: &metadata,
            approved_sha256: preview["approval_sha256"].as_str().unwrap(),
            publication_evidence: "Approved synthetic complete plan",
            recovery: false,
        })
        .map_err(|e| e.reason_code)?;
    for args in [
        vec!["init"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Work Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-m",
            "Fixture baseline",
        ],
    ] {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into());
        }
    }
    if development.is_some() {
        // The fixture actor deliberately leaves its own uncommitted source edit.
        write("c-existing.txt", b"existing uncommitted user contents\n")?;
    }
    let hierarchy = crate::hierarchy_catalog::LocalHierarchyCatalog {
        skill_root: skill_root.into(),
    };
    let skills = crate::skill_catalog::LocalSkillCatalog { roots: vec![] };
    let paths = crate::artifact_paths::LocalArtifactPaths {
        project_root: root.into(),
    };
    let tasks = crate::task::storage::LocalTaskStorage {
        project_root: root.into(),
    };
    let storage = crate::execution::storage::LocalExecutionStorage {
        project_root: root.into(),
    };
    let sources = CommandProjectSources {
        instructions: &hierarchy,
        skills: &skills,
        paths: &paths,
        task_repository: &tasks,
        skill_roots: &[],
    };
    let target = ExecutionProjectTarget {
        task_path: "outputs/work/tasks/example/index.json",
        execution_dir: "outputs/work/executions/example",
        task_id: "TASK-001",
    };
    let load = || load_execution_writer_context(&sources, target);
    let runtime = crate::execution::storage::RuntimeExecutionSession {
        storage: &storage,
        load_context: &load,
    };
    let choice: Value = serde_json::from_str(include_str!(
        "../fixtures/shared/execution/attempt-choice.json"
    ))
    .expect("fixed fixture JSON");
    let choice = if native_commands {
        let mut choice = choice;
        choice["command_positions"] = json!([1, 2]);
        choice["authorization_evidence"] = json!(
            "Approve actual native generation, full file publication and actual verification"
        );
        choice
    } else {
        choice
    };
    let mut choice = choice;
    match scenario {
        Some(FileFixtureScenario::Swap) => {
            choice["modifiable_files"] = json!(["a-old.txt", "c-existing.txt", "d-new.txt"])
        }
        Some(FileFixtureScenario::Replacement) => {
            choice["modifiable_files"] = json!(["a-old.txt", "b-moved.txt", "c-existing.txt"])
        }
        _ => {}
    }
    if development
        .as_ref()
        .is_some_and(|fixture| fixture.verification_only)
    {
        choice["command_positions"] = json!([1]);
        choice["modifiable_files"] = json!([]);
    }
    if mixed {
        choice["validation_positions"] = json!([1, 2, 3]);
    }
    if let Some(rollback) = rollback {
        choice["scope"] = json!({"project_root":root.to_string_lossy(),"requirement_id":"example",
            "goal":"Deliver and verify the complete project file result","module_paths":["."],
            "modification_types":["create","modify","move"],"risk_boundary":"Local source writes and restoration of only known failed current writes",
            "validation_scope":["Configured project validations"],"task_ids":["TASK-001"],
            "adjustments":["additional_files","retry"],"revoked_reason":null,"rollback_on_write_failure":rollback});
        if development.as_ref().is_some_and(|d| d.verification_only) {
            choice["scope"]["module_paths"] = json!(["src"]);
        }
    }
    if development.is_some() {
        let initial = storage
            .read_index("outputs/work/executions/example/index.json")
            .map_err(|e| e.reason_code)?;
        let index = work_operations::canonical::parse_json_contract(&initial)
            .map_err(|e| format!("{e:?}"))?;
        let context = load().map_err(|e| e.reason_code)?;
        let collection = &context.task_context().collection;
        work_operations::execution::preflight::check_task_eligibility(
            collection,
            &index,
            "TASK-003",
            None,
            &["pending"],
        )
        .map_err(|e| e.reason_code.to_owned())?;
        let blocked = work_operations::execution::preflight::check_task_eligibility(
            collection,
            &index,
            "TASK-002",
            None,
            &["pending"],
        )
        .unwrap_err();
        if blocked.reason_code != "execute_preflight_dependency_incomplete" {
            return Err(blocked.reason_code.into());
        }
    }
    let prepared = prepare_attempt_start_from_project(&sources, &runtime, target, &[], &choice)
        .map_err(|e| format!("{}: {}", e.reason_code, e.message))?;
    start_attempt_from_project(
        &sources,
        &runtime,
        target,
        &[],
        &prepared["request"],
        &crate::clock_workspace::local_timestamp(),
    )
    .map_err(|e| e.reason_code)?;
    let staging = root.join("outputs/work/transactions/example/file-test/staging");
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    for (p, b) in [
        ("b-moved.txt", b"final move\n".as_slice()),
        ("c-existing.txt", b"final modify\0\n".as_slice()),
        ("d-new.txt", b"new complete output\n".as_slice()),
    ] {
        std::fs::write(staging.join(p), b).map_err(|e| e.to_string())?;
    }
    load().map_err(|e| e.reason_code)
}

/// Direct simultaneous changes from synthetic fixture-owned generated bytes.
pub fn file_changes_fixture(
    root: &std::path::Path,
) -> Result<Vec<work_model::execution::file_transaction::FileChange>, String> {
    use work_model::execution::file_transaction::FileChange;
    let bytes = |p: &str| {
        std::fs::read(root.join(format!(
            "outputs/work/transactions/example/file-test/staging/{p}"
        )))
        .map_err(|e| e.to_string())
    };
    Ok(vec![
        FileChange::Move {
            source: "a-old.txt".into(),
            destination: "b-moved.txt".into(),
            bytes: Some(bytes("b-moved.txt")?),
        },
        FileChange::Write {
            path: "c-existing.txt".into(),
            bytes: bytes("c-existing.txt")?,
        },
        FileChange::Write {
            path: "d-new.txt".into(),
            bytes: bytes("d-new.txt")?,
        },
    ])
}

/// Preserve an actual interrupted native transaction in a caller-owned synthetic CLI fixture.
pub fn interrupt_file_transaction_fixture(
    context: &work_feature::execution::ExecutionWriterContext,
    skill_root: &std::path::Path,
    position: usize,
) -> Result<String, String> {
    use work_feature::ports::with_runtime_writer;
    let root = &context.writer().canonical_project_root;
    if !root
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.starts_with("work-file-cli-"))
        || !root.starts_with(
            std::env::temp_dir()
                .canonicalize()
                .map_err(|e| e.to_string())?,
        )
    {
        return Err("Fault fixture must be an isolated Work CLI temporary project".into());
    }
    let changes = file_changes_fixture(root)?;
    let store = crate::execution::file_transaction::storage::LocalFileTransactions::for_project(
        root.clone(),
        skill_root.into(),
    );
    let preview = work_feature::execution::file_transaction::prepare(
        &store,
        context,
        "ATTEMPT-001",
        &changes,
        None,
    )
    .map_err(|e| e.reason_code)?;
    let result = with_runtime_writer(
        &crate::writer_lock::LocalWriterLock,
        context.writer(),
        work_model::runtime::LockClass::Execution,
        |_| store.fixture_publish(context, &preview, position),
    );
    match result {
        Err(e) if e.reason_code == "fixture_interruption" => {
            Ok(preview.manifest.transaction_identity)
        }
        Err(e) => Err(e.reason_code),
        Ok(_) => Err("Fault fixture did not interrupt publication".into()),
    }
}

#[cfg(test)]
mod file_dependency_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formal_records_require_order_and_obsolete_independence_cannot_waive_it() {
        let fixture = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/cases/discussion/assembly/valid/input/session.json"
        ));
        let mut session: work_model::discussion::DiscussionSession =
            serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();
        let second = serde_json::to_string(&session.tasks[0])
            .unwrap()
            .replace("TASK-001", "TASK-002");
        session.tasks.push(serde_json::from_str(&second).unwrap());
        session.next_task_number = 3;
        for task in &mut session.tasks {
            task.files = serde_json::from_value(
                json!([{"key":"shared","action":"modify","path":"shared.txt"}]),
            )
            .unwrap();
            task.steps[0]
                .references
                .push(serde_json::from_value(json!({"kind":"files","key":"shared"})).unwrap());
        }
        session.commit.content_sha256 = discussion_sha256(&session);
        let records = work_operations::discussion::assembly::task_records(&session).unwrap();
        let contract = json!({"tasks":records});
        assert_eq!(
            work_operations::task::file_dependencies::validate(&contract)
                .unwrap_err()
                .reason_code,
            "task_file_dependency_required"
        );
        let mut obsolete = serde_json::to_value(&session).unwrap();
        obsolete["tasks"][0]["review"]["file_independence"] = json!([{
            "task_ids":["TASK-001","TASK-002"],"path":"shared.txt",
            "actions":["modify","modify"],"planning_sha256":["a".repeat(64),"b".repeat(64)],
            "confirmed":true,"evidence":"Separate regions confirmed"
        }]);
        assert!(
            serde_json::from_value::<work_model::discussion::DiscussionSession>(obsolete).is_err()
        );
        session.tasks[1].dependencies = vec!["TASK-001".into()];
        session.commit.content_sha256 = discussion_sha256(&session);
        let ordered =
            json!({"tasks":work_operations::discussion::assembly::task_records(&session).unwrap()});
        work_operations::task::file_dependencies::validate(&ordered).unwrap();
    }
}
