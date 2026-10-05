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
    let reviews: serde_json::Map<String,Value> = previous["tasks"].as_array().unwrap().iter().map(|task|(task["id"].as_str().unwrap().to_owned(),serde_json::json!({"outcome_decisions":"Reviewed the complete Source outcome.","technical_decisions":"Reviewed technical decisions against the complete Source.","boundary":"Reviewed the Task boundary and dependencies.","acceptance":"Reviewed retained and revised acceptance criteria.","skills":"Confirmed the selected Task Skills.","hierarchy":"Confirmed the selected Task hierarchy.","instructions":"Confirmed current instructions and references."}))).collect();
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
    let sources = fixture.join("outputs/work/sources");
    if !sources.exists() {
        return Ok(());
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

/// Restore the retained Execute baseline only inside an isolated test skill root.
pub fn restore_historical_execute_instructions(root: &std::path::Path) -> std::io::Result<()> {
    for (relative, raw) in [
        (
            "instructions.md",
            include_bytes!("../fixtures/handoff-closed/instruction-baseline/instructions.md")
                .as_slice(),
        ),
        (
            "references/execution-records.md",
            include_bytes!(
                "../fixtures/handoff-closed/instruction-baseline/references/execution-records.md"
            )
            .as_slice(),
        ),
        (
            "references/execution-recovery.md",
            include_bytes!(
                "../fixtures/handoff-closed/instruction-baseline/references/execution-recovery.md"
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
pub fn historical_execute_skill_root(
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
    restore_historical_execute_instructions(&root)?;
    Ok(root)
}

/// Prepare current transaction bytes for isolated recovery tests, with no writes.
pub use work_operations::derivation::transaction::{
    PublicationOrder, TransactionInput, TransactionKind,
};

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
