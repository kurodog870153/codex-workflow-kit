//! Specification verification request shape before journal access.

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq)]
pub struct VerificationIssue {
    pub reason_code: &'static str,
    pub message: &'static str,
    pub details: Value,
}

fn issue(reason_code: &'static str, message: &'static str, location: &str) -> VerificationIssue {
    VerificationIssue {
        reason_code,
        message,
        details: json!({"location":location}),
    }
}

fn fields(value: &Value, required: &[&str], location: &str) -> Result<(), VerificationIssue> {
    let object = value
        .as_object()
        .ok_or_else(|| issue("expected_object", "A JSON object is required.", location))?;
    let missing = required
        .iter()
        .filter(|key| !object.contains_key(**key))
        .collect::<Vec<_>>();
    let unknown = object
        .keys()
        .filter(|key| !required.contains(&key.as_str()))
        .collect::<Vec<_>>();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(VerificationIssue {
            reason_code: "invalid_object_fields",
            message: "The JSON object has missing or unknown fields.",
            details: json!({"location":location,"missing":missing,"unknown":unknown}),
        });
    }
    Ok(())
}

pub fn valid_record_id(value: &str) -> bool {
    value.strip_prefix("SPEC-UPDATE-").is_some_and(|suffix| {
        suffix.len() == 12
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
    })
}

pub fn validate_request(value: &Value) -> Result<(), VerificationIssue> {
    fields(
        value,
        &["schema", "requirement_id", "artifacts", "record_id"],
        "spec_verification",
    )?;
    if value["schema"] != "work-spec-verification-request/v1" {
        return Err(issue(
            "invalid_contract_value",
            "The verification request schema is invalid.",
            "schema",
        ));
    }
    if value["requirement_id"]
        .as_str()
        .is_none_or(|text| text.trim().is_empty())
    {
        return Err(issue(
            "empty_text_value",
            "A non-empty requirement is required.",
            "requirement_id",
        ));
    }
    fields(
        &value["artifacts"],
        &["plan", "task", "execution"],
        "artifacts",
    )?;
    for key in ["plan", "task", "execution"] {
        if !value["artifacts"][key].is_string() {
            return Err(issue(
                "invalid_contract_value",
                "A path string is required.",
                key,
            ));
        }
    }
    if value["record_id"]
        .as_str()
        .is_none_or(|id| !valid_record_id(id))
    {
        return Err(issue(
            "invalid_contract_value",
            "The verification record ID is invalid.",
            "record_id",
        ));
    }
    let _: work_model::specification::SpecVerificationRequest =
        serde_json::from_value(value.clone())
            .expect("validated verification request matches its model");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example() -> Value {
        json!({"schema":"work-spec-verification-request/v1","requirement_id":"example",
            "artifacts":{"plan":"outputs/work/plans/example.json",
                "task":"outputs/work/tasks/example/index.json",
                "execution":"outputs/work/executions/example"},
            "record_id":"SPEC-UPDATE-ABCDEF012345"})
    }

    #[test]
    fn verification_request_accepts_approval_prefix_and_rejects_malformed_shapes() {
        let value = example();
        validate_request(&value).unwrap();
        let raw = crate::canonical::canonical_json(&value).unwrap();
        validate_request(&serde_json::from_slice::<Value>(&raw).unwrap()).unwrap();
        assert!(valid_record_id("SPEC-UPDATE-ABCDEF012345"));
        for invalid in [
            json!([]),
            json!({"schema":"work-spec-verification-request/v2","requirement_id":"example","artifacts":value["artifacts"],"record_id":value["record_id"]}),
            json!({"schema":value["schema"],"requirement_id":"example","artifacts":value["artifacts"],"record_id":value["record_id"],"unexpected":true}),
            json!({"schema":value["schema"],"requirement_id":null,"artifacts":value["artifacts"],"record_id":value["record_id"]}),
            json!({"schema":value["schema"],"requirement_id":"example","artifacts":value["artifacts"],"record_id":"../other"}),
            json!({"schema":value["schema"],"requirement_id":"example","artifacts":value["artifacts"],"record_id":2}),
            json!({"schema":value["schema"],"requirement_id":"example","artifacts":value["artifacts"]}),
            json!({"schema":value["schema"],"requirement_id":"example","artifacts":{"plan":"example.json"},"record_id":value["record_id"]}),
            json!({"schema":value["schema"],"requirement_id":"example","artifacts":{"plan":"p","task":"t","execution":"e","unexpected":true},"record_id":value["record_id"]}),
        ] {
            assert!(validate_request(&invalid).is_err(), "{invalid}");
        }
    }
}
