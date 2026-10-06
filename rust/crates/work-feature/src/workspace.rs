//! Transaction workspace identity and allocation order.

use serde_json::{Value, json};
use work_operations::derivation::identity::workspace_transaction_id;
use work_operations::identifiers::{RequirementId, WorkflowId};

use crate::error::{ExitCode, WorkError};

pub trait WorkspaceAllocator {
    fn random_suffix(&self) -> Result<String, WorkError>;
    fn utc_stamp(&self) -> String;
    fn allocate(&self, relative: &str) -> Result<String, WorkError>;
}

fn invalid_identifier(reason: &str) -> WorkError {
    WorkError::new(
        ExitCode::Contract,
        reason,
        "The transaction workspace identifier is invalid.",
        json!({}),
    )
}

pub fn create(
    allocator: &impl WorkspaceAllocator,
    requirement_id: Option<&str>,
    workflow_id: &str,
) -> Result<Value, WorkError> {
    if let Some(raw) = requirement_id {
        if raw == "pending" {
            return Err(WorkError::new(
                ExitCode::Contract,
                "reserved_transaction_owner",
                "The pending transaction owner is reserved for work without a requirement ID.",
                json!({"requirement_id": raw}),
            ));
        }
        raw.parse::<RequirementId>()
            .map_err(|issue| invalid_identifier(issue.reason_code()))?;
    }
    let workflow = workflow_id
        .parse::<WorkflowId>()
        .map_err(|issue| invalid_identifier(issue.reason_code()))?;
    let suffix = allocator.random_suffix()?;
    let transaction_id = workspace_transaction_id(&allocator.utc_stamp(), &suffix)
        .map_err(|issue| invalid_identifier(issue.reason_code()))?;
    let requirement = requirement_id
        .map(str::parse::<RequirementId>)
        .transpose()
        .map_err(|issue| invalid_identifier(issue.reason_code()))?;
    let relative = work_operations::derivation::publication::transaction_workspace_path(
        requirement.as_ref(),
        &workflow,
        &transaction_id,
    );
    let relative = allocator.allocate(&relative)?;
    let mut result = json!({"schema":"work-transaction-workspace","requirement_id":requirement_id,
        "workflow_id":workflow_id,"transaction_id":transaction_id.as_str(),"path":relative});
    result["paths"] = json!({"inputs":format!("{relative}/inputs"),"requests":format!("{relative}/requests"),"responses":format!("{relative}/responses"),"envelopes":format!("{relative}/envelopes")});
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    struct Allocator(RefCell<Vec<&'static str>>);

    impl WorkspaceAllocator for Allocator {
        fn random_suffix(&self) -> Result<String, WorkError> {
            self.0.borrow_mut().push("random");
            Ok("01234567".to_owned())
        }
        fn utc_stamp(&self) -> String {
            self.0.borrow_mut().push("clock");
            "20260927T000000Z".to_owned()
        }
        fn allocate(&self, relative: &str) -> Result<String, WorkError> {
            self.0.borrow_mut().push("allocate");
            assert_eq!(relative, {
                "outputs/work/transactions/pending/invocation/20260927T000000Z-01234567"
            });
            Ok(relative.to_owned())
        }
    }

    #[test]
    fn rejects_invalid_identity_before_allocating_and_allocates_in_order() {
        let allocator = Allocator(RefCell::new(Vec::new()));
        assert_eq!(
            create(&allocator, Some("pending"), "invocation")
                .unwrap_err()
                .reason_code,
            "reserved_transaction_owner"
        );
        assert!(allocator.0.borrow().is_empty());
        let result = create(&allocator, None, "invocation").unwrap();
        assert_eq!(result["path"], {
            "outputs/work/transactions/pending/invocation/20260927T000000Z-01234567"
        });
        assert_eq!(*allocator.0.borrow(), ["random", "clock", "allocate"]);
    }
}
