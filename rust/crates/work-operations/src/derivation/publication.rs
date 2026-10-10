//! Deterministic publication evidence shared by Work storage adapters.

use crate::derivation::fingerprint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeOperation {
    ProjectFiles,
    AttemptStart,
    RecordBegin,
    CommandCorrection,
    RecordFinish,
    DeviationRecord,
    AttemptClose,
    Correction,
    SpecificationUpdate,
    SpecificationMigration,
    SpecificationMigrationItem,
    SpecificationMigrationReconcile,
    InstructionMigration,
    SourceRefresh,
}

impl RuntimeOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProjectFiles => "project-files",
            Self::AttemptStart => "attempt-start",
            Self::RecordBegin => "record-begin",
            Self::CommandCorrection => "command-correction",
            Self::RecordFinish => "record-finish",
            Self::DeviationRecord => "deviation-record",
            Self::AttemptClose => "attempt-close",
            Self::Correction => "correction",
            Self::SpecificationUpdate => "specification-update",
            Self::SpecificationMigration => "specification-migration",
            Self::SpecificationMigrationItem => "specification-migration-item",
            Self::SpecificationMigrationReconcile => "specification-migration-reconcile",
            Self::InstructionMigration => "instruction-migration",
            Self::SourceRefresh => "source-refresh",
        }
    }

    /// Execute inventories are complete fixed sets; journal targets are numbered separately.
    pub fn prepared_files(self) -> &'static [&'static str] {
        match self {
            Self::ProjectFiles => &["candidate.json"],
            Self::AttemptStart => &["index.locked.json.tmp", "index.started.json.tmp"],
            Self::RecordBegin | Self::CommandCorrection => &["index.json.tmp"],
            Self::RecordFinish | Self::AttemptClose => &["attempt.json.tmp", "index.json.tmp"],
            Self::DeviationRecord => &["attempt.json.tmp"],
            Self::Correction => &[
                "correction.json.tmp",
                "index.locked.json.tmp",
                "index.json.tmp",
            ],
            _ => &["journal.json.tmp"],
        }
    }
}

pub fn runtime_inventory(operation: RuntimeOperation, journal_target_count: usize) -> Vec<String> {
    let mut files = vec!["transaction.json".to_owned()];
    files.extend(
        operation
            .prepared_files()
            .iter()
            .map(|file| (*file).to_owned()),
    );
    if operation.prepared_files() == ["journal.json.tmp"] {
        files.extend((0..journal_target_count).map(|n| format!("targets/{n}.tmp")));
    }
    files
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceEntry {
    Request,
    Response,
    Envelope,
}

pub fn workspace_entry_path(
    workspace: &str,
    kind: WorkspaceEntry,
    step: usize,
    name: &str,
) -> Result<String, crate::specification::transaction::TransactionIssue> {
    if !crate::derivation::identity::runtime_relative_path(workspace)
        || !(1..=999).contains(&step)
        || name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || name.starts_with('-')
        || name.ends_with('-')
    {
        return Err(path_issue());
    }
    let directory = match kind {
        WorkspaceEntry::Request => "requests",
        WorkspaceEntry::Response => "responses",
        WorkspaceEntry::Envelope => "envelopes",
    };
    Ok(format!("{workspace}/{directory}/{step:03}-{name}.json"))
}

pub fn workspace_input_path(
    workspace: &str,
    original_name: &str,
) -> Result<String, crate::specification::transaction::TransactionIssue> {
    if !crate::derivation::identity::runtime_relative_path(workspace)
        || !crate::derivation::identity::runtime_relative_path(original_name)
        || original_name.contains('/')
    {
        return Err(path_issue());
    }
    Ok(format!("{workspace}/inputs/{original_name}"))
}

fn path_issue() -> crate::specification::transaction::TransactionIssue {
    crate::specification::transaction::TransactionIssue {
        reason_code: "runtime_artifact_path",
        message: "Runtime paths require complete validated identities and portable relative targets.",
        details: serde_json::json!({}),
    }
}

pub fn runtime_lock_path(
    requirement: &crate::identifiers::RequirementId,
    class: &str,
) -> Result<String, crate::specification::transaction::TransactionIssue> {
    if !matches!(class, "source" | "discussion" | "execution") {
        return Err(path_issue());
    }
    Ok(format!(
        "outputs/work/runtime/locks/{}/{class}.lock",
        requirement.as_str()
    ))
}

pub fn runtime_staging_path(
    requirement: &crate::identifiers::RequirementId,
    operation: RuntimeOperation,
    transaction_identity: &str,
) -> Result<String, crate::specification::transaction::TransactionIssue> {
    if !crate::protocol::valid_sha256(transaction_identity) {
        return Err(path_issue());
    }
    Ok(format!(
        "outputs/work/runtime/staging/{}/{}/{transaction_identity}",
        requirement.as_str(),
        operation.as_str()
    ))
}

pub fn source_capture_path(
    requirement: &crate::identifiers::RequirementId,
    source: &crate::identifiers::SourceId,
) -> String {
    format!(
        "outputs/work/runtime/staging/{}/source-capture/{}",
        requirement.as_str(),
        source.as_str()
    )
}

/// Derive the current retained journal layout.
pub fn retained_journal_path(
    execution_dir: &str,
    kind: JournalKind<'_>,
) -> Result<String, crate::specification::transaction::TransactionIssue> {
    if !crate::derivation::identity::runtime_relative_path(execution_dir) {
        return Err(path_issue());
    }
    let short =
        |digest: &str| -> Result<String, crate::specification::transaction::TransactionIssue> {
            if !crate::protocol::valid_sha256(digest) {
                return Err(path_issue());
            }
            Ok(digest[..12].to_ascii_uppercase())
        };
    let directory = match kind {
        JournalKind::SpecificationUpdate(id) => {
            if crate::identifiers::path_segment_issue(id).is_some() || id.contains(['/', '\\']) {
                return Err(path_issue());
            }
            format!("specification-update/{id}")
        }
        JournalKind::SpecificationMigration(approved) => {
            format!("specification-migration/{}", short(approved)?)
        }
        JournalKind::SpecificationMigrationItem { approved, position } => {
            if position >= 999 {
                return Err(path_issue());
            }
            format!(
                "specification-migration/{}/items/{:03}",
                short(approved)?,
                position + 1
            )
        }
        JournalKind::SpecificationMigrationReconcile(approved) => {
            format!("specification-migration/{}/reconcile", short(approved)?)
        }
        JournalKind::InstructionMigration(approved) => {
            format!("instruction-migration/{}", short(approved)?)
        }
        JournalKind::SourceRefresh(approved) => format!("source-refresh/{}", short(approved)?),
    };
    Ok(format!("{execution_dir}/journals/{directory}/journal.json"))
}

pub fn retained_journal_marker(
    journal: &str,
) -> Result<String, crate::specification::transaction::TransactionIssue> {
    if !crate::derivation::identity::runtime_relative_path(journal) {
        return Err(path_issue());
    }
    let directory = journal
        .strip_suffix("/journal.json")
        .ok_or_else(path_issue)?;
    Ok(format!("{directory}/committed.sha256"))
}

/// Parsed spelling is only a layout identity. Full approval is verified by binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedJournalAddress {
    pub operation: RuntimeOperation,
    pub identity: String,
    pub item_position: Option<usize>,
    pub family_directory: String,
    pub marker: String,
}

pub fn parse_retained_journal_path(
    execution_dir: &str,
    journal: &str,
) -> Result<RetainedJournalAddress, crate::specification::transaction::TransactionIssue> {
    if !crate::derivation::identity::runtime_relative_path(execution_dir)
        || !crate::derivation::identity::runtime_relative_path(journal)
    {
        return Err(path_issue());
    }
    let prefix = format!("{execution_dir}/journals/");
    let relative = journal.strip_prefix(&prefix).ok_or_else(path_issue)?;
    let segments = relative.split('/').collect::<Vec<_>>();
    let (operation, identity, item_position) = match segments.as_slice() {
        ["specification-update", id, "journal.json"]
            if (3..=64).contains(&id.len())
                && id.as_bytes()[0].is_ascii_uppercase()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-') =>
        {
            (RuntimeOperation::SpecificationUpdate, *id, None)
        }
        ["specification-migration", id, "journal.json"] => {
            (RuntimeOperation::SpecificationMigration, *id, None)
        }
        [
            "specification-migration",
            id,
            "items",
            number,
            "journal.json",
        ] if number.len() == 3
            && number.bytes().all(|b| b.is_ascii_digit())
            && *number != "000" =>
        {
            (
                RuntimeOperation::SpecificationMigrationItem,
                *id,
                Some(number.parse::<usize>().map_err(|_| path_issue())? - 1),
            )
        }
        ["specification-migration", id, "reconcile", "journal.json"] => {
            (RuntimeOperation::SpecificationMigrationReconcile, *id, None)
        }
        ["instruction-migration", id, "journal.json"] => {
            (RuntimeOperation::InstructionMigration, *id, None)
        }
        ["source-refresh", id, "journal.json"] => (RuntimeOperation::SourceRefresh, *id, None),
        _ => return Err(path_issue()),
    };
    if operation != RuntimeOperation::SpecificationUpdate
        && (identity.len() != 12
            || !identity
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b)))
    {
        return Err(path_issue());
    }
    Ok(RetainedJournalAddress {
        operation,
        identity: identity.to_owned(),
        item_position,
        family_directory: format!("{prefix}{}/{}", segments[0], identity),
        marker: retained_journal_marker(journal)?,
    })
}

/// Never treat a twelve-character directory name as a full approval proof.
pub fn bind_retained_journal_path(
    execution_dir: &str,
    journal: &str,
    approved_kind: JournalKind<'_>,
) -> Result<RetainedJournalAddress, crate::specification::transaction::TransactionIssue> {
    if retained_journal_path(execution_dir, approved_kind)? != journal {
        return Err(path_issue());
    }
    parse_retained_journal_path(execution_dir, journal)
}

/// Nested Migration journals share one reviewed approval and exclude that entire family.
pub fn retained_journal_history_includes(
    execution_dir: &str,
    journal: &str,
    current_journal: Option<&str>,
) -> Result<bool, crate::specification::transaction::TransactionIssue> {
    let address = parse_retained_journal_path(execution_dir, journal)?;
    let current = current_journal
        .map(|path| parse_retained_journal_path(execution_dir, path))
        .transpose()?;
    Ok(current.is_none_or(|current| current.family_directory != address.family_directory))
}

pub fn command_receipt_directory(
    execution_dir: &str,
    task_id: &str,
    attempt_id: &str,
    record_id: &str,
) -> Result<String, crate::specification::transaction::TransactionIssue> {
    let numbered = |id: &str, prefix: &str| {
        id.strip_prefix(prefix).is_some_and(|digits| {
            digits.len() == 3 && digits.bytes().all(|b| b.is_ascii_digit()) && digits != "000"
        })
    };
    let (base, retry) = record_id
        .split_once('#')
        .map_or((record_id, None), |(base, retry)| (base, Some(retry)));
    if !crate::derivation::identity::runtime_relative_path(execution_dir)
        || !numbered(task_id, "TASK-")
        || !numbered(attempt_id, "ATTEMPT-")
        || !numbered(base, "CMD-")
        || retry.is_some_and(|retry| {
            retry.is_empty() || retry.starts_with('0') || !retry.bytes().all(|b| b.is_ascii_digit())
        })
    {
        return Err(path_issue());
    }
    let instance = retry.map_or_else(|| base.to_owned(), |retry| format!("{base}-retry-{retry}"));
    Ok(format!(
        "{execution_dir}/{task_id}/{attempt_id}/receipts/{instance}"
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandReceiptPaths {
    pub directory: String,
    pub started: String,
    pub finished: String,
    /// Explicit refusal/upgrade evidence, never a normal writer fallback.
    pub legacy_started: String,
    pub legacy_finished: String,
}

pub fn command_receipt_paths(
    execution_dir: &str,
    task_id: &str,
    attempt_id: &str,
    record_id: &str,
) -> Result<CommandReceiptPaths, crate::specification::transaction::TransactionIssue> {
    let directory = command_receipt_directory(execution_dir, task_id, attempt_id, record_id)?;
    let legacy =
        super::legacy_layout::command_receipt_prefix(execution_dir, task_id, attempt_id, record_id);
    Ok(CommandReceiptPaths {
        started: format!("{directory}/started.json"),
        finished: format!("{directory}/finished.json"),
        directory,
        legacy_started: format!("{legacy}.started.json"),
        legacy_finished: format!("{legacy}.finished.json"),
    })
}

/// Decode only the canonical instance directory; semantic receipt bytes are checked separately.
pub fn command_receipt_instance(
    instance: &str,
) -> Result<String, crate::specification::transaction::TransactionIssue> {
    let record = instance.split_once("-retry-").map_or_else(
        || instance.to_owned(),
        |(base, retry)| format!("{base}#{retry}"),
    );
    let canonical = command_receipt_directory("execution", "TASK-001", "ATTEMPT-001", &record)?;
    if canonical.rsplit('/').next() != Some(instance) {
        return Err(path_issue());
    }
    Ok(record)
}

pub fn transaction_workspace_path(
    requirement: Option<&crate::identifiers::RequirementId>,
    workflow: &crate::identifiers::WorkflowId,
    transaction: &crate::identifiers::TransactionId,
) -> String {
    format!(
        "outputs/work/transactions/{}/{}/{}",
        requirement.map_or("pending", |id| id.as_str()),
        workflow.as_str(),
        transaction.as_str()
    )
}

pub fn completion_marker(journal_raw: &[u8]) -> Vec<u8> {
    format!("{}\n", fingerprint::raw(journal_raw)).into_bytes()
}

/// The marker for a published journal is adjacent to that journal.
pub fn completion_marker_path(journal_path: &str) -> String {
    retained_journal_marker(journal_path).expect("validated current journal path")
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
    retained_journal_path(execution_dir, kind).expect("validated current journal identities")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_journal_binding_and_history_cover_all_six_kinds() {
        let execution = "自訂 空白/執行";
        let approved = "ab".repeat(32);
        let kinds = [
            (
                JournalKind::SpecificationUpdate("UPDATE-ABCDEF123456"),
                RuntimeOperation::SpecificationUpdate,
            ),
            (
                JournalKind::SpecificationMigration(&approved),
                RuntimeOperation::SpecificationMigration,
            ),
            (
                JournalKind::SpecificationMigrationItem {
                    approved: &approved,
                    position: 998,
                },
                RuntimeOperation::SpecificationMigrationItem,
            ),
            (
                JournalKind::SpecificationMigrationReconcile(&approved),
                RuntimeOperation::SpecificationMigrationReconcile,
            ),
            (
                JournalKind::InstructionMigration(&approved),
                RuntimeOperation::InstructionMigration,
            ),
            (
                JournalKind::SourceRefresh(&approved),
                RuntimeOperation::SourceRefresh,
            ),
        ];
        let migration = retained_journal_path(execution, kinds[1].0).unwrap();
        for (kind, operation) in kinds {
            let path = retained_journal_path(execution, kind).unwrap();
            let bound = bind_retained_journal_path(execution, &path, kind).unwrap();
            assert_eq!(bound.operation, operation);
            assert_eq!(
                bound.marker,
                path.replace("journal.json", "committed.sha256")
            );
            assert!(!retained_journal_history_includes(execution, &path, Some(&path)).unwrap());
            assert!(retained_journal_history_includes(execution, &path, None).unwrap());
            assert_eq!(
                retained_journal_history_includes(execution, &path, Some(&migration)).unwrap(),
                !matches!(
                    operation,
                    RuntimeOperation::SpecificationMigration
                        | RuntimeOperation::SpecificationMigrationItem
                        | RuntimeOperation::SpecificationMigrationReconcile
                )
            );
            assert!(parse_retained_journal_path("other execution", &path).is_err());
        }
        for relative in [
            "specification-migration/abababababab/journal.json",
            "specification-migration/ABABABABABAB/items/000/journal.json",
            "specification-migration/ABABABABABAB/items/1/journal.json",
            "specification-migration/ABABABABABAB/items/1000/journal.json",
            "specification-migration/ABABABABABAB/items/001/foreign.json",
            "unknown/ABABABABABAB/journal.json",
            "source-refresh/ABABABABABAB/../journal.json",
            "specification-update/bad-id/journal.json",
        ] {
            assert!(
                parse_retained_journal_path(execution, &format!("{execution}/journals/{relative}"))
                    .is_err()
            );
        }
        assert!(
            bind_retained_journal_path(
                execution,
                &migration,
                JournalKind::SpecificationMigration(&"c".repeat(64))
            )
            .is_err()
        );
    }

    #[test]
    fn receipt_paths_share_one_instance_and_keep_legacy_evidence_explicit() {
        for (record, instance) in [
            ("CMD-001", "CMD-001"),
            ("CMD-001#1", "CMD-001-retry-1"),
            ("CMD-002#12", "CMD-002-retry-12"),
        ] {
            let paths =
                command_receipt_paths("自訂 空白/執行", "TASK-003", "ATTEMPT-002", record).unwrap();
            assert_eq!(
                paths.directory,
                format!("自訂 空白/執行/TASK-003/ATTEMPT-002/receipts/{instance}")
            );
            assert_eq!(paths.started, format!("{}/started.json", paths.directory));
            assert_eq!(paths.finished, format!("{}/finished.json", paths.directory));
            assert_eq!(
                paths.legacy_started,
                format!(
                    "自訂 空白/執行/TASK-003/ATTEMPT-002/.work-command-{instance}.started.json"
                )
            );
            assert_eq!(
                paths.legacy_finished,
                format!(
                    "自訂 空白/執行/TASK-003/ATTEMPT-002/.work-command-{instance}.finished.json"
                )
            );
            assert_eq!(command_receipt_instance(instance).unwrap(), record);
            let other =
                command_receipt_paths("自訂 空白/執行", "TASK-003", "ATTEMPT-003", record).unwrap();
            assert_ne!(paths.directory, other.directory);
        }
        for invalid in [
            "CMD-000",
            "CMD-1",
            "CMD-001#2",
            "CMD-001-retry-0",
            "CMD-001-retry-02",
            "CMD-001-retry-1/other",
            ".work-command-CMD-001",
            "CMD-001-retry-1-retry-2",
        ] {
            assert!(command_receipt_instance(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn all_runtime_families_have_exact_inventory_and_scoped_paths() {
        let requirement = "example".parse().unwrap();
        let digest = "a".repeat(64);
        for (operation, name, expected) in [
            (
                RuntimeOperation::AttemptStart,
                "attempt-start",
                vec![
                    "transaction.json",
                    "index.locked.json.tmp",
                    "index.started.json.tmp",
                ],
            ),
            (
                RuntimeOperation::RecordBegin,
                "record-begin",
                vec!["transaction.json", "index.json.tmp"],
            ),
            (
                RuntimeOperation::CommandCorrection,
                "command-correction",
                vec!["transaction.json", "index.json.tmp"],
            ),
            (
                RuntimeOperation::RecordFinish,
                "record-finish",
                vec!["transaction.json", "attempt.json.tmp", "index.json.tmp"],
            ),
            (
                RuntimeOperation::DeviationRecord,
                "deviation-record",
                vec!["transaction.json", "attempt.json.tmp"],
            ),
            (
                RuntimeOperation::AttemptClose,
                "attempt-close",
                vec!["transaction.json", "attempt.json.tmp", "index.json.tmp"],
            ),
            (
                RuntimeOperation::Correction,
                "correction",
                vec![
                    "transaction.json",
                    "correction.json.tmp",
                    "index.locked.json.tmp",
                    "index.json.tmp",
                ],
            ),
        ] {
            assert_eq!(runtime_inventory(operation, 0), expected);
            assert_eq!(
                runtime_staging_path(&requirement, operation, &digest).unwrap(),
                format!("outputs/work/runtime/staging/example/{name}/{digest}")
            );
        }
        for class in ["source", "discussion", "execution"] {
            assert_eq!(
                runtime_lock_path(&requirement, class).unwrap(),
                format!("outputs/work/runtime/locks/example/{class}.lock")
            );
        }
        assert_eq!(
            source_capture_path(&requirement, &"SRC-001".parse().unwrap()),
            "outputs/work/runtime/staging/example/source-capture/SRC-001"
        );
        assert_eq!(
            runtime_inventory(RuntimeOperation::SpecificationMigrationItem, 2),
            [
                "transaction.json",
                "journal.json.tmp",
                "targets/0.tmp",
                "targets/1.tmp"
            ]
        );
        assert!(runtime_lock_path(&requirement, "unknown").is_err());
        assert!(runtime_staging_path(&requirement, RuntimeOperation::Correction, "short").is_err());
    }

    #[test]
    fn journals_receipts_and_workspace_preserve_full_relative_context() {
        let approval = "a".repeat(64);
        let execution = "自訂 空白/execution";
        for (kind, directory) in [
            (
                JournalKind::SpecificationUpdate("SPEC-UPDATE-AAAAAAAAAAAA"),
                "specification-update/SPEC-UPDATE-AAAAAAAAAAAA",
            ),
            (
                JournalKind::SpecificationMigration(&approval),
                "specification-migration/AAAAAAAAAAAA",
            ),
            (
                JournalKind::SpecificationMigrationItem {
                    approved: &approval,
                    position: 2,
                },
                "specification-migration/AAAAAAAAAAAA/items/003",
            ),
            (
                JournalKind::SpecificationMigrationReconcile(&approval),
                "specification-migration/AAAAAAAAAAAA/reconcile",
            ),
            (
                JournalKind::InstructionMigration(&approval),
                "instruction-migration/AAAAAAAAAAAA",
            ),
            (
                JournalKind::SourceRefresh(&approval),
                "source-refresh/AAAAAAAAAAAA",
            ),
        ] {
            let journal = retained_journal_path(execution, kind).unwrap();
            assert_eq!(
                journal,
                format!("{execution}/journals/{directory}/journal.json")
            );
            assert_eq!(
                retained_journal_marker(&journal).unwrap(),
                format!("{execution}/journals/{directory}/committed.sha256")
            );
        }
        for (record, instance) in [("CMD-001", "CMD-001"), ("CMD-001#2", "CMD-001-retry-2")] {
            let directory =
                command_receipt_directory(execution, "TASK-001", "ATTEMPT-002", record).unwrap();
            assert_eq!(
                directory,
                format!("{execution}/TASK-001/ATTEMPT-002/receipts/{instance}")
            );
            assert_ne!(
                directory,
                command_receipt_directory(execution, "TASK-001", "ATTEMPT-003", record).unwrap()
            );
        }
        for record in [
            "CMD-000",
            "CMD-001#0",
            "CMD-001#02",
            "CMD-001#2/other",
            "OP-001",
        ] {
            assert!(
                command_receipt_directory(execution, "TASK-001", "ATTEMPT-001", record).is_err()
            );
        }
        assert!(
            retained_journal_path("../execution", JournalKind::SourceRefresh(&approval)).is_err()
        );
        assert!(
            retained_journal_path(execution, JournalKind::SpecificationMigration("short")).is_err()
        );
        assert!(
            retained_journal_path(
                execution,
                JournalKind::SpecificationMigrationItem {
                    approved: &approval,
                    position: 999
                }
            )
            .is_err()
        );
        let transaction = "20261006T010203Z-1234abcd".parse().unwrap();
        let invocation = "invocation".parse().unwrap();
        let pending = transaction_workspace_path(None, &invocation, &transaction);
        assert_eq!(
            pending,
            "outputs/work/transactions/pending/invocation/20261006T010203Z-1234abcd"
        );
        let formal = transaction_workspace_path(
            Some(&"example".parse().unwrap()),
            &invocation,
            &transaction,
        );
        assert!(formal.starts_with("outputs/work/transactions/example/invocation/"));
        for (kind, directory, name) in [
            (WorkspaceEntry::Request, "requests", "record-begin"),
            (WorkspaceEntry::Response, "responses", "record-begin"),
            (WorkspaceEntry::Envelope, "envelopes", "execute"),
        ] {
            assert_eq!(
                workspace_entry_path(&pending, kind, 1, name).unwrap(),
                format!("{pending}/{directory}/001-{name}.json")
            );
            assert_ne!(
                workspace_entry_path(&pending, kind, 1, name).unwrap(),
                workspace_entry_path(&pending, kind, 2, name).unwrap()
            );
        }
        assert_eq!(
            workspace_input_path(&pending, "來源 原文.pdf").unwrap(),
            format!("{pending}/inputs/來源 原文.pdf")
        );
        assert!(workspace_input_path(&pending, "../source.txt").is_err());
        assert!(workspace_entry_path(&pending, WorkspaceEntry::Request, 0, "capture").is_err());
    }

    #[test]
    fn marker_is_the_journal_digest_with_one_newline() {
        assert_eq!(
            completion_marker(b"{}\n"),
            b"ca3d163bab055381827226140568f3bef7eaac187cebd76878e0b63e9e442356\n"
        );
        assert_eq!(
            journal_path(
                "execution",
                JournalKind::SpecificationMigration("a".repeat(64).as_str())
            ),
            { "execution/journals/specification-migration/AAAAAAAAAAAA/journal.json" }
        );
        assert_eq!(
            journal_path(
                "execution",
                JournalKind::SpecificationMigrationItem {
                    approved: &"a".repeat(64),
                    position: 2,
                }
            ),
            { "execution/journals/specification-migration/AAAAAAAAAAAA/items/003/journal.json" }
        );
    }
}
