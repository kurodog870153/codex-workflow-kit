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
    publication_paths_with_layout(request, approved_sha256)
}

fn publication_paths_with_layout(
    request: &Value,
    approved_sha256: &str,
) -> Result<MigrationPublicationPaths, WorkError> {
    let index = request["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["kind"] == "task_index")
        .ok_or_else(|| {
            fail(
                "migration_execution_directory",
                "A candidate execution directory is required.",
            )
        })?;
    let execution = index["content"]["artifacts"]["execution"]
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
    let kind = work_operations::derivation::publication::JournalKind::SpecificationMigration(
        approved_sha256,
    );
    let journal = {
        work_operations::derivation::publication::retained_journal_path(execution, kind).map_err(
            |_| {
                fail(
                    "migration_publication_identity",
                    "A complete approved fingerprint and portable execution path are required.",
                )
            },
        )?
    };
    let marker = {
        work_operations::derivation::publication::retained_journal_marker(&journal).map_err(
            |_| {
                fail(
                    "migration_publication_identity",
                    "The retained journal path is invalid.",
                )
            },
        )?
    };
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
        let request = json!({"candidates":[{"kind":"task_index","content":{
            "artifacts":{"execution":"outputs/work/executions/example"}}}]});
        let paths = publication_paths(&request, &"a".repeat(64)).unwrap();
        assert_eq!(paths.journal, {
            "outputs/work/executions/example/journals/specification-migration/AAAAAAAAAAAA/journal.json"
        });
        assert_eq!(paths.marker, {
            "outputs/work/executions/example/journals/specification-migration/AAAAAAAAAAAA/committed.sha256".to_owned()
        });
        let mut legacy = request.clone();
        legacy["candidates"][0]["kind"] = json!("plan");
        assert!(publication_paths(&legacy, &"a".repeat(64)).is_err());
    }

    #[test]
    fn retained_paths_validate_full_digest_and_custom_scope_without_panics() {
        let mut request = json!({"candidates":[{"kind":"task_index","content":{"artifacts":{"execution":"custom execution/例"}}}]});
        let paths = publication_paths_with_layout(&request, &"ab".repeat(32)).unwrap();
        assert_eq!(
            paths.journal,
            "custom execution/例/journals/specification-migration/ABABABABABAB/journal.json"
        );
        assert_eq!(
            paths.marker,
            "custom execution/例/journals/specification-migration/ABABABABABAB/committed.sha256"
        );
        for digest in ["A".repeat(64), "a".repeat(12), "g".repeat(64)] {
            assert!(publication_paths_with_layout(&request, &digest).is_err());
        }
        for execution in [
            "../execution",
            "/execution",
            "execution//other",
            "execution/../other",
        ] {
            request["candidates"][0]["content"]["artifacts"]["execution"] = json!(execution);
            assert!(publication_paths_with_layout(&request, &"a".repeat(64)).is_err());
        }
    }

    #[test]
    fn blocked_preview_precedes_fingerprint_check() {
        let error = require_approved_preview(&json!({"writable_ready":false}), "invalid")
            .expect_err("blocked preview must fail");
        assert_eq!(error.reason_code, "migration_not_writable");
    }
}
