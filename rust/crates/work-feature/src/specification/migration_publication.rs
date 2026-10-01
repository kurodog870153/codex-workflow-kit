//! Approval and path decisions for migration publication.

use crate::error::{ExitCode, WorkError};
use serde_json::{Value, json};

pub struct MigrationPublicationPaths {
    pub execution: String,
    pub journal: String,
    pub marker: String,
}

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

pub fn publication_paths(
    request: &Value,
    approved_sha256: &str,
) -> Result<MigrationPublicationPaths, WorkError> {
    let plan = request["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["kind"] == "plan")
        .ok_or_else(|| {
            fail(
                "migration_execution_directory",
                "A candidate execution directory is required.",
            )
        })?;
    let execution = plan["content"]["artifacts"]["execution"]
        .as_str()
        .ok_or_else(|| {
            fail(
                "migration_execution_directory",
                "A candidate execution directory is required.",
            )
        })?;
    if approved_sha256.len() != 64 || !approved_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(fail(
            "migration_approval_changed",
            "The approved migration fingerprint is invalid.",
        ));
    }
    let journal = work_operations::derivation::publication::journal_path(
        execution,
        work_operations::derivation::publication::JournalKind::SpecificationMigration(
            approved_sha256,
        ),
    );
    let marker = work_operations::derivation::publication::completion_marker_path(&journal);
    Ok(MigrationPublicationPaths {
        execution: execution.to_owned(),
        journal,
        marker,
    })
}

pub fn require_approved_preview(preview: &Value, approved_sha256: &str) -> Result<(), WorkError> {
    if preview["writable_ready"] != true {
        return Err(fail(
            "migration_not_writable",
            "The migration preview is not ready for publication.",
        ));
    }
    if preview["fingerprint"] != approved_sha256 {
        return Err(fail(
            "migration_approval_changed",
            "The approved migration fingerprint changed.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_path_uses_approved_fingerprint() {
        let request = json!({"candidates":[{"kind":"plan","content":{
            "artifacts":{"execution":"outputs/work/executions/example"}}}]});
        let paths = publication_paths(&request, &"a".repeat(64)).unwrap();
        assert_eq!(
            paths.journal,
            "outputs/work/executions/example/.work-spec-migration-AAAAAAAAAAAA.json"
        );
        assert_eq!(paths.marker, format!("{}.done", paths.journal));
    }

    #[test]
    fn blocked_preview_precedes_fingerprint_check() {
        let error = require_approved_preview(&json!({"writable_ready":false}), "invalid")
            .expect_err("blocked preview must fail");
        assert_eq!(error.reason_code, "migration_not_writable");
    }
}
