//! Process-boundary response contract for Work.

pub mod contract;
pub mod discussion;
pub mod parser;
pub mod runtime;

use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};
use serde_json::Value;
use work_flow::error::{ExitCode, WorkError};
use work_flow::execution::{ordered_attempt, ordered_correction};

pub use work_model::common::CliStatus as Status;

#[derive(Debug, Serialize)]
pub struct CliResult {
    pub schema: &'static str,
    pub status: Status,
    pub reason_code: String,
    pub message: String,
    pub data: Value,
    #[serde(skip)]
    pub order_hint: Option<String>,
}

#[derive(Serialize)]
struct OrderedEnvelope<'a, T: Serialize> {
    schema: &'a str,
    status: Status,
    reason_code: &'a str,
    message: &'a str,
    data: T,
}

struct ScaffoldValue<'a> {
    value: &'a Value,
    orders: &'a Value,
    path: String,
}

impl Serialize for ScaffoldValue<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.value {
            Value::Array(items) => {
                let mut sequence = serializer.serialize_seq(Some(items.len()))?;
                for (index, item) in items.iter().enumerate() {
                    sequence.serialize_element(&ScaffoldValue {
                        value: item,
                        orders: self.orders,
                        path: format!("{}/{index}", self.path),
                    })?;
                }
                sequence.end()
            }
            Value::Object(items) => {
                let order = self.orders[&self.path].as_array();
                let mut entries = items.iter().collect::<Vec<_>>();
                entries.sort_by_key(|(key, _)| {
                    (
                        order
                            .and_then(|fields| {
                                fields
                                    .iter()
                                    .position(|field| field.as_str() == Some(key.as_str()))
                            })
                            .unwrap_or(usize::MAX),
                        *key,
                    )
                });
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(
                        key,
                        &ScaffoldValue {
                            value,
                            orders: self.orders,
                            path: format!("{}/{key}", self.path),
                        },
                    )?;
                }
                map.end()
            }
            value => value.serialize(serializer),
        }
    }
}

impl CliResult {
    pub fn success(data: Value, already_completed: bool) -> Self {
        if already_completed {
            Self {
                schema: "work-cli-result",
                status: Status::AlreadyCompleted,
                reason_code: "already_completed".into(),
                message: "The requested operation was already completed.".into(),
                data,
                order_hint: None,
            }
        } else {
            Self {
                schema: "work-cli-result",
                status: Status::Success,
                reason_code: "ok".into(),
                message: "The command completed successfully.".into(),
                data,
                order_hint: None,
            }
        }
    }

    pub fn from_error(error: WorkError) -> Self {
        Self {
            schema: "work-cli-result",
            status: if error.status() == "failed" {
                Status::Failed
            } else {
                Status::Rejected
            },
            reason_code: error.reason_code,
            message: error.message,
            data: error.details,
            order_hint: None,
        }
    }

    pub fn to_stdout(&self) -> Result<Vec<u8>, serde_json::Error> {
        let _: work_model::contract::CliEnvelope =
            serde_json::from_value(serde_json::to_value(self)?)
                .expect("CLI result matches its model");
        let mut bytes = if let Some(orders) = self
            .order_hint
            .as_deref()
            .and_then(contract::scaffold_order)
        {
            serde_json::to_vec_pretty(&OrderedEnvelope {
                schema: self.schema,
                status: self.status,
                reason_code: &self.reason_code,
                message: &self.message,
                data: ScaffoldValue {
                    value: &self.data,
                    orders,
                    path: String::new(),
                },
            })?
        } else if self.order_hint.as_deref() == Some("attempt-render") {
            serde_json::to_vec_pretty(&OrderedEnvelope {
                schema: self.schema,
                status: self.status,
                reason_code: &self.reason_code,
                message: &self.message,
                data: ordered_attempt(&self.data),
            })?
        } else if self.order_hint.as_deref() == Some("correction-render") {
            serde_json::to_vec_pretty(&OrderedEnvelope {
                schema: self.schema,
                status: self.status,
                reason_code: &self.reason_code,
                message: &self.message,
                data: ordered_correction(&self.data),
            })?
        } else if self.data.as_object().is_some_and(|details| {
            details.len() == 2
                && details.contains_key("recovery_required")
                && details.contains_key("record")
        }) {
            #[derive(Serialize)]
            struct RecoveryRecordData<'a> {
                recovery_required: &'a Value,
                record: &'a Value,
            }
            serde_json::to_vec_pretty(&OrderedEnvelope {
                schema: self.schema,
                status: self.status,
                reason_code: &self.reason_code,
                message: &self.message,
                data: RecoveryRecordData {
                    recovery_required: &self.data["recovery_required"],
                    record: &self.data["record"],
                },
            })?
        } else if self.reason_code == "invalid_object_fields"
            && self.data.as_object().is_some_and(|details| {
                details.len() == 4
                    && details.contains_key("location")
                    && details.contains_key("missing")
                    && details.contains_key("unknown")
                    && details.contains_key("issues")
            })
        {
            #[derive(Serialize)]
            struct ModelFieldsData<'a> {
                location: &'a Value,
                missing: &'a Value,
                unknown: &'a Value,
                issues: &'a Value,
            }
            serde_json::to_vec_pretty(&OrderedEnvelope {
                schema: self.schema,
                status: self.status,
                reason_code: &self.reason_code,
                message: &self.message,
                data: ModelFieldsData {
                    location: &self.data["location"],
                    missing: &self.data["missing"],
                    unknown: &self.data["unknown"],
                    issues: &self.data["issues"],
                },
            })?
        } else if self.reason_code == "invalid_utf8"
            && self.data.as_object().is_some_and(|details| {
                details.len() == 2
                    && details.contains_key("source")
                    && details.contains_key("byte_offset")
            })
        {
            #[derive(Serialize)]
            struct Utf8LocationData<'a> {
                source: &'a Value,
                byte_offset: &'a Value,
            }
            serde_json::to_vec_pretty(&OrderedEnvelope {
                schema: self.schema,
                status: self.status,
                reason_code: &self.reason_code,
                message: &self.message,
                data: Utf8LocationData {
                    source: &self.data["source"],
                    byte_offset: &self.data["byte_offset"],
                },
            })?
        } else if self.reason_code == "invalid_json_contract"
            && self.data.as_object().is_some_and(|details| {
                details.len() == 2 && details.contains_key("line") && details.contains_key("column")
            })
        {
            #[derive(Serialize)]
            struct JsonLocationData<'a> {
                line: &'a Value,
                column: &'a Value,
            }
            serde_json::to_vec_pretty(&OrderedEnvelope {
                schema: self.schema,
                status: self.status,
                reason_code: &self.reason_code,
                message: &self.message,
                data: JsonLocationData {
                    line: &self.data["line"],
                    column: &self.data["column"],
                },
            })?
        } else {
            serde_json::to_vec_pretty(self)?
        };
        bytes.push(b'\n');
        Ok(bytes)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResponseIssue {
    Empty,
    Malformed,
    InvalidContract,
}

pub fn parse_response(stdout: &[u8], exit_code: i32) -> Result<Value, ResponseIssue> {
    if stdout.is_empty() {
        return Err(ResponseIssue::Empty);
    }
    let response: Value = serde_json::from_slice(stdout).map_err(|_| ResponseIssue::Malformed)?;
    let object = response.as_object().ok_or(ResponseIssue::InvalidContract)?;
    if object.get("schema").and_then(Value::as_str) != Some("work-cli-result")
        || object.get("reason_code").and_then(Value::as_str).is_none()
        || object.get("message").and_then(Value::as_str).is_none()
        || !object.contains_key("data")
    {
        return Err(ResponseIssue::InvalidContract);
    }
    let valid = match object.get("status").and_then(Value::as_str) {
        Some("success" | "already_completed") => exit_code == ExitCode::Success as i32,
        Some("rejected") => (2..=7).contains(&exit_code),
        Some("failed") => exit_code == 8 || exit_code == 10,
        _ => false,
    };
    if !valid {
        return Err(ResponseIssue::InvalidContract);
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_error_envelopes_match_current_contract_stdout() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../crates/work-operations/fixtures/migration.json"
        ))
        .unwrap();
        for case in fixtures["cli_cases"].as_array().unwrap().iter().take(2) {
            let expected: Value = serde_json::from_str(case["stdout"].as_str().unwrap()).unwrap();
            let error = WorkError::new(
                ExitCode::CliUsage,
                expected["reason_code"].as_str().unwrap(),
                expected["message"].as_str().unwrap(),
                expected["data"].clone(),
            );
            let bytes = CliResult::from_error(error).to_stdout().unwrap();
            assert_eq!(bytes, case["stdout"].as_str().unwrap().as_bytes());
            assert!(parse_response(&bytes, 2).is_ok());
        }
    }

    #[test]
    fn migration_success_envelope_matches_current_contract_stdout() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../crates/work-operations/fixtures/migration.json"
        ))
        .unwrap();
        let case = &fixtures["cli_cases"][2];
        let expected: Value = serde_json::from_str(case["stdout"].as_str().unwrap()).unwrap();
        let bytes = CliResult::success(expected["data"].clone(), false)
            .to_stdout()
            .unwrap();
        assert_eq!(bytes, case["stdout"].as_str().unwrap().as_bytes());
        assert!(parse_response(&bytes, 0).is_ok());
        let completed = CliResult::success(serde_json::json!({}), true);
        assert_eq!(completed.status, Status::AlreadyCompleted);
        assert_eq!(completed.reason_code, "already_completed");
    }

    #[test]
    fn malformed_and_empty_stdout_cannot_succeed() {
        assert_eq!(parse_response(b"", 0), Err(ResponseIssue::Empty));
        assert_eq!(parse_response(b"{", 0), Err(ResponseIssue::Malformed));
        assert_eq!(
            parse_response(b"{}", 0),
            Err(ResponseIssue::InvalidContract)
        );
        let failed = CliResult::from_error(WorkError::new(
            ExitCode::IoFailure,
            "io_failure",
            "I/O failed.",
            serde_json::json!({}),
        ));
        assert_eq!(
            parse_response(&failed.to_stdout().unwrap(), 0),
            Err(ResponseIssue::InvalidContract)
        );
    }
}
