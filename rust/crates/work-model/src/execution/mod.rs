//! Persisted execution artifacts and their fixed data shapes.

pub mod acceptance;
pub mod attempt;
pub mod correction;
pub mod deviation;
pub mod file_transaction;
pub mod file_transaction_contracts;
pub mod index;
pub mod recovery;
pub mod request;
pub mod response;

/// Internal capability issued by a trusted executor, never by Task JSON or argv.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifiedEffectBoundary {
    Unknown,
    ReadOnly,
    /// Enforced isolation must also cover external effects, not just a temp directory.
    Isolated,
}

/// Executor-produced policy stored inside the preview's execution environment.
/// Neither a Task declaration nor this DTO alone grants execution authority.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandIsolation {
    pub backend: String,
    pub read_only_project_root: String,
    /// Physical read view; Windows executes relative paths in a private snapshot.
    pub read_only_project_view: String,
    pub writable_directory: String,
    pub network_access: bool,
}

#[cfg(test)]
mod tests {
    use super::{deviation, request, response};
    use serde::de::DeserializeOwned;
    use serde_json::Value;

    fn example<T: DeserializeOwned>(registry: &Value, id: &str) {
        let value = registry["items"][id]["description"]["example"].clone();
        serde_json::from_value::<T>(value)
            .unwrap_or_else(|error| panic!("{id} example does not match its model: {error}"));
    }

    #[test]
    fn public_execution_examples_match_model_shapes() {
        let registry: Value = crate::contract_data::registry_value();
        example::<request::AttemptCloseRequest>(&registry, "work-attempt-close-request");
        example::<request::AttemptStartPrepareRequest>(
            &registry,
            "work-attempt-start-prepare-request",
        );
        example::<request::AttemptStartRequest>(&registry, "work-attempt-start-request");
        example::<request::CommandCorrectionRequest>(&registry, "work-command-correction-request");
        example::<request::CommandRunRequest>(&registry, "work-command-run-request");
        example::<request::CorrectionCreateRequest>(&registry, "work-correction-create-request");
        example::<request::DeviationSemanticRequest>(
            &registry,
            "work-execution-deviation-semantic-request",
        );
        example::<request::ExecutionRecoveryPrepareRequest>(
            &registry,
            "work-execution-recovery-prepare-request",
        );
        example::<request::ExecutionRecoveryRequest>(&registry, "work-execution-recovery-request");
        example::<request::RecordFinishRequest>(&registry, "work-record-finish-request");
        example::<response::AttemptCloseResponse>(&registry, "work-attempt-close");
        example::<response::AttemptStartPrepareResponse>(&registry, "work-attempt-start-prepare");
        example::<response::AttemptStartRecoveryResponse>(&registry, "work-attempt-start-recovery");
        example::<response::AttemptStartResponse>(&registry, "work-attempt-start");
        example::<response::AttemptValidation>(&registry, "work-attempt-validation");
        example::<response::CommandCorrectionResponse>(&registry, "work-command-correction");
        example::<response::CommandPreview>(&registry, "work-command-preview");
        example::<response::CommandResult>(&registry, "work-command-result");
        example::<response::CorrectionCreateResponse>(&registry, "work-correction-create");
        example::<response::ExecutePreflight>(&registry, "work-execute-preflight");
        example::<response::WorktreeSnapshot>(&registry, "work-execute-worktree-snapshot");
        example::<response::ExecuteWorktree>(&registry, "work-execute-worktree");
        example::<response::DeviationPreview>(&registry, "work-execution-deviation-preview");
        example::<deviation::DeviationProposal>(&registry, "work-execution-deviation-proposal");
        example::<response::DeviationRecordResponse>(&registry, "work-execution-deviation-record");
        example::<response::ExecutionRecoveryPrepare>(&registry, "work-execution-recovery-prepare");
        example::<response::ExecutionRecoveryResponse>(&registry, "work-execution-recovery");
        example::<response::RecordBeginResponse>(&registry, "work-record-begin");
        example::<response::RecordFinishResponse>(&registry, "work-record-finish");
    }
}
