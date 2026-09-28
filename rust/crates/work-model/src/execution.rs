//! Persisted execution artifacts and their fixed data shapes.

pub mod attempt;
pub mod correction;
pub mod deviation;
pub mod index;
pub mod recovery;
pub mod request;
pub mod response;

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
        example::<request::AttemptCloseRequest>(&registry, "work-attempt-close-request/v1");
        example::<request::AttemptStartPrepareRequest>(
            &registry,
            "work-attempt-start-prepare-request/v1",
        );
        example::<request::AttemptStartRequest>(&registry, "work-attempt-start-request/v1");
        example::<request::CommandCorrectionRequest>(
            &registry,
            "work-command-correction-request/v1",
        );
        example::<request::CommandRunRequest>(&registry, "work-command-run-request/v1");
        example::<request::CorrectionCreateRequest>(&registry, "work-correction-create-request/v1");
        example::<request::DeviationSemanticRequest>(
            &registry,
            "work-execution-deviation-semantic-request/v1",
        );
        example::<request::ExecutionRecoveryPrepareRequest>(
            &registry,
            "work-execution-recovery-prepare-request/v1",
        );
        example::<request::ExecutionRecoveryRequest>(
            &registry,
            "work-execution-recovery-request/v1",
        );
        example::<request::RecordFinishRequest>(&registry, "work-record-finish-request/v1");
        example::<response::AttemptCloseResponse>(&registry, "work-attempt-close/v1");
        example::<response::AttemptStartPrepareResponse>(
            &registry,
            "work-attempt-start-prepare/v1",
        );
        example::<response::AttemptStartRecoveryResponse>(
            &registry,
            "work-attempt-start-recovery/v1",
        );
        example::<response::AttemptStartResponse>(&registry, "work-attempt-start/v1");
        example::<response::AttemptValidation>(&registry, "work-attempt-validation/v1");
        example::<response::CommandCorrectionResponse>(&registry, "work-command-correction/v1");
        example::<response::CommandPreview>(&registry, "work-command-preview/v1");
        example::<response::CommandResult>(&registry, "work-command-result/v1");
        example::<response::CorrectionCreateResponse>(&registry, "work-correction-create/v1");
        example::<response::ExecutePreflight>(&registry, "work-execute-preflight/v1");
        example::<response::WorktreeSnapshot>(&registry, "work-execute-worktree-snapshot/v1");
        example::<response::ExecuteWorktree>(&registry, "work-execute-worktree/v1");
        example::<response::DeviationPreview>(&registry, "work-execution-deviation-preview/v1");
        example::<deviation::DeviationProposal>(&registry, "work-execution-deviation-proposal/v1");
        example::<response::DeviationRecordResponse>(
            &registry,
            "work-execution-deviation-record/v1",
        );
        example::<response::ExecutionRecoveryPrepare>(
            &registry,
            "work-execution-recovery-prepare/v1",
        );
        example::<response::ExecutionRecoveryResponse>(&registry, "work-execution-recovery/v1");
        example::<response::RecordBeginResponse>(&registry, "work-record-begin/v1");
        example::<response::RecordFinishResponse>(&registry, "work-record-finish/v1");
    }
}
