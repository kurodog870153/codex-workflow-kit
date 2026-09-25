//! Specification transaction evidence, progress and canonical journal rules.

use std::collections::BTreeSet;

use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use serde_json::{Value, json};

use crate::canonical::{canonical_json_sha256, sha256_hex};
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

fn base64_encode(raw: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(raw.len().div_ceil(3) * 4);
    for chunk in raw.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        result.push(ALPHABET[(first >> 2) as usize] as char);
        result.push(ALPHABET[(((first & 3) << 4) | (second >> 4)) as usize] as char);
        result.push(if chunk.len() > 1 {
            ALPHABET[(((second & 15) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            ALPHABET[(third & 63) as usize] as char
        } else {
            '='
        });
    }
    result
}

fn base64_decode(value: &str) -> Option<Vec<u8>> {
    if value.len() % 4 != 0 || !value.is_ascii() {
        return None;
    }
    let mut result = Vec::with_capacity(value.len() / 4 * 3);
    for (index, chunk) in value.as_bytes().chunks_exact(4).enumerate() {
        let last = index + 1 == value.len() / 4;
        let digit = |byte: u8| -> Option<u8> {
            match byte {
                b'A'..=b'Z' => Some(byte - b'A'),
                b'a'..=b'z' => Some(byte - b'a' + 26),
                b'0'..=b'9' => Some(byte - b'0' + 52),
                b'+' => Some(62),
                b'/' => Some(63),
                _ => None,
            }
        };
        let a = digit(chunk[0])?;
        let b = digit(chunk[1])?;
        let c = if chunk[2] == b'=' && last {
            0
        } else {
            digit(chunk[2])?
        };
        let d = if chunk[3] == b'=' && last {
            0
        } else {
            digit(chunk[3])?
        };
        if chunk[2] == b'=' && chunk[3] != b'=' {
            return None;
        }
        result.push((a << 2) | (b >> 4));
        if chunk[2] != b'=' {
            result.push((b << 4) | (c >> 2));
        }
        if chunk[3] != b'=' {
            result.push((c << 6) | d);
        }
    }
    (base64_encode(&result) == value).then_some(result)
}

pub fn encode_snapshot(raw: &[u8]) -> Value {
    json!({"raw_sha256": sha256_hex(raw), "base64": base64_encode(raw)})
}

pub fn decode_snapshot(value: &Value) -> Result<Vec<u8>, TransactionIssue> {
    strict(value, &["raw_sha256", "base64"], &[])?;
    let raw = base64_decode(value["base64"].as_str().ok_or_else(|| {
        issue(
            "invalid_contract_value",
            "Transaction bytes must use canonical base64.",
        )
    })?)
    .ok_or_else(|| {
        issue(
            "invalid_contract_value",
            "Transaction bytes must use canonical base64.",
        )
    })?;
    if !sha(&value["raw_sha256"]) || value["raw_sha256"] != sha256_hex(&raw) {
        return Err(issue(
            "invalid_contract_value",
            "Transaction bytes do not match their fingerprint.",
        ));
    }
    Ok(raw)
}

pub fn approval_sha256(files: &Value, metadata: &Value) -> String {
    canonical_json_sha256(&json!({"files":files,"metadata":metadata}))
        .expect("JSON value serializes")
}

pub fn derived_transaction_id(kind: &str, approval: &str) -> Result<String, TransactionIssue> {
    if !matches!(kind, "UPDATE" | "MIGRATION" | "RECONCILIATION") || !sha(&json!(approval)) {
        return Err(issue(
            "spec_transaction_identity",
            "A validated transaction kind and approval fingerprint are required.",
        ));
    }
    Ok(format!(
        "SPEC-{kind}-{}",
        approval[..12].to_ascii_uppercase()
    ))
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
    if value["schema"] != "work-spec-transaction/v1" || !sha(&value["approval_sha256"]) {
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
    if document == Some("work-task-repair-request/v1")
        && path.starts_with(&["metadata".to_owned(), "request".to_owned()])
    {
        let nested = &path[2..];
        if nested.is_empty() {
            return &[
                "schema",
                "stage",
                "requirement_id",
                "artifacts",
                "expected",
                "decisions",
                "task_index",
                "task_items",
            ];
        }
        if nested == ["artifacts"] {
            return &["plan", "task", "execution"];
        }
        if nested == ["decisions"] {
            return &["location", "decision"];
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
    if document == Some("work-spec-update-request/v1")
        && path.starts_with(&["metadata".to_owned(), "request".to_owned()])
    {
        let nested = &path[2..];
        if nested.is_empty() {
            return &[
                "schema",
                "reason",
                "expected",
                "plan",
                "task_index",
                "task_items",
            ];
        }
        if nested == ["expected"] {
            return &[
                "plan_sha256",
                "task_index_sha256",
                "execution_index_sha256",
                "task_item_sha256",
            ];
        }
        if nested[0] == "plan" {
            return crate::plan::fields(&nested[1..]);
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
            "work-plan/v1" => crate::plan::fields(nested),
            "work-task-index/v1" => crate::task::ordering::fields(
                nested,
                crate::task::ordering::TaskDocumentKind::Index,
            ),
            "work-task-item/v1" => {
                crate::task::ordering::fields(nested, crate::task::ordering::TaskDocumentKind::Item)
            }
            "work-execution-index/v1" => crate::execution::index::order(nested),
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
        return &["plan", "task", "execution"];
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
            if self.document.as_deref() == Some("work-spec-update-request/v1")
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
            } else if self.document.as_deref() == Some("work-task-repair-request/v1")
                && matches!(self.path.as_slice(), [metadata, hashes] if metadata == "metadata" && (hashes == "source_sha256" || hashes == "candidate_sha256"))
            {
                let source = self.path[1] == "source_sha256";
                keys.sort_by_key(|key| {
                    let rank = if key.contains("/tasks/") && !key.ends_with("/index.json") {
                        if source { 0 } else { 1 }
                    } else if key.contains("/tasks/") {
                        if source { 1 } else { 0 }
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

pub fn completion_marker(record_raw: &[u8]) -> Vec<u8> {
    format!("{}\n", sha256_hex(record_raw)).into_bytes()
}

pub fn completion_state(record_raw: &[u8], marker_raw: Option<&[u8]>) -> CompletionState {
    match marker_raw {
        None => CompletionState::Incomplete,
        Some(marker) if marker == completion_marker(record_raw) => CompletionState::Completed,
        Some(_) => CompletionState::Corrupt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_journal_identity_snapshot_and_progress_cases_match_python() {
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
        let value = json!({"schema":"work-spec-transaction/v1","transaction_id":id,
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
    fn python_transaction_example_matches_bytes_and_progress() {
        let metadata = json!({"request":{"schema":"example/v1"},"artifacts":{},"affected_task_ids":["TASK-001"],"history_sha256":{},"source_sha256":{},"candidate_sha256":{}});
        let files = json!([{"phase":10,"path":"example.json","operation":"add","after":encode_snapshot(b"{}") }]);
        let approval = approval_sha256(&files, &metadata);
        assert_eq!(
            approval,
            "655c3708cea35b6203cbeb668548c7f85921daf87dccb47d7abe3420c204a798"
        );
        let value = json!({"schema":"work-spec-transaction/v1","transaction_id":derived_transaction_id("UPDATE", &approval).unwrap(),"approval_sha256":approval,"state":"prepared","published_count":0,"metadata":metadata,"files":files});
        let raw = render_transaction(&value).unwrap();
        assert_eq!(raw.len(), 696);
        assert_eq!(
            sha256_hex(&raw),
            "963388f30a045a484f2807bfeebfbea5920c18159cb03706b217ea5468d835e1"
        );
        let mut incomplete = value;
        incomplete["published_count"] = json!(1);
        assert_eq!(
            validate_transaction(&incomplete).unwrap_err().reason_code,
            "invalid_contract_value"
        );
    }
}
