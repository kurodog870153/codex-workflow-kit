//! Public Specification request, transaction, and result formats.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{Nullable, deserialize_optional_nullable, deserialize_required_nullable};
use crate::schema::PublicSchema;
use crate::task::{index::TaskIndex, item::TaskItem};

pub fn verified<T: serde::de::DeserializeOwned>(value: Value) -> Value {
    let _: T =
        serde_json::from_value(value.clone()).expect("Specification value matches its model");
    value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPaths {
    pub source: String,
    pub task: String,
    pub execution: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceFingerprint {
    pub path: String,
    pub raw_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationCandidateKind {
    TaskIndex,
    TaskItem,
    ExecutionIndex,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateDocument {
    pub path: String,
    pub kind: MigrationCandidateKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub content: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticDecision {
    pub id: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub resolution: Option<Nullable<Value>>,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditTarget {
    pub artifact: SpecificationEditArtifact,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecificationEditArtifact {
    TaskIndex,
    TaskItem,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecificationEdit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EditTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_after: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_position: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecPrepareRequest {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub reason: String,
    pub edits: Vec<SpecificationEdit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_update: Option<SpecificationSourceUpdate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecificationSourceUpdate {
    pub source: crate::task::draft::PlanningSource,
    pub selections: BTreeMap<String, crate::task::draft::DraftInstructionSelection>,
    pub source_confirmation: crate::task::request::SourceReplacementConfirmation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecMigrationPrepareRequest {
    pub schema: PublicSchema,
    pub mode: String,
    pub requirement_id: String,
    pub sources: Vec<SourceFingerprint>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub reason: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub edits: Option<Nullable<Vec<SpecificationEdit>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_context: Option<MigrationTaskContext>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub task_title: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub task_summary: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub execution_defaults: Option<Nullable<BTreeMap<String, String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub tasks: Option<Nullable<Vec<MigrationSemanticTask>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_decisions: Option<Vec<SemanticDecision>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationTaskContext {
    pub artifacts: crate::task::source::TaskArtifactPaths,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::task::source::TaskProvenance>,
    pub hierarchy_selection: crate::hierarchy::HierarchySelection,
    pub skill_selection: crate::skill::SkillSelection,
    pub acceptance_criteria: Vec<crate::task::source::TaskAcceptance>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationSemanticTask {
    pub title: String,
    pub goal: String,
    pub skill_id: Option<String>,
    pub selected_paths: Vec<String>,
    pub references: Vec<String>,
    pub dependency_positions: Vec<u64>,
    pub candidate: crate::task::draft::SemanticTaskCandidate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecMigrationPreviewRequest {
    pub schema: PublicSchema,
    pub sources: Vec<SourceFingerprint>,
    pub candidates: Vec<CandidateDocument>,
    pub semantic_decisions: Vec<SemanticDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentDiff {
    pub path: String,
    pub operation: String,
    pub unified_diff: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecMigrationPreview {
    pub schema: PublicSchema,
    pub status: String,
    pub documents: Vec<String>,
    pub diffs: Vec<DocumentDiff>,
    pub validator_results: Vec<CheckResult>,
    pub relationship_results: Vec<CheckResult>,
    pub unresolved_items: Vec<String>,
    pub fingerprint: String,
    pub writable_ready: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecMigrationPublication {
    pub schema: PublicSchema,
    pub status: String,
    pub fingerprint: String,
    pub transaction_approval_sha256: String,
    pub journal: String,
    pub completion_marker: String,
    pub documents: Vec<String>,
    pub publication_status: String,
    pub validator_results: Vec<CheckResult>,
    pub relationship_results: Vec<CheckResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMigrationItem {
    pub id: String,
    pub path: String,
    pub kind: String,
    pub target_schema: String,
    pub required: bool,
    pub source_sha256: String,
    pub source_size: u64,
    pub raw: Vec<u8>,
    pub issue: String,
    pub resolution_status: ArtifactMigrationItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposed_content: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactMigrationItemStatus {
    Proposed,
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMigrationAnalysis {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub items: Vec<ArtifactMigrationItem>,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactMigrationAction {
    Apply,
    Modify,
    Skip,
    Abort,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMigrationDecision {
    pub item: ArtifactMigrationItem,
    pub action: ArtifactMigrationAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Explicit AI-reviewed replacement content; raw evidence is supplied unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMigrationChoice {
    pub id: String,
    pub action: ArtifactMigrationAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMigrationDecisionInput {
    pub schema: String,
    pub analysis: ArtifactMigrationAnalysis,
    pub choices: Vec<ArtifactMigrationChoice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMigrationRequest {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub analysis_fingerprint: String,
    pub decisions: Vec<ArtifactMigrationDecision>,
}

impl ArtifactMigrationRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        use std::collections::BTreeSet;

        if self.schema != PublicSchema::WorkArtifactMigrationRequest
            || self.requirement_id.is_empty()
            || !valid_migration_hash(&self.analysis_fingerprint)
            || self.decisions.is_empty()
        {
            return Err("invalid_migration_request");
        }
        let mut ids = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for decision in &self.decisions {
            let item = &decision.item;
            if item.id.is_empty()
                || item.path.is_empty()
                || item.issue.is_empty()
                || item.target_schema
                    != match item.kind.as_str() {
                        "source" => "work-source-snapshot",
                        "task_index" => "work-task-index",
                        "task_item" => "work-task-item",
                        "execution_index" => "work-execution-index",
                        _ => return Err("invalid_migration_item"),
                    }
                || (item.resolution_status == ArtifactMigrationItemStatus::Proposed)
                    != item.proposed_content.is_some()
                || !matches!(
                    item.kind.as_str(),
                    "source" | "task_index" | "task_item" | "execution_index"
                )
                || item.source_size != item.raw.len() as u64
                || !valid_migration_hash(&item.source_sha256)
                || !ids.insert(&item.id)
                || !paths.insert(&item.path)
            {
                return Err("invalid_migration_item");
            }
            match decision.action {
                ArtifactMigrationAction::Apply
                    if item.proposed_content.is_some() && decision.content.is_none() => {}
                ArtifactMigrationAction::Modify if decision.content.is_some() => {}
                ArtifactMigrationAction::Skip
                    if decision.content.is_none()
                        && decision
                            .reason
                            .as_ref()
                            .is_some_and(|reason| !reason.is_empty()) => {}
                _ => return Err("invalid_migration_decision"),
            }
        }
        Ok(())
    }

    pub fn executable(&self) -> bool {
        self.validate().is_ok()
            && self.decisions.iter().all(|decision| {
                !(decision.item.required && decision.action == ArtifactMigrationAction::Skip)
            })
    }
}

fn valid_migration_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotBytes {
    pub raw_sha256: String,
    pub base64: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionOperation {
    Add,
    Replace,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionFile {
    pub phase: u64,
    pub path: String,
    pub operation: TransactionOperation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<SnapshotBytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<SnapshotBytes>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionMetadata {
    pub request: Value,
    pub artifacts: BTreeMap<String, Value>,
    pub affected_task_ids: Vec<String>,
    pub history_sha256: BTreeMap<String, String>,
    pub source_sha256: BTreeMap<String, String>,
    pub candidate_sha256: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    Prepared,
    Publishing,
    Published,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecTransaction {
    pub schema: PublicSchema,
    pub transaction_id: String,
    pub approval_sha256: String,
    pub state: TransactionState,
    pub published_count: u64,
    pub metadata: TransactionMetadata,
    pub files: Vec<TransactionFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecUpdateExpected {
    pub source_sha256: String,
    pub task_index_sha256: String,
    pub execution_index_sha256: String,
    pub task_item_sha256: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecCandidate {
    pub task_index: TaskIndex,
    pub task_items: BTreeMap<String, TaskItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecUpdateRequest {
    pub schema: PublicSchema,
    pub reason: String,
    pub expected: SpecUpdateExpected,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_confirmation: Option<crate::task::request::SourceReplacementConfirmation>,
    pub task_index: TaskIndex,
    pub task_items: BTreeMap<String, TaskItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecUpdate {
    pub schema: PublicSchema,
    pub status: String,
    pub requirement_id: String,
    pub record_id: String,
    pub approved_sha256: String,
    pub affected_task_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_impact: Option<Vec<Value>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub changed_fields: Option<Nullable<Vec<String>>>,
    pub artifacts: ArtifactPaths,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub candidate: Option<Nullable<SpecCandidate>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub transaction: Option<Nullable<SpecTransaction>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub file_readiness: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub next_step: Option<Nullable<Value>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub publication_status: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub verification_request: Option<Nullable<SpecVerificationRequest>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareTransport {
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub output_file: Nullable<String>,
    pub request_field: String,
    pub request_schema: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NextStep {
    pub command: String,
    pub input: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecPrepare {
    pub schema: PublicSchema,
    pub request: SpecUpdateRequest,
    pub preview: SpecUpdate,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub output_file: Nullable<String>,
    pub transport: PrepareTransport,
    pub next_step: NextStep,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecVerificationRequest {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub artifacts: ArtifactPaths,
    pub record_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecVerification {
    pub schema: PublicSchema,
    pub status: String,
    pub verified: bool,
    pub record_id: String,
    pub requirement_id: String,
    pub artifacts: ArtifactPaths,
    pub task_collection_sha256: String,
    pub journal_sha256: String,
    pub verification_scope: String,
    pub execution_authorized: bool,
    pub next_step: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationChoice {
    All,
    Selective,
    RetainOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecReconciliationPrepareRequest {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub task_position: u64,
    pub attempt_position: u64,
    pub choice: ReconciliationChoice,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<SourceFingerprint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deviation_positions: Option<Vec<u64>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub reason: Option<Nullable<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edits: Option<Vec<SpecificationEdit>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_decisions: Option<Vec<Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecReconciliationPreviewRequest {
    pub schema: PublicSchema,
    pub attempt_path: String,
    pub choice: ReconciliationChoice,
    pub deviation_ids: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub migration: Nullable<SpecMigrationPreviewRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviationTarget {
    TaskOnly,
    TaskAndExecution,
    RetainOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationLedgerEntry {
    pub deviation_id: String,
    pub attempt_sha256: String,
    pub reconciliation_fingerprint: String,
    pub target: DeviationTarget,
    pub outcome: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationLedger {
    pub schema: String,
    pub attempt_path: String,
    pub entries: Vec<ReconciliationLedgerEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecReconciliationPreview {
    pub schema: PublicSchema,
    pub status: String,
    pub attempt_path: String,
    pub attempt_sha256: String,
    pub choice: ReconciliationChoice,
    pub pending_deviation_ids: Vec<String>,
    pub selected_deviation_ids: Vec<String>,
    pub retained_deviation_ids: Vec<String>,
    pub deviation_classifications: BTreeMap<String, DeviationTarget>,
    pub ledger_path: String,
    pub ledger: ReconciliationLedger,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub migration_preview: Nullable<SpecMigrationPreview>,
    pub fingerprint: String,
    pub publication_required: bool,
    pub publication_ready: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecReconciliationPublication {
    pub schema: PublicSchema,
    pub status: String,
    pub reconciliation_fingerprint: String,
    pub attempt_path: String,
    pub attempt_sha256: String,
    pub selected_deviation_ids: Vec<String>,
    pub retained_deviation_ids: Vec<String>,
    pub ledger_path: String,
    pub ledger_sha256: String,
    pub publication: SpecMigrationPublication,
    pub deviation_classifications: BTreeMap<String, DeviationTarget>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_migration_decisions_validate_and_round_trip() {
        let item = ArtifactMigrationItem {
            id: "MIGRATION-task-index".into(),
            path: "outputs/work/tasks/example/index.json".into(),
            kind: "task_index".into(),
            target_schema: "work-task-index".into(),
            required: true,
            source_sha256: "a".repeat(64),
            source_size: 3,
            raw: b"old".to_vec(),
            issue: "legacy schema".into(),
            resolution_status: ArtifactMigrationItemStatus::Proposed,
            proposed_content: Some(serde_json::json!({"schema":"work-task-index"})),
        };
        let mut request = ArtifactMigrationRequest {
            schema: PublicSchema::WorkArtifactMigrationRequest,
            requirement_id: "example".into(),
            analysis_fingerprint: "b".repeat(64),
            decisions: vec![ArtifactMigrationDecision {
                item,
                action: ArtifactMigrationAction::Apply,
                content: None,
                reason: None,
            }],
        };
        let serialized = serde_json::to_value(&request).unwrap();
        let mut missing = serialized.clone();
        missing["decisions"][0]["item"]
            .as_object_mut()
            .unwrap()
            .remove("raw");
        assert!(serde_json::from_value::<ArtifactMigrationRequest>(missing).is_err());
        let mut legacy = request.clone();
        legacy.decisions[0].item.kind = "plan".into();
        legacy.decisions[0].item.target_schema = "work-plan/v1".into();
        assert_eq!(legacy.validate(), Err("invalid_migration_item"));
        let mut wrong_size = request.clone();
        wrong_size.decisions[0].item.source_size += 1;
        assert_eq!(wrong_size.validate(), Err("invalid_migration_item"));
        assert_eq!(request.validate(), Ok(()));
        assert!(request.executable());
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(
            serde_json::from_value::<ArtifactMigrationRequest>(value).unwrap(),
            request
        );
        request.decisions[0].action = ArtifactMigrationAction::Skip;
        request.decisions[0].reason = Some("Deferred pending owner review".into());
        assert_eq!(request.validate(), Ok(()));
        assert!(!request.executable());
        request.decisions.push(request.decisions[0].clone());
        assert_eq!(request.validate(), Err("invalid_migration_item"));
        request.decisions.pop();
        request.decisions[0].item.required = false;
        assert!(request.executable());
        request.decisions[0].item.target_schema = "work-task-index/v0".into();
        assert_eq!(request.validate(), Err("invalid_migration_item"));
        request.decisions[0].item.target_schema = "work-task-index".into();
        request.decisions[0].item.resolution_status = ArtifactMigrationItemStatus::NeedsReview;
        assert_eq!(request.validate(), Err("invalid_migration_item"));
        request.decisions[0].item.resolution_status = ArtifactMigrationItemStatus::Proposed;
        request.decisions[0].action = ArtifactMigrationAction::Abort;
        assert_eq!(request.validate(), Err("invalid_migration_decision"));
    }

    #[test]
    fn public_specification_examples_match_models() {
        let registry: Value = crate::contract_data::registry_value();
        let items = &registry["items"];
        macro_rules! example {
            ($id:literal, $model:ty) => {
                serde_json::from_value::<$model>(items[$id]["description"]["example"].clone())
                    .unwrap_or_else(|error| panic!("{}: {error}", $id));
            };
        }
        example!(
            "work-spec-migration-prepare-request",
            SpecMigrationPrepareRequest
        );
        example!(
            "work-spec-migration-preview-request",
            SpecMigrationPreviewRequest
        );
        example!("work-spec-migration-preview", SpecMigrationPreview);
        example!("work-spec-migration-publication", SpecMigrationPublication);
        example!(
            "work-artifact-migration-analysis",
            ArtifactMigrationAnalysis
        );
        example!("work-artifact-migration-request", ArtifactMigrationRequest);
        example!("work-spec-prepare-request", SpecPrepareRequest);
        example!("work-spec-prepare", SpecPrepare);
        example!(
            "work-spec-reconciliation-prepare-request",
            SpecReconciliationPrepareRequest
        );
        example!(
            "work-spec-reconciliation-preview-request",
            SpecReconciliationPreviewRequest
        );
        example!(
            "work-spec-reconciliation-preview",
            SpecReconciliationPreview
        );
        example!(
            "work-spec-reconciliation-publication",
            SpecReconciliationPublication
        );
        example!("work-spec-transaction", SpecTransaction);
        example!("work-spec-update-request", SpecUpdateRequest);
        example!("work-spec-update", SpecUpdate);
        example!("work-spec-verification-request", SpecVerificationRequest);
        example!("work-spec-verification", SpecVerification);
    }
}
