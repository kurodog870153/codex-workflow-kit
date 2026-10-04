//! Discussion-only progress contract and byte ordering.

use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use serde_json::{Value, json};

use crate::canonical::{parse_json_contract, sha256_hex};
use crate::identifiers::RequirementId;
use crate::progress_ordered::OrderedValue;
use crate::protocol::TASK_ID_PREFIX;

const FIELDS: [&str; 15] = [
    "schema",
    "requirement_id",
    "mode",
    "revision",
    "status",
    "title",
    "request",
    "current_task_id",
    "context",
    "source_status",
    "notes",
    "confirmed_decisions",
    "tentative",
    "open_questions",
    "next_discussion_point",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressIssue {
    pub reason_code: &'static str,
    pub message: &'static str,
    pub details: Value,
}

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> ProgressIssue {
    ProgressIssue {
        reason_code,
        message,
        details,
    }
}

fn nonempty(value: &Value, location: &str) -> Result<(), ProgressIssue> {
    if value.as_str().is_some_and(|text| !text.trim().is_empty()) {
        Ok(())
    } else {
        Err(issue(
            "empty_text_value",
            "A non-empty string is required.",
            json!({"location":location}),
        ))
    }
}

pub fn validate_progress(value: &Value) -> Result<(), ProgressIssue> {
    let object = value
        .as_object()
        .filter(|object| {
            object.len() == FIELDS.len() && FIELDS.iter().all(|field| object.contains_key(*field))
        })
        .ok_or_else(|| {
            issue(
                "invalid_object_fields",
                "The JSON object has missing or unknown fields.",
                json!({"location":"progress"}),
            )
        })?;
    if object["schema"] != "work-discussion-progress" || object["status"] != "discussion_only" {
        return Err(issue(
            "invalid_progress_schema",
            "Progress must be discussion-only memory.",
            json!({}),
        ));
    }
    nonempty(&object["requirement_id"], "requirement_id")?;
    let requirement_id = object["requirement_id"].as_str().expect("checked string");
    requirement_id.parse::<RequirementId>().map_err(|failure| {
        issue(
            failure.reason_code(),
            "The requirement ID is invalid.",
            json!({"requirement_id":requirement_id}),
        )
    })?;
    if object["mode"] != "task" {
        return Err(issue(
            "invalid_progress_mode",
            "Only Task discussions can be saved.",
            json!({}),
        ));
    }
    if object["revision"]
        .as_u64()
        .filter(|revision| *revision > 0)
        .is_none()
    {
        return Err(issue(
            "invalid_progress_revision",
            "A positive integer revision is required.",
            json!({}),
        ));
    }
    if !object["current_task_id"].is_null() {
        let valid = object["mode"] == "task"
            && object["current_task_id"].as_str().is_some_and(|id| {
                id.len() == 8
                    && id.starts_with(TASK_ID_PREFIX)
                    && id[5..].bytes().all(|byte| byte.is_ascii_digit())
            });
        if !valid {
            return Err(issue(
                "invalid_progress_task",
                "Only Task progress may identify a current TASK-NNN.",
                json!({}),
            ));
        }
    }
    for field in ["title", "request", "next_discussion_point"] {
        nonempty(&object[field], field)?;
    }
    if !object["context"].is_object() {
        return Err(issue(
            "invalid_progress_context",
            "Supplied context must be a JSON object.",
            json!({}),
        ));
    }
    for field in [
        "source_status",
        "notes",
        "tentative",
        "open_questions",
        "confirmed_decisions",
    ] {
        let rows = object[field].as_array().ok_or_else(|| {
            issue(
                "invalid_progress_list",
                "Discussion entries must be arrays.",
                json!({"field":field}),
            )
        })?;
        for (index, row) in rows.iter().enumerate() {
            let location = format!("{field}[{index}]");
            if field == "confirmed_decisions" {
                let decision = row
                    .as_object()
                    .filter(|decision| {
                        decision.contains_key("statement")
                            && decision
                                .keys()
                                .all(|key| key == "statement" || key == "rationale")
                    })
                    .ok_or_else(|| {
                        issue(
                            "invalid_object_fields",
                            "The JSON object has missing or unknown fields.",
                            json!({"location":location}),
                        )
                    })?;
                nonempty(&decision["statement"], &format!("{location}.statement"))?;
                if let Some(rationale) = decision.get("rationale") {
                    nonempty(rationale, &format!("{location}.rationale"))?;
                }
            } else {
                nonempty(row, &location)?;
            }
        }
    }
    let _: work_model::progress::DiscussionProgress =
        serde_json::from_value(value.clone()).expect("validated progress matches its model");
    Ok(())
}

struct Ordered<'a> {
    value: &'a Value,
    section: &'a str,
    context: Option<&'a OrderedValue>,
}

impl Serialize for Ordered<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(object) = self.value.as_object() {
            let fields: &[&str] = if self.section == "root" {
                &FIELDS
            } else if self.section == "decision" {
                &["statement", "rationale"]
            } else {
                &[]
            };
            let mut output = serializer.serialize_map(Some(object.len()))?;
            for key in fields.iter().copied().chain(
                object
                    .keys()
                    .filter(|key| !fields.contains(&key.as_str()))
                    .map(String::as_str),
            ) {
                if let Some(value) = object.get(key) {
                    if self.section == "root" && key == "context" {
                        if let Some(context) = self.context {
                            output.serialize_entry(key, context)?;
                            continue;
                        }
                    }
                    let section = if self.section == "root" && key == "confirmed_decisions" {
                        "decisions"
                    } else if self.section == "decisions" {
                        "decision"
                    } else {
                        "nested"
                    };
                    output.serialize_entry(
                        key,
                        &Ordered {
                            value,
                            section,
                            context: self.context,
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
                    section: if self.section == "decisions" {
                        "decision"
                    } else {
                        self.section
                    },
                    context: self.context,
                })?;
            }
            output.end()
        } else {
            self.value.serialize(serializer)
        }
    }
}

pub fn render_progress(value: &Value) -> Result<Vec<u8>, ProgressIssue> {
    render_with_context(value, None)
}

fn render_with_context(
    value: &Value,
    context: Option<&OrderedValue>,
) -> Result<Vec<u8>, ProgressIssue> {
    validate_progress(value)?;
    let mut raw = serde_json::to_vec_pretty(&Ordered {
        value,
        section: "root",
        context,
    })
    .expect("JSON value serializes");
    raw.push(b'\n');
    Ok(raw)
}

struct SaveBinding<'a> {
    path: &'a str,
    expected_revision: u64,
    previous_sha256: Option<&'a str>,
    progress: &'a Value,
    context: Option<&'a OrderedValue>,
}

impl Serialize for SaveBinding<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut output = serializer.serialize_map(Some(5))?;
        output.serialize_entry("schema", "work-progress-save-request")?;
        output.serialize_entry("path", self.path)?;
        output.serialize_entry("expected_revision", &self.expected_revision)?;
        output.serialize_entry("previous_sha256", &self.previous_sha256)?;
        output.serialize_entry(
            "progress",
            &Ordered {
                value: self.progress,
                section: "root",
                context: self.context,
            },
        )?;
        output.end()
    }
}

pub fn approval_sha256(
    path: &str,
    expected_revision: u64,
    previous_sha256: Option<&str>,
    progress: &Value,
) -> Result<String, ProgressIssue> {
    approval_with_context(path, expected_revision, previous_sha256, progress, None)
}

fn approval_with_context(
    path: &str,
    expected_revision: u64,
    previous_sha256: Option<&str>,
    progress: &Value,
    context: Option<&OrderedValue>,
) -> Result<String, ProgressIssue> {
    validate_progress(progress)?;
    let binding = SaveBinding {
        path,
        expected_revision,
        previous_sha256,
        progress,
        context,
    };
    let _: work_model::progress::ProgressSaveRequest =
        serde_json::from_value(serde_json::to_value(&binding).expect("binding serializes"))
            .expect("save binding matches its model");
    let mut raw = serde_json::to_vec_pretty(&SaveBinding {
        path,
        expected_revision,
        previous_sha256,
        progress,
        context,
    })
    .expect("JSON value serializes");
    raw.push(b'\n');
    Ok(sha256_hex(&raw))
}

#[derive(Debug, Clone)]
pub struct ProgressDocument {
    pub value: Value,
    pub canonical_raw: Vec<u8>,
    context: OrderedValue,
}

impl ProgressDocument {
    pub fn parse(raw: &[u8]) -> Result<Self, ProgressIssue> {
        let value = parse_json_contract(raw).map_err(|_| {
            issue(
                "invalid_json",
                "The progress input is not valid JSON.",
                json!({}),
            )
        })?;
        Self::from_value_with_context_raw(value, raw)
    }

    pub fn from_value_with_context_raw(
        value: Value,
        context_source: &[u8],
    ) -> Result<Self, ProgressIssue> {
        validate_progress(&value)?;
        let ordered: OrderedValue = serde_json::from_slice(context_source).map_err(|_| {
            issue(
                "invalid_json",
                "The progress input is not valid JSON.",
                json!({}),
            )
        })?;
        let context = ordered
            .get("context")
            .ok_or_else(|| {
                issue(
                    "invalid_progress_context",
                    "Supplied context must be a JSON object.",
                    json!({}),
                )
            })?
            .clone();
        if serde_json::to_value(&context).expect("JSON value serializes") != value["context"] {
            return Err(issue(
                "invalid_progress_context",
                "Supplied context must match the progress candidate.",
                json!({}),
            ));
        }
        let canonical_raw = render_with_context(&value, Some(&context))?;
        Ok(Self {
            value,
            canonical_raw,
            context,
        })
    }

    pub fn approval_sha256(
        &self,
        path: &str,
        expected_revision: u64,
        previous_sha256: Option<&str>,
    ) -> Result<String, ProgressIssue> {
        approval_with_context(
            path,
            expected_revision,
            previous_sha256,
            &self.value,
            Some(&self.context),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::sha256_hex;

    #[test]
    fn discussion_progress_context_decisions_and_role_boundaries_match_current_contract() {
        let mut value = json!({"schema":"work-discussion-progress",
            "requirement_id":"example","mode":"task","revision":1,
            "status":"discussion_only","title":"Discussion",
            "request":"Retain unfinished decisions","current_task_id":"TASK-001",
            "context":{"skill_selection":{"unavailable":true},"command":"do not run"},
            "source_status":["Revision pending"],"notes":["Reviewed evidence"],
            "confirmed_decisions":[{"statement":"Keep scope","rationale":"User decision"}],
            "tentative":["Proposed approach"],"open_questions":["Which validation?"],
            "next_discussion_point":"Confirm validation"});
        let original = value.clone();
        validate_progress(&value).unwrap();
        assert_eq!(value, original);
        assert_eq!(value["context"]["skill_selection"]["unavailable"], true);
        let mut parsed = ProgressDocument::parse(&render_progress(&value).unwrap()).unwrap();
        parsed.value["context"]["skill_selection"]["unavailable"] = json!(false);
        assert_eq!(value, original);
        for (field, replacement, reason) in [
            ("revision", json!(true), "invalid_progress_revision"),
            ("revision", json!(0), "invalid_progress_revision"),
            ("revision", json!(1.5), "invalid_progress_revision"),
            ("mode", json!("execute"), "invalid_progress_mode"),
            ("mode", json!("plan"), "invalid_progress_mode"),
            ("current_task_id", json!("TASK-1"), "invalid_progress_task"),
            ("status", json!("completed"), "invalid_progress_schema"),
        ] {
            value[field] = replacement;
            assert_eq!(validate_progress(&value).unwrap_err().reason_code, reason);
            value[field] = original[field].clone();
        }
        for decision in [
            json!({}),
            json!({"statement":""}),
            json!({"statement":"Keep","rationale":" "}),
            json!({"statement":"Keep","approved":true}),
        ] {
            value["confirmed_decisions"] = json!([decision]);
            assert!(validate_progress(&value).is_err());
        }
        value["mode"] = json!("task");
        value["current_task_id"] = Value::Null;
        value["confirmed_decisions"] = json!([{"statement":"Keep"}]);
        validate_progress(&value).unwrap();
    }

    #[test]
    fn task_example_bytes_and_discussion_only_rules() {
        let example = json!({"schema":"work-discussion-progress","requirement_id":"example","mode":"task","revision":1,"status":"discussion_only","title":"Example","request":"Example request.","current_task_id":null,"context":{},"source_status":[],"notes":[],"confirmed_decisions":[],"tentative":[],"open_questions":[],"next_discussion_point":"Continue."});
        let raw = render_progress(&example).unwrap();
        assert_eq!(raw.len(), 389);
        assert_eq!(
            sha256_hex(&raw),
            "b43e8b4fe1b6a66a2f2353f62a59082c56b9f1ae45511890a26d3fcb9d8f7d5e"
        );
        assert_eq!(
            approval_sha256(
                "outputs/work/progress/example/task/progress.json",
                0,
                None,
                &example
            )
            .unwrap(),
            "a0082819b36e8120797f5edc5f4b4e33116f7adbd0c3b2971ea83555a0769516"
        );
        let mut with_decision = example.clone();
        with_decision["confirmed_decisions"] = json!([{"statement":"Yes","rationale":"Because"}]);
        assert_eq!(
            sha256_hex(&render_progress(&with_decision).unwrap()),
            "d83ee4924837240b7c2b87e47baf60ef4b5e3babb04f8d0b9677ff2081779cb8"
        );
        let mut invalid = example;
        invalid["current_task_id"] = json!("TASK-1");
        assert_eq!(
            validate_progress(&invalid).unwrap_err().reason_code,
            "invalid_progress_task"
        );
    }

    #[test]
    fn task_unicode_fixture_matches_artifact_and_approval() {
        let progress = json!({"schema":"work-discussion-progress","requirement_id":"example","mode":"task","revision":1,"status":"discussion_only","title":"保存尚未完成的討論","request":"先記錄目前共識，稍後繼續。","current_task_id":null,"context":{"scope":["需求規劃"]},"source_status":["Revision is pending; acceptance decisions are missing."],"notes":["具體討論細節。"],"confirmed_decisions":[{"statement":"保留已確認需求。"}],"tentative":["候選驗收方式尚未決定。"],"open_questions":["哪些結果可供觀察？"],"next_discussion_point":"繼續確認驗收結果。"});
        assert_eq!(
            sha256_hex(&render_progress(&progress).unwrap()),
            "2864f829d9b2f219935b840a279f6c302772f2a88470ea7bfc7693bd969f6f89"
        );
        assert_eq!(
            approval_sha256(
                "outputs/work/progress/example/task/progress.json",
                0,
                None,
                &progress
            )
            .unwrap(),
            "a5811737a174ff198df4a05c12f3bc61b7153466c131114ceae33bdf10bc18a8"
        );
    }

    #[test]
    fn raw_progress_preserves_nested_context_order_without_new_dependency() {
        let value = json!({"schema":"work-discussion-progress","requirement_id":"example","mode":"task","revision":1,"status":"discussion_only","title":"Example","request":"Example request.","current_task_id":null,"context":{"a":2,"z":1},"source_status":[],"notes":[],"confirmed_decisions":[],"tentative":[],"open_questions":[],"next_discussion_point":"Continue."});
        let raw = br#"{"schema":"work-discussion-progress","requirement_id":"example","mode":"task","revision":1,"status":"discussion_only","title":"Example","request":"Example request.","current_task_id":null,"context":{"z":1,"a":2},"source_status":[],"notes":[],"confirmed_decisions":[],"tentative":[],"open_questions":[],"next_discussion_point":"Continue."}"#;
        let document = ProgressDocument::parse(raw).unwrap();
        assert_eq!(document.value, value);
        assert_eq!(
            sha256_hex(&document.canonical_raw),
            "b6643764e52be97a860af6d91387cbc853c3168b9218353af294a7a041c328ca"
        );
    }

    #[test]
    fn cli_progress_invalid_contract_cases_are_rejected() {
        let example = json!({"schema":"work-discussion-progress","requirement_id":"example",
            "mode":"task","revision":1,"status":"discussion_only","title":"Discussion",
            "request":"Resume later.","current_task_id":null,"context":{},"source_status":[],
            "notes":[],"confirmed_decisions":[],"tentative":[],"open_questions":[],
            "next_discussion_point":"Continue."});
        for (field, value) in [
            ("schema", json!("work-plan/v1")),
            ("status", json!("confirmed")),
            ("mode", json!("execute")),
            ("revision", json!(true)),
            ("revision", json!(0)),
            ("current_task_id", json!("TASK-1")),
            ("notes", json!("not an array")),
            ("context", json!([])),
            ("next_discussion_point", json!(" ")),
            (
                "confirmed_decisions",
                json!([{"statement":"Known","approved":true}]),
            ),
            (
                "confirmed_decisions",
                json!([{"statement":"Known","rationale":" "}]),
            ),
            ("requirement_id", json!("../outside")),
            ("requirement_id", json!("CON")),
        ] {
            let mut candidate = example.clone();
            candidate[field] = value;
            assert!(
                validate_progress(&candidate).is_err(),
                "{field}: {candidate}"
            );
        }
    }
}
