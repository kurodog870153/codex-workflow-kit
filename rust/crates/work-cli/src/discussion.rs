//! Public Session and formal publication bridge.

use serde_json::{Value, json};
use std::path::Path;
use work_flow::error::{ExitCode, WorkError};
use work_infrastructure::discussion::assembly::LocalDiscussionAssembly;
use work_infrastructure::discussion::storage::LocalDiscussionStorage;
use work_infrastructure::files::LocalFiles;
use work_infrastructure::skill_catalog::SkillRootConfig;
use work_model::discussion::request::DiscussionRequest;

pub fn dispatch(
    project_root: &Path,
    skill_root: &Path,
    skills: &[SkillRootConfig],
    operation: &str,
    input: Value,
) -> Result<Value, WorkError> {
    let request: DiscussionRequest = serde_json::from_value(input).map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_discussion_request",
            "An exact structured discussion request is required.",
            json!({}),
        )
    })?;
    if request.command.name() != operation {
        return Err(WorkError::new(
            ExitCode::Contract,
            "discussion_command_mismatch",
            "The request must match the invoked discussion command.",
            json!({}),
        ));
    }
    let repository = LocalDiscussionStorage {
        project_root: project_root.to_path_buf(),
        files: LocalFiles,
    };
    let publication = LocalDiscussionAssembly {
        project_root: project_root.to_path_buf(),
        skill_root: skill_root.to_path_buf(),
        skill_configs: skills.to_vec(),
    };
    work_flow::discussion::dispatch(&repository, &publication, &request)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_or_mismatched_request_is_rejected_before_storage_access() {
        let missing = Path::new("/missing-discussion-project");
        assert_eq!(
            dispatch(missing, missing, &[], "read", json!({}))
                .unwrap_err()
                .reason_code,
            "invalid_discussion_request"
        );
        let request = json!({"schema":"work-discussion-request","requirement_id":"example","command":{"kind":"read"}});
        assert_eq!(
            dispatch(missing, missing, &[], "update", request)
                .unwrap_err()
                .reason_code,
            "discussion_command_mismatch"
        );
    }
}
