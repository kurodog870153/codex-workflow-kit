//! Work data shapes and single-type invariants.

pub mod common;
pub mod contract;
pub mod contract_data;
pub mod delegation;
pub mod execution;
pub mod handoff;
pub mod hierarchy;
pub mod identifiers;
pub mod instruction;
pub mod invocation;
pub mod operation;
pub mod plan;
pub mod progress;
pub mod schema;
pub mod skill;
pub mod source;
pub mod specification;
pub mod task;
pub mod workflow;

#[cfg(test)]
mod remaining_contract_examples {
    use crate::{delegation, handoff, invocation, operation, source};
    use serde::de::DeserializeOwned;
    use serde_json::Value;

    fn matches<T: DeserializeOwned>(items: &Value, id: &str) {
        serde_json::from_value::<T>(items[id]["description"]["example"].clone())
            .unwrap_or_else(|error| panic!("{id}: {error}"));
    }

    #[test]
    fn public_remaining_examples_match_models() {
        let registry: Value = crate::contract_data::registry_value();
        let items = &registry["items"];
        matches::<delegation::DelegationBuildRequest>(items, "work-delegation-build-request/v1");
        matches::<delegation::DelegationEnvelope>(items, "work-delegation-envelope/v1");
        matches::<delegation::DelegationValidation>(items, "work-delegation-validation/v1");
        matches::<handoff::DiscussionHandoffRequest>(items, "work-discussion-handoff-request/v1");
        matches::<handoff::DiscussionHandoff>(items, "work-discussion-handoff/v1");
        matches::<handoff::HandoffSourceValidation>(items, "work-handoff-source-validation/v1");
        matches::<handoff::HandoffValidation>(items, "work-handoff-validation/v1");
        matches::<handoff::FormalHandoff>(items, "work-handoff/v1");
        matches::<invocation::Invocation>(items, "work-invocation/v1");
        matches::<operation::OperationEnvelope>(items, "work-operation-envelope/v1");
        matches::<operation::OperationResult>(items, "work-operation-result/v1");
        matches::<source::SourceImpact>(items, "work-source-impact/v1");
        matches::<source::SourceRefreshPreview>(items, "work-source-refresh-preview/v1");
        matches::<source::SourceRefreshPublication>(items, "work-source-refresh-publication/v1");
    }
}
