//! Deterministic publication evidence shared by Work storage adapters.

use crate::derivation::fingerprint;

pub fn completion_marker(journal_raw: &[u8]) -> Vec<u8> {
    format!("{}\n", fingerprint::raw(journal_raw)).into_bytes()
}

/// The marker for a published journal is adjacent to that journal.
pub fn completion_marker_path(journal_path: &str) -> String {
    format!("{journal_path}.done")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalKind<'a> {
    InstructionMigration(&'a str),
    SourceRefresh(&'a str),
    SpecificationUpdate(&'a str),
    SpecificationMigration(&'a str),
    SpecificationMigrationItem { approved: &'a str, position: usize },
    SpecificationMigrationReconcile(&'a str),
}

/// Derive journal names from approved identities and the execution directory.
pub fn journal_path(execution_dir: &str, kind: JournalKind<'_>) -> String {
    let short = |digest: &str| {
        digest
            .chars()
            .take(12)
            .collect::<String>()
            .to_ascii_uppercase()
    };
    let name = match kind {
        JournalKind::InstructionMigration(approved) => {
            format!(".work-instruction-migration-{}.json", short(approved))
        }
        JournalKind::SourceRefresh(approved) => {
            format!(".work-source-refresh-{}.json", short(approved))
        }
        JournalKind::SpecificationUpdate(id) => format!(".work-spec-update-{id}.json"),
        JournalKind::SpecificationMigration(approved) => {
            format!(".work-spec-migration-{}.json", short(approved))
        }
        JournalKind::SpecificationMigrationItem { approved, position } => format!(
            ".work-spec-migration-{}-{:03}.json",
            short(approved),
            position + 1
        ),
        JournalKind::SpecificationMigrationReconcile(request) => {
            format!(".work-spec-migration-{}-reconcile.json", short(request))
        }
    };
    format!("{execution_dir}/{name}")
}

/// Command execution evidence is keyed by the allocated TASK, Attempt and record IDs.
pub fn command_receipt_prefix(
    execution_dir: &str,
    task_id: &str,
    attempt_id: &str,
    record_id: &str,
) -> String {
    format!(
        "{execution_dir}/{task_id}/{attempt_id}/.work-command-{}",
        record_id.replace('#', "-retry-")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_is_the_journal_digest_with_one_newline() {
        assert_eq!(
            completion_marker(b"{}\n"),
            b"ca3d163bab055381827226140568f3bef7eaac187cebd76878e0b63e9e442356\n"
        );
        assert_eq!(
            completion_marker_path("execution/.work-spec-update.json"),
            "execution/.work-spec-update.json.done"
        );
        assert_eq!(
            command_receipt_prefix("execution", "TASK-001", "ATTEMPT-002", "CMD-001#3"),
            "execution/TASK-001/ATTEMPT-002/.work-command-CMD-001-retry-3"
        );
        assert_eq!(
            journal_path(
                "execution",
                JournalKind::SpecificationMigration("a".repeat(64).as_str())
            ),
            "execution/.work-spec-migration-AAAAAAAAAAAA.json"
        );
        assert_eq!(
            journal_path(
                "execution",
                JournalKind::SpecificationMigrationItem {
                    approved: &"a".repeat(64),
                    position: 2,
                }
            ),
            "execution/.work-spec-migration-AAAAAAAAAAAA-003.json"
        );
    }
}
