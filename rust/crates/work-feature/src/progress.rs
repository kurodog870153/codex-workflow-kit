//! Discussion progress preview, preparation, read and save use cases.

use serde_json::{Map, Value, json};
use work_operations::canonical::{parse_json_contract, sha256_hex};
use work_operations::identifiers::RequirementId;
use work_operations::progress::{
    ProgressDocument, ProgressIssue, approval_sha256, render_progress, validate_progress,
};
use work_operations::protocol::{INVALID_SHA256_ERROR_CODE, valid_sha256};

use crate::error::{ExitCode, WorkError};

const SEMANTIC_FIELDS: [&str; 10] = [
    "title",
    "request",
    "current_task_id",
    "context",
    "source_status",
    "notes",
    "confirmed_decisions",
    "tentative",
    "open_questions",
    "next_discussion_point",
];

pub trait ProgressRepository {
    fn read_current(&self, relative: &str) -> Result<Option<Vec<u8>>, WorkError>;
    fn read_history(&self, relative: &str) -> Result<Vec<u8>, WorkError>;
    fn history_exists(&self, relative: &str) -> Result<bool, WorkError>;
    fn publish(
        &self,
        directory: &str,
        history: &str,
        current: &str,
        raw: &[u8],
        previous_sha256: Option<&str>,
    ) -> Result<(), WorkError>;
}

fn issue(error: ProgressIssue) -> WorkError {
    WorkError::new(
        ExitCode::Contract,
        error.reason_code,
        error.message,
        error.details,
    )
}

fn failure(exit: ExitCode, reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(exit, reason, message, details)
}

fn location(requirement_id: &str, mode: &str) -> Result<(String, String), WorkError> {
    requirement_id.parse::<RequirementId>().map_err(|invalid| {
        failure(
            ExitCode::Contract,
            invalid.reason_code(),
            "The requirement ID is invalid.",
            json!({}),
        )
    })?;
    if !matches!(mode, "plan" | "task") {
        return Err(failure(
            ExitCode::Contract,
            "invalid_progress_mode",
            "Only Plan and Task discussions can be saved.",
            json!({}),
        ));
    }
    let directory = format!("outputs/work/progress/{requirement_id}/{mode}");
    Ok((directory.clone(), format!("{directory}/progress.json")))
}

pub fn read_progress(
    repo: &impl ProgressRepository,
    requirement_id: &str,
    mode: &str,
) -> Result<Value, WorkError> {
    let (directory, relative) = location(requirement_id, mode)?;
    let raw = repo.read_current(&relative)?.ok_or_else(|| {
        failure(
            ExitCode::WorkflowState,
            "progress_not_saved",
            "No committed progress exists for this requirement and mode.",
            json!({}),
        )
    })?;
    parse_json_contract(&raw).map_err(|_| {
        failure(
            ExitCode::InputFormat,
            "invalid_json",
            "Stored progress is not valid JSON.",
            json!({"source":relative}),
        )
    })?;
    let document = ProgressDocument::parse(&raw).map_err(issue)?;
    let value = document.value;
    if document.canonical_raw != raw {
        return Err(failure(
            ExitCode::ArtifactIntegrity,
            "progress_not_canonical",
            "Stored progress is not canonical JSON.",
            json!({}),
        ));
    }
    if value["requirement_id"] != requirement_id || value["mode"] != mode {
        return Err(failure(
            ExitCode::ArtifactIntegrity,
            "progress_identity_mismatch",
            "Stored progress identifies another requirement or mode.",
            json!({}),
        ));
    }
    let revision = value["revision"].as_u64().expect("validated revision");
    let history = format!("{directory}/history/{revision}/progress.json");
    if repo.read_history(&history)? != raw {
        return Err(failure(
            ExitCode::ArtifactIntegrity,
            "progress_history_mismatch",
            "Current progress differs from its saved history.",
            json!({}),
        ));
    }
    Ok(work_model::progress::verified::<
        work_model::progress::ProgressRead,
    >(
        json!({"schema":"work-progress-read/v1","status":"saved","path":relative,"sha256":sha256_hex(&raw),"progress":value}),
    ))
}

pub fn preview_progress(
    repo: &impl ProgressRepository,
    value: &Value,
    expected_revision: u64,
) -> Result<Value, WorkError> {
    validate_progress(value).map_err(issue)?;
    let requirement_id = value["requirement_id"].as_str().expect("validated ID");
    let mode = value["mode"].as_str().expect("validated mode");
    let (directory, relative) = location(requirement_id, mode)?;
    let previous = if repo.read_current(&relative)?.is_some() {
        Some(read_progress(repo, requirement_id, mode)?)
    } else {
        None
    };
    let current_revision = previous
        .as_ref()
        .and_then(|previous| previous["progress"]["revision"].as_u64())
        .unwrap_or(0);
    if current_revision != expected_revision
        || value["revision"].as_u64() != expected_revision.checked_add(1)
    {
        return Err(failure(
            ExitCode::WorkflowState,
            "progress_revision_conflict",
            "Read and review current progress before saving the next revision.",
            json!({}),
        ));
    }
    let history = format!(
        "{directory}/history/{}",
        value["revision"].as_u64().expect("validated revision")
    );
    if repo.history_exists(&history)? {
        return Err(failure(
            ExitCode::WorkflowState,
            "progress_save_pending",
            "An existing uncommitted revision requires review; do not retry or overwrite it.",
            json!({"path":history,"committed_revision":current_revision}),
        ));
    }
    let approved = approval_sha256(
        &relative,
        expected_revision,
        previous.as_ref().and_then(|value| value["sha256"].as_str()),
        value,
    )
    .map_err(issue)?;
    Ok(work_model::progress::verified::<
        work_model::progress::ProgressReview,
    >(
        json!({"schema":"work-progress-preview/v1","status":"valid","path":relative,"expected_revision":expected_revision,"approved_sha256":approved,"progress":value,"source_validation":"not_checked","evidence_trust":"historical_context_only","formal_readiness":"not_established"}),
    ))
}

pub fn preview_progress_raw(
    repo: &impl ProgressRepository,
    raw: &[u8],
    expected_revision: u64,
) -> Result<Value, WorkError> {
    let document = ProgressDocument::parse(raw).map_err(issue)?;
    let mut preview = preview_progress(repo, &document.value, expected_revision)?;
    let previous_sha = if expected_revision == 0 {
        None
    } else {
        Some(
            read_progress(
                repo,
                document.value["requirement_id"].as_str().unwrap(),
                document.value["mode"].as_str().unwrap(),
            )?["sha256"]
                .as_str()
                .unwrap()
                .to_owned(),
        )
    };
    preview["approved_sha256"] = json!(
        document
            .approval_sha256(
                preview["path"].as_str().unwrap(),
                expected_revision,
                previous_sha.as_deref()
            )
            .map_err(issue)?
    );
    Ok(preview)
}

pub fn prepare_progress(
    repo: &impl ProgressRepository,
    semantic: &Value,
    requirement_id: &str,
    mode: &str,
    expected_revision: u64,
) -> Result<Value, WorkError> {
    location(requirement_id, mode)?;
    let supplied = semantic.as_object().ok_or_else(|| {
        failure(
            ExitCode::Contract,
            "expected_object",
            "A JSON object is required.",
            json!({"location":"progress_prepare"}),
        )
    })?;
    let initial = expected_revision == 0;
    if supplied
        .keys()
        .any(|field| !SEMANTIC_FIELDS.contains(&field.as_str()))
        || (initial
            && (supplied.len() != SEMANTIC_FIELDS.len()
                || SEMANTIC_FIELDS
                    .iter()
                    .any(|field| !supplied.contains_key(*field))))
    {
        return Err(failure(
            ExitCode::Contract,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location":"progress_prepare"}),
        ));
    }
    if !initial && supplied.is_empty() {
        return Err(failure(
            ExitCode::Contract,
            "progress_prepare_empty_change",
            "Supply at least one changed discussion field.",
            json!({}),
        ));
    }
    let mut previous_sha = None;
    let mut content = if initial {
        Map::new()
    } else {
        let previous = read_progress(repo, requirement_id, mode)?;
        if previous["progress"]["revision"].as_u64() != Some(expected_revision) {
            return Err(failure(
                ExitCode::WorkflowState,
                "progress_revision_conflict",
                "Read and review current progress before saving the next revision.",
                json!({}),
            ));
        }
        previous_sha = previous["sha256"].as_str().map(str::to_owned);
        previous["progress"]
            .as_object()
            .expect("validated progress")
            .clone()
    };
    for (field, value) in supplied {
        content.insert(field.clone(), value.clone());
    }
    content.insert("schema".into(), json!("work-discussion-progress/v1"));
    content.insert("requirement_id".into(), json!(requirement_id));
    content.insert("mode".into(), json!(mode));
    content.insert("revision".into(), json!(expected_revision + 1));
    content.insert("status".into(), json!("discussion_only"));
    let mut prepared = preview_progress(repo, &Value::Object(content), expected_revision)?;
    if let Some(expected) = previous_sha {
        if read_progress(repo, requirement_id, mode)?["sha256"] != expected {
            return Err(failure(
                ExitCode::WorkflowState,
                "progress_revision_conflict",
                "The discussion changed during preparation.",
                json!({}),
            ));
        }
    }
    prepared["schema"] = json!("work-progress-prepare/v1");
    let _: work_model::progress::ProgressReview =
        serde_json::from_value(prepared.clone()).expect("prepared progress matches its model");
    Ok(prepared)
}

#[derive(Debug, Clone)]
pub struct PreparedProgress {
    pub response: Value,
    pub candidate_raw: Vec<u8>,
}

pub fn prepare_progress_raw(
    repo: &impl ProgressRepository,
    semantic_raw: &[u8],
    requirement_id: &str,
    mode: &str,
    expected_revision: u64,
) -> Result<Value, WorkError> {
    Ok(
        prepare_progress_document(repo, semantic_raw, requirement_id, mode, expected_revision)?
            .response,
    )
}

pub fn prepare_progress_document(
    repo: &impl ProgressRepository,
    semantic_raw: &[u8],
    requirement_id: &str,
    mode: &str,
    expected_revision: u64,
) -> Result<PreparedProgress, WorkError> {
    let semantic = parse_json_contract(semantic_raw).map_err(|_| {
        failure(
            ExitCode::InputFormat,
            "invalid_json",
            "The progress request is not valid JSON.",
            json!({}),
        )
    })?;
    let mut prepared = prepare_progress(repo, &semantic, requirement_id, mode, expected_revision)?;
    let source = if semantic.get("context").is_some() {
        semantic_raw.to_vec()
    } else {
        let (_, relative) = location(requirement_id, mode)?;
        repo.read_current(&relative)?.ok_or_else(|| {
            failure(
                ExitCode::WorkflowState,
                "progress_not_saved",
                "No committed progress exists for this requirement and mode.",
                json!({}),
            )
        })?
    };
    let document =
        ProgressDocument::from_value_with_context_raw(prepared["progress"].clone(), &source)
            .map_err(issue)?;
    let previous_sha = if expected_revision == 0 {
        None
    } else {
        Some(
            read_progress(repo, requirement_id, mode)?["sha256"]
                .as_str()
                .unwrap()
                .to_owned(),
        )
    };
    prepared["approved_sha256"] = json!(
        document
            .approval_sha256(
                prepared["path"].as_str().unwrap(),
                expected_revision,
                previous_sha.as_deref()
            )
            .map_err(issue)?
    );
    Ok(PreparedProgress {
        response: prepared,
        candidate_raw: document.canonical_raw,
    })
}

pub fn save_progress(
    repo: &impl ProgressRepository,
    value: &Value,
    expected_revision: u64,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    let raw = render_progress(value).map_err(issue)?;
    save_progress_raw(repo, &raw, expected_revision, approved_sha256)
}

pub fn save_progress_raw(
    repo: &impl ProgressRepository,
    input_raw: &[u8],
    expected_revision: u64,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    if !valid_sha256(approved_sha256) {
        return Err(failure(
            ExitCode::Contract,
            INVALID_SHA256_ERROR_CODE,
            "A lowercase SHA-256 fingerprint is required.",
            json!({"location":"approved_sha256"}),
        ));
    }
    let document = ProgressDocument::parse(input_raw).map_err(issue)?;
    let value = &document.value;
    let checked = preview_progress_raw(repo, input_raw, expected_revision)?;
    if checked["approved_sha256"] != approved_sha256 {
        return Err(failure(
            ExitCode::ArtifactIntegrity,
            "progress_approval_changed",
            "The progress content or saved baseline changed after review.",
            json!({}),
        ));
    }
    let relative = checked["path"].as_str().expect("preview path");
    let (directory, _) = location(
        value["requirement_id"].as_str().expect("validated ID"),
        value["mode"].as_str().expect("validated mode"),
    )?;
    let history = format!(
        "{directory}/history/{}",
        value["revision"].as_u64().expect("validated revision")
    );
    let previous_sha = if expected_revision == 0 {
        None
    } else {
        Some(
            read_progress(
                repo,
                value["requirement_id"].as_str().unwrap(),
                value["mode"].as_str().unwrap(),
            )?["sha256"]
                .as_str()
                .unwrap()
                .to_owned(),
        )
    };
    if document
        .approval_sha256(relative, expected_revision, previous_sha.as_deref())
        .map_err(issue)?
        != approved_sha256
    {
        return Err(failure(
            ExitCode::ArtifactIntegrity,
            "progress_approval_changed",
            "The progress content or saved baseline changed after review.",
            json!({}),
        ));
    }
    let raw = document.canonical_raw;
    repo.publish(
        &directory,
        &history,
        relative,
        &raw,
        previous_sha.as_deref(),
    )?;
    let mut saved = read_progress(
        repo,
        value["requirement_id"].as_str().unwrap(),
        value["mode"].as_str().unwrap(),
    )?;
    if saved["sha256"] != sha256_hex(&raw) {
        return Err(failure(
            ExitCode::ArtifactIntegrity,
            "progress_write_mismatch",
            "The committed progress changed during verification.",
            json!({}),
        ));
    }
    saved["schema"] = json!("work-progress-save/v1");
    saved["approved_sha256"] = json!(approved_sha256);
    let _: work_model::progress::ProgressSave =
        serde_json::from_value(saved.clone()).expect("saved progress matches its model");
    Ok(saved)
}
