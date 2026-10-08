//! Specification transaction evidence, progress and canonical journal rules.

use std::collections::BTreeSet;

use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use serde_json::{Value, json};

#[cfg(test)]
use crate::canonical::sha256_hex;
use crate::derivation::identity::derived_transaction_id;
use crate::derivation::publication::completion_marker;
use crate::derivation::snapshot::decode_snapshot;
#[cfg(test)]
use crate::derivation::snapshot::encode_snapshot;
use crate::derivation::transaction::approval_sha256;
use crate::protocol::valid_sha256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionIssue {
    pub reason_code: &'static str,
    pub message: &'static str,
    pub details: Value,
}

fn issue(reason_code: &'static str, message: &'static str) -> TransactionIssue {
    TransactionIssue {
        reason_code,
        message,
        details: json!({}),
    }
}

fn strict(value: &Value, required: &[&str], optional: &[&str]) -> Result<(), TransactionIssue> {
    let Some(object) = value.as_object() else {
        return Err(issue(
            "invalid_contract_value",
            "The JSON contract contains an invalid value.",
        ));
    };
    if required.iter().any(|field| !object.contains_key(*field))
        || object
            .keys()
            .any(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
    {
        return Err(issue(
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
        ));
    }
    Ok(())
}

fn sha(value: &Value) -> bool {
    value.as_str().is_some_and(valid_sha256)
}

pub fn validate_transaction(value: &Value) -> Result<(), TransactionIssue> {
    strict(
        value,
        &[
            "schema",
            "transaction_id",
            "approval_sha256",
            "state",
            "published_count",
            "metadata",
            "files",
        ],
        &[],
    )?;
    if value["schema"] != "work-spec-transaction" || !sha(&value["approval_sha256"]) {
        return Err(issue(
            "invalid_contract_value",
            "The specification transaction identity is invalid.",
        ));
    }
    let id = value["transaction_id"]
        .as_str()
        .filter(|id| {
            id.len() >= 3
                && id.len() <= 64
                && id
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_uppercase())
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-')
        })
        .ok_or_else(|| issue("invalid_contract_value", "The transaction ID is invalid."))?;
    strict(
        &value["metadata"],
        &[
            "request",
            "artifacts",
            "affected_task_ids",
            "history_sha256",
            "source_sha256",
            "candidate_sha256",
        ],
        &[],
    )?;
    if !value["metadata"]["request"].is_object()
        || !value["metadata"]["artifacts"].is_object()
        || !value["metadata"]["affected_task_ids"].is_array()
        || ["history_sha256", "source_sha256", "candidate_sha256"]
            .iter()
            .any(|field| !value["metadata"][*field].is_object())
    {
        return Err(issue(
            "invalid_contract_value",
            "The transaction metadata is invalid.",
        ));
    }
    for field in ["history_sha256", "source_sha256", "candidate_sha256"] {
        if value["metadata"][field]
            .as_object()
            .expect("checked map")
            .values()
            .any(|hash| !sha(hash))
        {
            return Err(issue(
                "invalid_contract_value",
                "The transaction metadata fingerprint is invalid.",
            ));
        }
    }
    let files = value["files"]
        .as_array()
        .filter(|files| !files.is_empty())
        .ok_or_else(|| {
            issue(
                "invalid_contract_value",
                "The transaction needs at least one file.",
            )
        })?;
    let mut previous: Option<(u64, String)> = None;
    let mut paths = BTreeSet::new();
    for file in files {
        let operation = file["operation"].as_str();
        let required: &[&str] = match operation {
            Some("add") => &["phase", "path", "operation", "after"],
            Some("replace") => &["phase", "path", "operation", "before", "after"],
            Some("remove") => &["phase", "path", "operation", "before"],
            _ => {
                return Err(issue(
                    "invalid_contract_value",
                    "The transaction operation is invalid.",
                ));
            }
        };
        strict(file, required, &[])?;
        let phase = file["phase"].as_u64().ok_or_else(|| {
            issue(
                "invalid_contract_value",
                "The transaction phase is invalid.",
            )
        })?;
        let path = file["path"]
            .as_str()
            .filter(|path| {
                !path.is_empty()
                    && !path.starts_with('/')
                    && !path.contains('\\')
                    && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
            })
            .ok_or_else(|| {
                issue(
                    "invalid_contract_value",
                    "Transaction paths must be safe project-relative POSIX paths.",
                )
            })?;
        if previous
            .as_ref()
            .is_some_and(|(old_phase, old_path)| (*old_phase, old_path.as_str()) > (phase, path))
            || !paths.insert(path)
        {
            return Err(issue(
                "invalid_contract_value",
                "Transaction files must have unique paths in phase and lexical order.",
            ));
        }
        previous = Some((phase, path.into()));
        if file.get("before").is_some() {
            decode_snapshot(&file["before"])?;
        }
        if file.get("after").is_some() {
            decode_snapshot(&file["after"])?;
        }
    }
    let expected = approval_sha256(&value["files"], &value["metadata"]);
    if value["approval_sha256"] != expected {
        return Err(issue(
            "invalid_contract_value",
            "The approval fingerprint does not match the file set.",
        ));
    }
    for kind in ["UPDATE", "MIGRATION", "RECONCILIATION"] {
        if id.starts_with(&format!("SPEC-{kind}-"))
            && id != derived_transaction_id(kind, &expected)?
        {
            return Err(issue(
                "invalid_contract_value",
                "The transaction ID must derive from approved contents.",
            ));
        }
    }
    let count = value["published_count"]
        .as_u64()
        .filter(|count| *count <= files.len() as u64)
        .ok_or_else(|| {
            issue(
                "invalid_contract_value",
                "The published file count is invalid.",
            )
        })?;
    let state = if count == 0 {
        "prepared"
    } else if count == files.len() as u64 {
        "published"
    } else {
        "publishing"
    };
    if value["state"] != state {
        return Err(issue(
            "invalid_contract_value",
            "The transaction state does not match its progress.",
        ));
    }
    let _: work_model::specification::SpecTransaction = serde_json::from_value(value.clone())
        .expect("validated Specification transaction matches its model");
    Ok(())
}

struct Ordered<'a> {
    value: &'a Value,
    path: Vec<String>,
    document: Option<String>,
}

fn order(path: &[String], document: Option<&str>) -> &'static [&'static str] {
    if document == Some("work-spec-update-request")
        && path.starts_with(&["metadata".to_owned(), "request".to_owned()])
    {
        let nested = &path[2..];
        if nested.is_empty() {
            return &["schema", "reason", "expected", "task_index", "task_items"];
        }
        if nested == ["expected"] {
            return &[
                "source_sha256",
                "task_index_sha256",
                "execution_index_sha256",
                "task_item_sha256",
            ];
        }
        if nested[0] == "task_index" {
            return crate::task::ordering::fields(
                &nested[1..],
                crate::task::ordering::TaskDocumentKind::Index,
            );
        }
        if nested.len() >= 2 && nested[0] == "task_items" {
            return crate::task::ordering::fields(
                &nested[2..],
                crate::task::ordering::TaskDocumentKind::Item,
            );
        }
    }
    if path
        == [
            "metadata",
            "request",
            "migration",
            "candidates",
            "content",
            "changes",
            "edits",
        ]
    {
        return &[
            "artifact",
            "operation",
            "path",
            "task_id",
            "before",
            "after",
        ];
    }
    if let (Some(schema), Some(position)) =
        (document, path.iter().position(|part| part == "content"))
    {
        let nested = &path[position + 1..];
        return match schema {
            "work-task-index" => crate::task::ordering::fields(
                nested,
                crate::task::ordering::TaskDocumentKind::Index,
            ),
            "work-task-item" => {
                crate::task::ordering::fields(nested, crate::task::ordering::TaskDocumentKind::Item)
            }
            "work-execution-index" => crate::execution::index::order(nested),
            _ => &[],
        };
    }
    if path == ["metadata", "request"] {
        return &[
            "migration",
            "preview_fingerprint",
            "reconciliation_fingerprint",
            "attempt_path",
        ];
    }
    if path == ["metadata", "request", "migration"] {
        return &["schema", "sources", "candidates", "semantic_decisions"];
    }
    if path == ["metadata", "request", "migration", "sources"] {
        return &["path", "raw_sha256"];
    }
    if path == ["metadata", "request", "migration", "candidates"] {
        return &["path", "kind", "task_id", "content"];
    }
    if path == ["metadata", "artifacts"] {
        return &["source", "task", "execution"];
    }
    match path.last().map(String::as_str) {
        None => &[
            "schema",
            "transaction_id",
            "approval_sha256",
            "state",
            "published_count",
            "metadata",
            "files",
        ],
        Some("metadata") => &[
            "request",
            "artifacts",
            "affected_task_ids",
            "history_sha256",
            "source_sha256",
            "candidate_sha256",
        ],
        Some("files") => &["phase", "path", "operation", "before", "after"],
        Some("before" | "after") => &["raw_sha256", "base64"],
        _ => &[],
    }
}

impl Serialize for Ordered<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(object) = self.value.as_object() {
            let priority = if self.path == ["metadata", "request"]
                && matches!(
                    self.value["kind"].as_str(),
                    Some("instruction_migration" | "source_refresh")
                ) {
                &["kind", "requirement_id", "preview_fingerprint"][..]
            } else {
                order(&self.path, self.document.as_deref())
            };
            let mut output = serializer.serialize_map(Some(object.len()))?;
            let mut keys = priority
                .iter()
                .copied()
                .chain(
                    object
                        .keys()
                        .filter(|key| !priority.contains(&key.as_str()))
                        .map(String::as_str),
                )
                .collect::<Vec<_>>();
            if self.document.as_deref() == Some("work-spec-update-request")
                && matches!(self.path.as_slice(), [metadata, hashes] if metadata == "metadata" && (hashes == "source_sha256" || hashes == "candidate_sha256"))
            {
                keys.sort_by_key(|key| {
                    let rank = if key.contains("/plans/") {
                        0
                    } else if key.contains("/tasks/") && key.ends_with("/index.json") {
                        1
                    } else if key.contains("/executions/") {
                        2
                    } else {
                        3
                    };
                    (rank, *key)
                });
            }
            for key in keys {
                if let Some(value) = object.get(key) {
                    let mut path = self.path.clone();
                    path.push(key.into());
                    output.serialize_entry(
                        key,
                        &Ordered {
                            value,
                            path,
                            document: if key == "content"
                                || (key == "request" && self.path == ["metadata"])
                            {
                                value["schema"].as_str().map(str::to_owned)
                            } else {
                                self.document.clone()
                            },
                        },
                    )?;
                }
            }
            output.end()
        } else if let Some(array) = self.value.as_array() {
            let mut output = serializer.serialize_seq(Some(array.len()))?;
            for value in array {
                output.serialize_element(&Ordered {
                    value,
                    path: self.path.clone(),
                    document: self.document.clone(),
                })?;
            }
            output.end()
        } else {
            self.value.serialize(serializer)
        }
    }
}

pub fn render_transaction(value: &Value) -> Result<Vec<u8>, TransactionIssue> {
    validate_transaction(value)?;
    let mut bytes = serde_json::to_vec_pretty(&Ordered {
        value,
        path: Vec::new(),
        document: value["metadata"]["request"]["schema"]
            .as_str()
            .map(str::to_owned),
    })
    .expect("JSON value serializes");
    bytes.push(b'\n');
    Ok(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionState {
    Incomplete,
    Completed,
    Corrupt,
}

pub fn completion_state(record_raw: &[u8], marker_raw: Option<&[u8]>) -> CompletionState {
    match marker_raw {
        None => CompletionState::Incomplete,
        Some(marker) if marker == completion_marker(record_raw) => CompletionState::Completed,
        Some(_) => CompletionState::Corrupt,
    }
}

pub fn verified_retained_journal_kind<'a>(
    execution: &str,
    relative: &str,
    journal: &'a Value,
) -> Result<crate::derivation::publication::JournalKind<'a>, TransactionIssue> {
    use crate::derivation::publication::{self, JournalKind, RuntimeOperation as O};
    validate_transaction(journal)?;
    let address = publication::parse_retained_journal_path(execution, relative)?;
    if journal["metadata"]["artifacts"]
        .get("execution")
        .is_some_and(|path| path != execution)
    {
        return Err(issue(
            "journal_layout_metadata",
            "The journal execution path differs from its verified scope.",
        ));
    }
    let request = &journal["metadata"]["request"];
    let transaction_id = journal["transaction_id"]
        .as_str()
        .expect("validated identity");
    let hash = |field: &str| {
        request[field]
            .as_str()
            .filter(|sha| valid_sha256(sha))
            .ok_or_else(|| {
                issue(
                    "journal_layout_metadata",
                    "Complete approved journal identity evidence is required.",
                )
            })
    };
    let kind = match address.operation {
        O::SpecificationUpdate => {
            if !transaction_id.starts_with("SPEC-UPDATE-")
                || request["schema"] != "work-spec-update-request"
                || request["task_index"]["requirement_id"]
                    .as_str()
                    .is_none_or(|id| id.parse::<crate::identifiers::RequirementId>().is_err())
            {
                return Err(issue(
                    "journal_layout_metadata",
                    "The original Specification update identity is required.",
                ));
            }
            JournalKind::SpecificationUpdate(transaction_id)
        }
        O::SpecificationMigration => {
            if request.get("migration").is_some() {
                if !transaction_id.starts_with("SPEC-MIGRATION-") {
                    return Err(issue(
                        "journal_layout_metadata",
                        "The migration journal kind is invalid.",
                    ));
                }
                strict(request, &["migration", "preview_fingerprint"], &[])?;
                JournalKind::SpecificationMigration(hash("preview_fingerprint")?)
            } else {
                if !transaction_id.starts_with("SPEC-RECONCILIATION-") {
                    return Err(issue(
                        "journal_layout_metadata",
                        "The reconciliation journal kind is invalid.",
                    ));
                }
                strict(
                    request,
                    &["reconciliation_fingerprint", "attempt_path"],
                    &[],
                )?;
                let numbered = |id: &str, prefix: &str| {
                    id.strip_prefix(prefix).is_some_and(|digits| {
                        digits.len() == 3
                            && digits != "000"
                            && digits.bytes().all(|b| b.is_ascii_digit())
                    })
                };
                let in_scope = request["attempt_path"].as_str().and_then(|path| path.strip_prefix(&format!("{execution}/")))
                    .is_some_and(|path| matches!(path.split('/').collect::<Vec<_>>().as_slice(), [task, attempt, "attempt.json"] if numbered(task, "TASK-") && numbered(attempt, "ATTEMPT-")));
                if !in_scope {
                    return Err(issue(
                        "journal_layout_metadata",
                        "The reconciliation Attempt is outside the verified execution scope.",
                    ));
                }
                JournalKind::SpecificationMigration(hash("reconciliation_fingerprint")?)
            }
        }
        O::SpecificationMigrationItem => {
            if !transaction_id.starts_with("SPEC-MIGRATION-")
                || journal["metadata"]["artifacts"]["execution"] != execution
            {
                return Err(issue(
                    "journal_layout_metadata",
                    "Migration item journals require the approved execution scope.",
                ));
            }
            strict(
                request,
                &["request_sha256", "analysis_fingerprint", "item_id"],
                &[],
            )?;
            hash("analysis_fingerprint")?;
            if request["item_id"].as_str().is_none_or(|id| id.is_empty()) {
                return Err(issue(
                    "journal_layout_metadata",
                    "The original migration item identity is required.",
                ));
            }
            JournalKind::SpecificationMigrationItem {
                approved: hash("request_sha256")?,
                position: address.item_position.expect("parsed item position"),
            }
        }
        O::SpecificationMigrationReconcile => {
            if !transaction_id.starts_with("SPEC-RECONCILIATION-") {
                return Err(issue(
                    "journal_layout_metadata",
                    "The reconciliation journal kind is invalid.",
                ));
            }
            strict(request, &["request_sha256", "phase"], &[])?;
            if request["phase"] != "reconciliation" {
                return Err(issue(
                    "journal_layout_metadata",
                    "The original migration reconciliation identity is required.",
                ));
            }
            JournalKind::SpecificationMigrationReconcile(hash("request_sha256")?)
        }
        O::InstructionMigration | O::SourceRefresh => {
            strict(
                request,
                &["kind", "requirement_id", "preview_fingerprint"],
                &[],
            )?;
            let expected_kind = if address.operation == O::InstructionMigration {
                "instruction_migration"
            } else {
                "source_refresh"
            };
            if request["kind"] != expected_kind
                || request["requirement_id"]
                    .as_str()
                    .is_none_or(|id| id.parse::<crate::identifiers::RequirementId>().is_err())
            {
                return Err(issue(
                    "journal_layout_metadata",
                    "The original recover-only journal kind and requirement are required.",
                ));
            }
            let transaction_kind = if address.operation == O::InstructionMigration {
                crate::derivation::identity::PreviewTransactionKind::InstructionMigration
            } else {
                crate::derivation::identity::PreviewTransactionKind::SourceRefresh
            };
            if crate::derivation::identity::preview_transaction_id(
                transaction_kind,
                hash("preview_fingerprint")?,
            )? != transaction_id
            {
                return Err(issue(
                    "journal_layout_metadata",
                    "The recover-only journal ID differs from its full original preview proof.",
                ));
            }
            if address.operation == O::InstructionMigration {
                JournalKind::InstructionMigration(hash("preview_fingerprint")?)
            } else {
                JournalKind::SourceRefresh(hash("preview_fingerprint")?)
            }
        }
        _ => {
            return Err(issue(
                "journal_layout_metadata",
                "Only retained journal operations are valid here.",
            ));
        }
    };
    publication::bind_retained_journal_path(execution, relative, kind)?;
    Ok(kind)
}

pub fn verify_retained_journal_layout(
    execution: &str,
    relative: &str,
    journal: &Value,
) -> Result<crate::derivation::publication::RetainedJournalAddress, TransactionIssue> {
    let kind = verified_retained_journal_kind(execution, relative, journal)?;
    crate::derivation::publication::bind_retained_journal_path(execution, relative, kind)
}

/// Only canonical progress fields may differ from the complete frozen journal.
pub fn verify_retained_journal_progress(
    prepared: &Value,
    actual_raw: &[u8],
) -> Result<Value, TransactionIssue> {
    validate_transaction(prepared)?;
    if prepared["state"] != "prepared" || prepared["published_count"] != 0 {
        return Err(issue(
            "journal_preparation_invalid",
            "Original prepared journal evidence is required.",
        ));
    }
    let actual: Value = serde_json::from_slice(actual_raw).map_err(|_| {
        issue(
            "journal_progress_invalid",
            "Canonical journal progress is required.",
        )
    })?;
    if render_transaction(&actual)? != actual_raw {
        return Err(issue(
            "journal_progress_noncanonical",
            "Canonical journal progress is required.",
        ));
    }
    let mut expected = prepared.clone();
    expected["state"] = actual["state"].clone();
    expected["published_count"] = actual["published_count"].clone();
    if actual != expected {
        return Err(issue(
            "journal_progress_identity",
            "Journal progress changed immutable approved evidence.",
        ));
    }
    Ok(actual)
}

pub fn verify_retained_journal_commit(
    prepared: &Value,
    actual_raw: &[u8],
    marker_raw: Option<&[u8]>,
) -> Result<Value, TransactionIssue> {
    let actual = verify_retained_journal_progress(prepared, actual_raw)?;
    if actual["state"] != "published"
        || completion_state(actual_raw, marker_raw) != CompletionState::Completed
    {
        return Err(issue(
            "journal_commit_evidence",
            "Published journal and exact committed marker evidence are required.",
        ));
    }
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_metadata_binding_covers_six_kinds_and_ledger_only_without_short_hash_proof() {
        use crate::derivation::{
            publication::{self, JournalKind},
            transaction::*,
        };
        let execution = "custom execution/例";
        let approved = "ab".repeat(32);
        let requests = [
            json!({"schema":"work-spec-update-request","task_index":{"requirement_id":"example"}}),
            json!({"migration":{},"preview_fingerprint":approved}),
            json!({"request_sha256":approved,"analysis_fingerprint":"c".repeat(64),"item_id":"ITEM-003"}),
            json!({"request_sha256":approved,"phase":"reconciliation"}),
            json!({"kind":"instruction_migration","requirement_id":"example","preview_fingerprint":approved}),
            json!({"kind":"source_refresh","requirement_id":"example","preview_fingerprint":approved}),
            json!({"reconciliation_fingerprint":approved,"attempt_path":format!("{execution}/TASK-001/ATTEMPT-001/attempt.json")}),
        ];
        for (position, request) in requests.into_iter().enumerate() {
            let transaction_kind = match position {
                1 | 2 => TransactionKind::Migration,
                3 | 6 => TransactionKind::Reconciliation,
                4 => TransactionKind::InstructionMigration,
                5 => TransactionKind::SourceRefresh,
                _ => TransactionKind::Update,
            };
            let journal = TransactionDeriver::derive(TransactionInput {
                kind: transaction_kind,
                order: PublicationOrder::Flat,
                request,
                artifacts: if position == 6 {
                    json!({})
                } else {
                    json!({"execution":execution})
                },
                affected_task_ids: vec![],
                history: std::collections::BTreeMap::new(),
                source: std::collections::BTreeMap::new(),
                candidate: std::collections::BTreeMap::from([(
                    "formal.json".into(),
                    b"new".to_vec(),
                )]),
            })
            .unwrap()
            .journal;
            let kind = match position {
                0 => JournalKind::SpecificationUpdate(journal["transaction_id"].as_str().unwrap()),
                1 | 6 => JournalKind::SpecificationMigration(&approved),
                2 => JournalKind::SpecificationMigrationItem {
                    approved: &approved,
                    position: 2,
                },
                3 => JournalKind::SpecificationMigrationReconcile(&approved),
                4 => JournalKind::InstructionMigration(&approved),
                5 => JournalKind::SourceRefresh(&approved),
                _ => unreachable!(),
            };
            let path = publication::retained_journal_path(execution, kind).unwrap();
            let before = render_transaction(&journal).unwrap();
            verify_retained_journal_layout(execution, &path, &journal).unwrap();
            assert!(verify_retained_journal_layout("foreign", &path, &journal).is_err());
            let copied = path.replace(execution, "foreign");
            assert!(
                verify_retained_journal_layout("foreign", &copied, &journal).is_err(),
                "position={position}"
            );
            assert_eq!(render_transaction(&journal).unwrap(), before);
            if position != 0 {
                let field = if position == 2 || position == 3 {
                    "request_sha256"
                } else if position == 6 {
                    "reconciliation_fingerprint"
                } else {
                    "preview_fingerprint"
                };
                let mut missing = journal.clone();
                missing["metadata"]["request"]
                    .as_object_mut()
                    .unwrap()
                    .remove(field);
                assert!(verify_retained_journal_layout(execution, &path, &missing).is_err());
            }
        }
    }

    #[test]
    fn retained_commit_requires_full_original_identity_canonical_progress_and_exact_marker() {
        use crate::derivation::transaction::{
            PublicationOrder, TransactionDeriver, TransactionInput, TransactionKind,
        };
        let prepared = TransactionDeriver::derive(TransactionInput {
            kind: TransactionKind::Update,
            order: PublicationOrder::Flat,
            request: json!({}),
            artifacts: json!({}),
            affected_task_ids: vec![],
            history: std::collections::BTreeMap::new(),
            source: std::collections::BTreeMap::new(),
            candidate: std::collections::BTreeMap::from([
                ("task/one.json".into(), b"one".to_vec()),
                ("task/two.json".into(), b"two".to_vec()),
            ]),
        })
        .unwrap()
        .journal;
        for (count, state) in [(0, "prepared"), (1, "publishing"), (2, "published")] {
            let mut actual = prepared.clone();
            actual["state"] = json!(state);
            actual["published_count"] = json!(count);
            let raw = render_transaction(&actual).unwrap();
            assert_eq!(
                verify_retained_journal_progress(&prepared, &raw).unwrap(),
                actual
            );
            let marker = completion_marker(&raw);
            assert_eq!(
                verify_retained_journal_commit(&prepared, &raw, Some(&marker)).is_ok(),
                count == 2
            );
            for missing in [
                None,
                Some(&b""[..]),
                Some(&marker[..20]),
                Some(&b"foreign marker\n"[..]),
            ] {
                assert!(verify_retained_journal_commit(&prepared, &raw, missing).is_err());
            }
            let mut noncanonical = raw.clone();
            noncanonical.push(b'\n');
            assert!(verify_retained_journal_progress(&prepared, &noncanonical).is_err());
            let mut foreign = actual.clone();
            foreign["metadata"]["request"] = json!({"foreign":true});
            let approved = approval_sha256(&foreign["files"], &foreign["metadata"]);
            foreign["approval_sha256"] = json!(approved);
            foreign["transaction_id"] = json!(derived_transaction_id("UPDATE", &approved).unwrap());
            assert!(
                verify_retained_journal_progress(&prepared, &render_transaction(&foreign).unwrap())
                    .is_err()
            );
        }
    }

    #[test]
    fn shared_journal_identity_snapshot_and_progress_cases_match_current_contract() {
        let metadata = json!({"request":{"schema":"test/v1"},"artifacts":{},
            "affected_task_ids":["TASK-001"],"history_sha256":{},
            "source_sha256":{},"candidate_sha256":{}});
        let files = json!([
            {"phase":20,"path":"tasks/TASK-001.json","operation":"replace",
                "before":encode_snapshot(b"old"),"after":encode_snapshot(b"new")},
            {"phase":20,"path":"tasks/TASK-002.json","operation":"add",
                "after":encode_snapshot(b"added")},
        ]);
        let approval = approval_sha256(&files, &metadata);
        let id = derived_transaction_id("UPDATE", &approval).unwrap();
        assert_eq!(id, derived_transaction_id("UPDATE", &approval).unwrap());
        let value = json!({"schema":"work-spec-transaction","transaction_id":id,
            "approval_sha256":approval,"state":"prepared","published_count":0,
            "metadata":metadata,"files":files});
        validate_transaction(&value).unwrap();
        let raw = render_transaction(&value).unwrap();
        assert!(raw.ends_with(b"\n"));
        let parsed: Value = serde_json::from_slice(&raw).unwrap();
        validate_transaction(&parsed).unwrap();
        assert_eq!(parsed["published_count"], 0);
        let mut changed = value.clone();
        changed["metadata"]["request"] = json!({"schema":"different/v1"});
        assert_eq!(
            validate_transaction(&changed).unwrap_err().reason_code,
            "invalid_contract_value"
        );
        let changed_approval = approval_sha256(&changed["files"], &changed["metadata"]);
        assert_ne!(
            value["transaction_id"],
            derived_transaction_id("UPDATE", &changed_approval).unwrap()
        );
        changed["approval_sha256"] = json!(changed_approval);
        changed["transaction_id"] =
            json!(derived_transaction_id("UPDATE", &changed_approval).unwrap());
        validate_transaction(&changed).unwrap();
        let mut caller_id = value.clone();
        caller_id["transaction_id"] = json!("SPEC-UPDATE-002");
        assert!(render_transaction(&caller_id).is_err());
        for id in [
            "TASK-REPAIR-ABCDEF012345",
            "SOURCE-REFRESH-ABCDEF012345",
            "INSTRUCTION-MIGRATION-ABCDEF012345",
        ] {
            let mut shared = value.clone();
            shared["transaction_id"] = json!(id);
            validate_transaction(&shared).unwrap();
            render_transaction(&shared).unwrap();
        }
        let mut invalid = value.clone();
        invalid["files"][0]["before"]["base64"] = json!("bmV3");
        assert!(render_transaction(&invalid).is_err());
        let mut invalid = value.clone();
        invalid["approval_sha256"] = json!("0".repeat(64));
        assert!(render_transaction(&invalid).is_err());
        let mut invalid = value.clone();
        invalid["files"].as_array_mut().unwrap().reverse();
        invalid["approval_sha256"] =
            json!(approval_sha256(&invalid["files"], &invalid["metadata"]));
        assert!(render_transaction(&invalid).is_err());
        let mut invalid = value.clone();
        invalid["published_count"] = json!(1);
        assert!(render_transaction(&invalid).is_err());
        let mut invalid = value;
        invalid["schema"] = json!("work-spec-transaction/v2");
        assert!(render_transaction(&invalid).is_err());
    }

    #[test]
    fn current_contract_transaction_example_matches_bytes_and_progress() {
        let metadata = json!({"request":{"schema":"example/v1"},"artifacts":{},"affected_task_ids":["TASK-001"],"history_sha256":{},"source_sha256":{},"candidate_sha256":{}});
        let files = json!([{"phase":10,"path":"example.json","operation":"add","after":encode_snapshot(b"{}") }]);
        let approval = approval_sha256(&files, &metadata);
        assert_eq!(
            approval,
            "655c3708cea35b6203cbeb668548c7f85921daf87dccb47d7abe3420c204a798"
        );
        let value = json!({"schema":"work-spec-transaction","transaction_id":derived_transaction_id("UPDATE", &approval).unwrap(),"approval_sha256":approval,"state":"prepared","published_count":0,"metadata":metadata,"files":files});
        let raw = render_transaction(&value).unwrap();
        assert_eq!(raw.len(), 693);
        assert_eq!(
            sha256_hex(&raw),
            "5b335245063ed28f9fae63429818eb6b73b3fbcb772d0f88fb3b681db2d01ca5"
        );
        let mut incomplete = value;
        incomplete["published_count"] = json!(1);
        assert_eq!(
            validate_transaction(&incomplete).unwrap_err().reason_code,
            "invalid_contract_value"
        );
    }
}
