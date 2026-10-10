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
    for (relative, raw) in [
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
    review_discussion_fixture(&mut session);
    session.commit.content_sha256 = discussion_sha256(&session);
    work_operations::discussion::verify_integrity(&session).map_err(|issue| issue.0.to_owned())?;
    Ok(session)
}

pub fn discussion_sha256(session: &work_model::discussion::DiscussionSession) -> String {
    work_operations::derivation::fingerprint::discussion_session(session)
}

/// Explicit semantic review for synthetic current test plans; never rewrites fixture history.
pub fn review_discussion_fixture(session: &mut work_model::discussion::DiscussionSession) {
    use work_model::discussion::{GranularityReview, SplitDecision};
    let bindings: Vec<_> = session
        .tasks
        .iter()
        .map(|t| work_operations::derivation::fingerprint::discussion_planning(session, &t.id))
        .collect();
    for (task, planning_sha256) in session.tasks.iter_mut().zip(bindings) {
        if let work_model::common::Nullable::Value(review) = &mut task.review {
            review.granularity = Some(GranularityReview {
                outcome: task.goal.clone(),
                split_decision: SplitDecision::SingleOutcome,
                indivisibility_reason: String::new(),
                transaction_feasible: true,
                evidence: "Synthetic plan reviewed for one verifiable outcome".into(),
                planning_sha256,
                semantic: Some(work_model::discussion::OutcomeConsistencyReview {
                    outcomes: vec![work_model::discussion::ReviewedOutcome {
                        id: "OUTCOME-001".into(),
                        statement: task.goal.clone(),
                        acceptance_ids: task
                            .acceptance_criteria
                            .iter()
                            .map(|a| a.id.clone())
                            .collect(),
                        file_keys: task.files.iter().map(|f| f.key.clone()).collect(),
                        scope: task.scope.clone(),
                        independently_acceptable: true,
                        needs_confirmation: false,
                        evidence: "Synthetic outcome maps the complete verified plan".into(),
                    }],
                    coupled_outcome_ids: vec!["OUTCOME-001".into()],
                    separation_consequence:
                        "Splitting the fixture leaves the declared result incomplete".into(),
                }),
            });
        }
    }
}

/// Isolated current planning -> formalization -> Attempt fixture for project-file integration tests.
/// The caller provides a newly allocated temporary project; existing projects are never accepted.
pub fn file_transaction_fixture(
    root: &std::path::Path,
    skill_root: &std::path::Path,
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
    let mut session = prepare_discussion_fixture(fixture, root, skill_root)?;
    let second = serde_json::to_string(&session.tasks[0])
        .map_err(|e| e.to_string())?
        .replace("TASK-001", "TASK-002");
    let mut second: work_model::discussion::DiscussionTask =
        serde_json::from_str(&second).map_err(|e| e.to_string())?;
    second.dependencies = vec!["TASK-001".into()];
    session.tasks.push(second);
    session.next_task_number = 3;
    session.tasks[0].files = serde_json::from_value(json!([
        {"key":"moved","action":"move","source":"a-old.txt","destination":"b-moved.txt"},
        {"key":"modified","action":"modify","path":"c-existing.txt"},
        {"key":"created","action":"create","path":"d-new.txt"}
    ]))
    .map_err(|e| e.to_string())?;
    for key in ["moved", "modified", "created"] {
        session.tasks[0].steps[0].references.push(
            work_model::task::candidate::CandidateReference {
                kind: work_model::task::candidate::CandidateReferenceKind::Files,
                key: key.into(),
            },
        );
    }
    review_discussion_fixture(&mut session);
    if let Nullable::Value(review) = &mut session.tasks[0].review {
        let granularity = review.granularity.as_mut().expect("fixture reviewed");
        granularity.split_decision = work_model::discussion::SplitDecision::Indivisible;
        granularity.indivisibility_reason =
            "One published output requires coordinated create, modify and move".into();
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
    let metadata = json!({"title":"File publication","summary":"Complete output"});
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
    let choice = json!({"command_positions":[],"validation_positions":[1],"modifiable_files":["a-old.txt","b-moved.txt","c-existing.txt","d-new.txt"],
        "external_operation_positions":[],"allowed_deviations":[],"authorization_evidence":"Fixture user authorized complete file publication and manual acceptance"});
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
    let staged = ["b-moved.txt", "c-existing.txt", "d-new.txt"]
        .into_iter()
        .map(|p| {
            (
                p.into(),
                format!("outputs/work/transactions/example/file-test/staging/{p}"),
            )
        })
        .collect();
    let store = crate::execution::file_transaction_storage::LocalFileTransactions::for_project(
        root.clone(),
        skill_root.into(),
    );
    let preview =
        work_feature::execution::file_transaction::prepare(&store, context, "ATTEMPT-001", &staged)
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
mod file_review_tests {
    use super::*;
    use serde_json::json;
    use work_model::common::Nullable;
    #[test]
    fn confirmed_independence_binds_both_plans_and_rejects_added_inputs() {
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
        let parent = serde_json::to_string(&session.tasks[0])
            .unwrap()
            .replace("TASK-001", "TASK-003");
        session.tasks.push(serde_json::from_str(&parent).unwrap());
        session.next_task_number = 4;
        for task in &mut session.tasks[..2] {
            task.dependencies = vec!["TASK-003".into()];
            task.files = serde_json::from_value(
                json!([{"key":"shared","action":"modify","path":"shared.txt"}]),
            )
            .unwrap();
            task.steps[0]
                .references
                .push(serde_json::from_value(json!({"kind":"files","key":"shared"})).unwrap());
        }
        review_discussion_fixture(&mut session);
        let hashes = ["TASK-001", "TASK-002"]
            .map(|id| work_operations::derivation::fingerprint::discussion_planning(&session, id));
        let Nullable::Value(review) = &mut session.tasks[0].review else {
            unreachable!()
        };
        review
            .file_independence
            .push(work_model::discussion::FileIndependenceReview {
                task_ids: ["TASK-001".into(), "TASK-002".into()],
                path: "shared.txt".into(),
                actions: ["modify".into(), "modify".into()],
                planning_sha256: hashes,
                confirmed: true,
                evidence: "User reviewed independently applicable edits".into(),
            });
        session.commit.content_sha256 = discussion_sha256(&session);
        let records = work_operations::discussion::assembly::task_records(&session).unwrap();
        let mut contract = json!({"tasks":records});
        assert!(
            work_operations::task::file_dependencies::validate(&contract, Some(&session)).is_ok()
        );
        assert_eq!(
            work_operations::task::file_dependencies::validate(&contract, None)
                .unwrap_err()
                .reason_code,
            "task_file_independence_confirmation_required"
        );
        contract["tasks"][0]["inputs"] = json!([{"id":"INPUT-001","kind":"project_state","source":"another.txt","precondition":"Ready"}]);
        assert_eq!(
            work_operations::task::file_dependencies::validate(&contract, Some(&session))
                .unwrap_err()
                .reason_code,
            "task_file_review_binding_mismatch"
        );
        let mut upstream_drift = json!({"tasks":records});
        upstream_drift["tasks"][2]["goal"] = json!("Changed formal upstream outcome");
        assert_eq!(
            work_operations::task::file_dependencies::validate(&upstream_drift, Some(&session))
                .unwrap_err()
                .reason_code,
            "task_file_review_binding_mismatch"
        );
        let Nullable::Value(source) = &session.context.confirmed_source else {
            unreachable!()
        };
        let mut changed_source = json!({"tasks":records,"source":{"kind":"snapshot","manifest":source.snapshot},"hierarchy_selection":source.hierarchy_selection,"skill_selection":source.skill_selection,"acceptance_criteria":source.acceptance_criteria});
        assert!(
            work_operations::task::file_dependencies::validate(&changed_source, Some(&session))
                .is_ok()
        );
        changed_source["source"]["manifest"]["source_id"] = json!("SRC-999");
        assert_eq!(
            work_operations::task::file_dependencies::validate(&changed_source, Some(&session))
                .unwrap_err()
                .reason_code,
            "task_file_review_binding_mismatch"
        );
        let contract = json!({"tasks":records});
        session.tasks[1].goal.push_str(" changed");
        session.commit.content_sha256 = discussion_sha256(&session);
        assert_eq!(
            work_operations::task::file_dependencies::validate(&contract, Some(&session))
                .unwrap_err()
                .reason_code,
            "task_granularity_review_stale"
        );
    }
}
