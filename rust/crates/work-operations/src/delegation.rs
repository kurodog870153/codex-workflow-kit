//! Delegation envelope role, sender and mode boundaries.

use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegationIssue {
    pub reason_code: &'static str,
    pub message: &'static str,
}

fn issue(message: &'static str) -> DelegationIssue {
    DelegationIssue {
        reason_code: "delegation_boundary_mismatch",
        message,
    }
}

pub fn role_marker(role: &str) -> Option<&'static str> {
    match role {
        "task-coordinator" | "execute" => Some("WORK_DELEGATION"),
        "task-skill" => Some("WORK_TASK_SKILL"),
        "artifact-editor" => Some("WORK_ARTIFACT_EDIT"),
        _ => None,
    }
}

pub fn sender_for_role(role: &str) -> Option<&'static str> {
    role_marker(role).map(|_| {
        if role == "task-skill" {
            "task-coordinator"
        } else {
            "parent"
        }
    })
}

fn mode_allowed(role: &str, mode: &str) -> bool {
    match role {
        "task-coordinator" | "task-skill" => mode == "task",
        "execute" => mode == "execute",
        "artifact-editor" => matches!(mode, "task" | "execute"),
        _ => false,
    }
}

pub fn build_envelope(
    role: &str,
    mode: &str,
    request: &str,
    project_root: &str,
    skill_root: &str,
    context: &Value,
) -> Result<Value, DelegationIssue> {
    let marker = role_marker(role).ok_or_else(|| issue("Unknown delegation role."))?;
    if !mode_allowed(role, mode) || request.trim().is_empty() || !context.is_object() {
        return Err(issue("The delegation envelope structure is invalid."));
    }
    let envelope = json!({"schema":"work-delegation-envelope","marker":marker,"skill":"$work","role":role,"sender":sender_for_role(role).expect("known role"),"mode":mode,"project_root":project_root,"skill_root":skill_root,"request":request,"context":context});
    let _: work_model::delegation::DelegationEnvelope = serde_json::from_value(envelope.clone())
        .expect("built delegation envelope matches its model");
    Ok(envelope)
}

pub fn validate_envelope(
    value: &Value,
    role: &str,
    sender: &str,
    project_root: &str,
    skill_root: &str,
) -> Result<(String, String, Value, bool), DelegationIssue> {
    let object = value
        .as_object()
        .filter(|object| {
            object.len() == 10
                && [
                    "schema",
                    "marker",
                    "skill",
                    "role",
                    "sender",
                    "mode",
                    "project_root",
                    "skill_root",
                    "request",
                    "context",
                ]
                .iter()
                .all(|field| object.contains_key(*field))
                && object["schema"] == "work-delegation-envelope"
                && object["marker"].is_string()
                && object["skill"] == "$work"
                && object["role"]
                    .as_str()
                    .is_some_and(|value| role_marker(value).is_some())
                && matches!(
                    object["sender"].as_str(),
                    Some("parent" | "task-coordinator")
                )
                && matches!(object["mode"].as_str(), Some("task" | "execute"))
                && object["project_root"].is_string()
                && object["skill_root"].is_string()
                && object["request"].is_string()
                && object["context"].is_object()
        })
        .ok_or_else(|| issue("The delegation envelope structure is invalid."))?;
    let marker = role_marker(role)
        .ok_or_else(|| issue("The expected sender cannot delegate to this role."))?;
    if sender_for_role(role) != Some(sender) {
        return Err(issue("The expected sender cannot delegate to this role."));
    }
    if object["schema"] != "work-delegation-envelope"
        || object["marker"] != marker
        || object["skill"] != "$work"
        || object["role"] != role
        || object["sender"] != sender
    {
        return Err(issue(
            "Envelope marker, skill, role or sender differs from the receiving context.",
        ));
    }
    if object["project_root"] != project_root
        || object["skill_root"] != skill_root
        || !Path::new(project_root).is_absolute()
        || !Path::new(skill_root).is_absolute()
    {
        return Err(issue(
            "Envelope roots must match the resolved receiving roots.",
        ));
    }
    let request = object["request"]
        .as_str()
        .filter(|request| !request.trim().is_empty())
        .ok_or_else(|| issue("request must be a nonempty string."))?;
    let mode = object["mode"]
        .as_str()
        .filter(|mode| mode_allowed(role, mode))
        .ok_or_else(|| issue("The role does not accept this mode."))?;
    let context = object["context"]
        .as_object()
        .filter(|context| !context.is_empty())
        .ok_or_else(|| issue("context must be a nonempty object."))?;
    Ok((
        request.into(),
        mode.into(),
        Value::Object(context.clone()),
        context.contains_key("session_view"),
    ))
}

pub fn validation_result(role: &str, mode: &str, resume: bool) -> Value {
    let result = json!({"schema":"work-delegation-validation","status":"valid","role":role,"mode":mode,"scope":if resume { "discussion_restoration" } else { "role_context" },"source_validation":"not_checked","sender_authentication":"not_checked","grants_authorization":false});
    let _: work_model::delegation::DelegationValidation =
        serde_json::from_value(result.clone()).expect("delegation validation matches its model");
    result
}

struct OrderedEnvelope<'a>(&'a Value);

impl Serialize for OrderedEnvelope<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let object = self.0.as_object().expect("validated envelope");
        let mut output = serializer.serialize_map(Some(object.len()))?;
        for field in [
            "schema",
            "marker",
            "skill",
            "role",
            "sender",
            "mode",
            "project_root",
            "skill_root",
            "request",
            "context",
        ] {
            if let Some(value) = object.get(field) {
                output.serialize_entry(field, value)?;
            }
        }
        output.end()
    }
}

pub fn render_envelope(value: &Value) -> Result<Vec<u8>, DelegationIssue> {
    let role = value["role"]
        .as_str()
        .ok_or_else(|| issue("The delegation envelope structure is invalid."))?;
    let sender = sender_for_role(role).ok_or_else(|| issue("Unknown delegation role."))?;
    validate_envelope(
        value,
        role,
        sender,
        value["project_root"].as_str().unwrap_or(""),
        value["skill_root"].as_str().unwrap_or(""),
    )?;
    let mut raw =
        serde_json::to_vec_pretty(&OrderedEnvelope(value)).expect("JSON value serializes");
    raw.push(b'\n');
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn absolute_roots() -> (&'static str, &'static str) {
        if cfg!(windows) {
            ("C:/project", "C:/work")
        } else {
            ("/project", "/work")
        }
    }

    #[test]
    fn role_sender_mode_and_resume_boundaries() {
        let (project_root, skill_root) = absolute_roots();
        let context = json!({"session_view":{"revision":1}});
        assert!(
            build_envelope(
                "plan",
                "plan",
                "Removed role",
                project_root,
                skill_root,
                &context
            )
            .is_err()
        );
        assert!(
            build_envelope(
                "task-coordinator",
                "plan",
                "Save",
                project_root,
                skill_root,
                &context
            )
            .is_err()
        );
        let envelope = build_envelope(
            "task-coordinator",
            "task",
            "Continue",
            project_root,
            skill_root,
            &context,
        )
        .unwrap();
        assert!(
            validate_envelope(
                &envelope,
                "task-coordinator",
                "parent",
                project_root,
                skill_root
            )
            .unwrap()
            .3
        );
        assert_eq!(
            validation_result("task-coordinator", "task", true)["scope"],
            "discussion_restoration"
        );
        assert_eq!(
            validate_envelope(
                &envelope,
                "task-coordinator",
                "task-coordinator",
                project_root,
                skill_root
            )
            .unwrap_err()
            .reason_code,
            "delegation_boundary_mismatch"
        );
        assert!(
            build_envelope(
                "task-skill",
                "plan",
                "Review",
                project_root,
                skill_root,
                &context
            )
            .is_err()
        );
        let mut relative = envelope.clone();
        relative["project_root"] = json!("relative/project");
        assert_eq!(
            validate_envelope(
                &relative,
                "task-coordinator",
                "parent",
                "relative/project",
                skill_root
            )
            .unwrap_err()
            .reason_code,
            "delegation_boundary_mismatch"
        );
    }

    #[test]
    fn current_contract_envelope_literal_errors_precede_receiver_mismatch() {
        let (project_root, skill_root) = absolute_roots();
        let envelope = build_envelope(
            "task-coordinator",
            "task",
            "Confirmed role request",
            project_root,
            skill_root,
            &json!({"hierarchy_selection":{}}),
        )
        .unwrap();
        let (request, mode, context, resume) = validate_envelope(
            &envelope,
            "task-coordinator",
            "parent",
            project_root,
            skill_root,
        )
        .unwrap();
        assert_eq!(request, "Confirmed role request");
        assert_eq!(mode, "task");
        assert_eq!(context, envelope["context"]);
        assert!(!resume);
        for (field, changed) in [
            ("sender", json!("execute")),
            ("skill", json!("$other")),
            ("schema", json!("other/v1")),
        ] {
            let mut invalid = envelope.clone();
            invalid[field] = changed;
            assert_eq!(
                validate_envelope(
                    &invalid,
                    "task-coordinator",
                    "parent",
                    project_root,
                    skill_root
                )
                .unwrap_err()
                .message,
                "The delegation envelope structure is invalid.",
                "{field}"
            );
        }
        for (field, changed) in [
            ("mode", json!("execute")),
            ("request", json!("")),
            ("project_root", json!(".")),
            ("skill_root", json!(".")),
            ("authorized", json!(true)),
        ] {
            let mut invalid = envelope.clone();
            invalid[field] = changed;
            assert!(
                validate_envelope(
                    &invalid,
                    "task-coordinator",
                    "parent",
                    project_root,
                    skill_root
                )
                .is_err(),
                "{field}"
            );
        }
        assert!(
            validate_envelope(&envelope, "execute", "parent", project_root, skill_root).is_err()
        );
        let mut wrong_marker = envelope;
        wrong_marker["marker"] = json!("WORK_PROGRESS_SAVE");
        assert_eq!(
            validate_envelope(
                &wrong_marker,
                "task-coordinator",
                "parent",
                project_root,
                skill_root
            )
            .unwrap_err()
            .message,
            "Envelope marker, skill, role or sender differs from the receiving context."
        );
    }

    #[test]
    fn receiving_roles_reject_versioned_markers() {
        let (project_root, skill_root) = absolute_roots();
        for (role, mode) in [
            ("task-coordinator", "task"),
            ("task-skill", "task"),
            ("execute", "execute"),
            ("artifact-editor", "task"),
        ] {
            let mut envelope = build_envelope(
                role,
                mode,
                "Confirmed request",
                project_root,
                skill_root,
                &json!({"repository_evidence":[]}),
            )
            .unwrap();
            let sender = sender_for_role(role).unwrap();
            validate_envelope(&envelope, role, sender, project_root, skill_root).unwrap();
            envelope["marker"] = json!(format!("{}_V1", role_marker(role).unwrap()));
            assert_eq!(
                validate_envelope(&envelope, role, sender, project_root, skill_root)
                    .unwrap_err()
                    .reason_code,
                "delegation_boundary_mismatch",
                "{role}"
            );
        }
    }

    #[test]
    fn current_contract_envelope_example_has_exact_canonical_bytes() {
        let example = json!({"schema":"work-delegation-envelope","marker":"WORK_DELEGATION","skill":"$work","role":"task-coordinator","sender":"parent","mode":"task","project_root":"/project","skill_root":"/work","request":"Confirmed role request.","context":{"task_source":{}}});
        let mut raw = serde_json::to_vec_pretty(&OrderedEnvelope(&example)).unwrap();
        raw.push(b'\n');
        assert_eq!(
            crate::canonical::sha256_hex(&raw),
            "75719577aa23351afba2135ba64dbf66d8f39278e97355feba0767d2c5817877"
        );
    }
}
