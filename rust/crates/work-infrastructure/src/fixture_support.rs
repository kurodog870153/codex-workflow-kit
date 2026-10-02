//! Artifact builders used by process boundary integration tests.

use serde_json::Value;

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
        "work-execution-deviation-proposal/v1" => {
            Some(deviation::validate_deviation_proposal(value))
        }
        "work-execution-deviation/v1" => Some(deviation::validate_deviation_artifact(value)),
        "work-execution-deviation-preview/v1" => Some(deviation::validate_deviation_preview(value)),
        "work-execution-deviation-record/v1" => {
            Some(deviation::validate_deviation_record_response(value))
        }
        "work-execution-deviation-semantic-request/v1" => {
            Some(deviation::validate_semantic_deviation_request(value))
        }
        _ => None,
    };
    if let Some(result) = execution {
        return result.map_err(|issue| issue.reason_code.to_owned());
    }
    match id {
        "work-spec-prepare-request/v1" => {
            work_operations::specification::prepare::validate_prepare_request(value)
                .map_err(|issue| issue.reason_code.to_owned())
        }
        "work-spec-verification-request/v1" => {
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
        &work_model::source_snapshot::SnapshotSource::UserText {},
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
            let manifest: work_model::source_snapshot::SourceSnapshot =
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
