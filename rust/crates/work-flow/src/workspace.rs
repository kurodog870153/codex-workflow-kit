//! Transaction workspace command flow.

use serde_json::Value;
use work_feature::error::WorkError;
use work_feature::workspace::WorkspaceAllocator;

pub fn create(
    allocator: &impl WorkspaceAllocator,
    requirement_id: Option<&str>,
    workflow_id: &str,
) -> Result<Value, WorkError> {
    work_feature::workspace::create(allocator, requirement_id, workflow_id)
}
