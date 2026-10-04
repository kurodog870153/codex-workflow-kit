//! Explicit Work invocation syntax and entry classification.

use serde_json::{Value, json};
use work_model::invocation::{
    Invocation, InvocationConfirmation, InvocationEntry, InvocationEntryKind, InvocationMode,
    InvocationOrigin,
};
use work_model::schema::PublicSchema;

use crate::identifiers::{IdentifierIssue, RequirementId};

const SYNTAX: &str = "$work <task|revise|migration|execute> -- <request>";
use crate::protocol::INVOCATION_MODES as PUBLIC_MODES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationIssue {
    pub reason_code: &'static str,
    pub message: &'static str,
    pub details: Value,
    pub contract: bool,
}

fn usage(reason_code: &'static str, message: &'static str, details: Value) -> InvocationIssue {
    let mut result = json!({"syntax":SYNTAX});
    if let Some(fields) = details.as_object() {
        for (name, value) in fields {
            result[name] = value.clone();
        }
    }
    InvocationIssue {
        reason_code,
        message,
        details: result,
        contract: false,
    }
}

fn requirement_issue(value: &str, issue: IdentifierIssue) -> InvocationIssue {
    let (message, details) = match issue {
        IdentifierIssue::InvalidRequirementId => (
            "The requirement ID must use lowercase letters, digits, dots, underscores, or hyphens.",
            json!({"requirement_id":value}),
        ),
        IdentifierIssue::WindowsDeviceName => (
            "The path contains a reserved Windows device name.",
            json!({"field":"requirement_id","segment":value}),
        ),
        _ => (
            "The path contains an empty, current, or parent segment.",
            json!({"field":"requirement_id","segment":value}),
        ),
    };
    InvocationIssue {
        reason_code: issue.reason_code(),
        message,
        details,
        contract: true,
    }
}

fn words(text: &str) -> Vec<(usize, usize)> {
    let mut result = Vec::new();
    let mut start = None;
    for (index, character) in text.char_indices() {
        if character.is_whitespace() {
            if let Some(begin) = start.take() {
                result.push((begin, index));
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(begin) = start {
        result.push((begin, text.len()));
    }
    result
}

pub fn parse_invocation(text: &str) -> Result<Value, InvocationIssue> {
    let tokens = words(text);
    let token = |index: usize| tokens.get(index).map(|(start, end)| &text[*start..*end]);
    if token(0) != Some("$work") {
        return Err(usage(
            "work_invocation_not_explicit",
            "An explicit $work invocation must start the input.",
            json!({}),
        ));
    }
    let Some(mode) = token(1).filter(|mode| *mode != "--") else {
        return Err(usage(
            "work_invocation_mode_missing",
            "Choose one Work mode.",
            json!({"modes":PUBLIC_MODES}),
        ));
    };
    if !PUBLIC_MODES.contains(&mode) {
        return Err(usage(
            "work_invocation_mode_invalid",
            "Only task, revise, migration and execute are public Work modes.",
            json!({"modes":PUBLIC_MODES}),
        ));
    }
    if token(2) != Some("--") {
        return Err(usage(
            "work_invocation_delimiter",
            "Place -- directly after the mode, without hierarchy paths or extra tokens.",
            json!({}),
        ));
    }
    let request = &text[tokens[2].1..];
    let mode: InvocationMode = serde_json::from_value(json!(mode)).expect("validated public mode");
    build_invocation(mode, InvocationOrigin::Explicit, request, None)
}

fn classify_entry(mode: InvocationMode, request: &str) -> Result<InvocationEntry, InvocationIssue> {
    let request_words: Vec<_> = request.split_whitespace().collect();
    if request_words.is_empty() {
        return Err(usage(
            "work_invocation_request_missing",
            "Supply the request after --; surrounding conversation is not a request.",
            json!({}),
        ));
    }
    let mut entry = InvocationEntry {
        kind: if mode == InvocationMode::Migration {
            InvocationEntryKind::Migration
        } else {
            InvocationEntryKind::Workflow
        },
        requirement_id: None,
    };
    if mode == InvocationMode::Task && request_words[0] == "resume" {
        if request_words.len() != 2 {
            return Err(usage(
                "work_invocation_resume",
                "Use exactly resume <requirement-id> for discussion restoration.",
                json!({}),
            ));
        }
        let id = request_words[1]
            .parse::<RequirementId>()
            .map_err(|issue| requirement_issue(request_words[1], issue))?;
        entry = InvocationEntry {
            kind: InvocationEntryKind::ProgressResume,
            requirement_id: Some(id.as_str().into()),
        };
    } else if mode == InvocationMode::Task && request_words.len() == 1 {
        if let Ok(id) = request_words[0].parse::<RequirementId>() {
            entry = InvocationEntry {
                kind: InvocationEntryKind::TaskPlanning,
                requirement_id: Some(id.as_str().into()),
            };
        }
    }
    Ok(entry)
}

fn build_invocation(
    mode: InvocationMode,
    origin: InvocationOrigin,
    request: &str,
    confirmation: Option<InvocationConfirmation>,
) -> Result<Value, InvocationIssue> {
    let invocation = Invocation {
        schema: PublicSchema::WorkInvocationV1,
        mode,
        origin,
        request: request.into(),
        confirmation,
        entry: classify_entry(mode, request)?,
    };
    Ok(serde_json::to_value(invocation).expect("invocation serializes"))
}

/// Confirmation binds the exact mode and opaque request and cannot choose explicit origin.
pub fn confirm_invocation(value: &Value) -> Result<Value, InvocationIssue> {
    let input: work_model::invocation::ImplicitInvocationRequest =
        serde_json::from_value(value.clone()).map_err(|_| {
            usage(
                "work_invocation_confirmation_invalid",
                "A strict mode, request and user confirmation are required.",
                json!({}),
            )
        })?;
    if !input.confirmation.confirmed || input.confirmation.evidence.trim().is_empty() {
        return Err(usage(
            "work_invocation_confirmation_required",
            "User confirmation evidence is required before implicit invocation.",
            json!({}),
        ));
    }
    if input.mode != input.confirmation.mode || input.request != input.confirmation.request {
        return Err(usage(
            "work_invocation_confirmation_mismatch",
            "Confirmation must cover the exact mode and original request.",
            json!({}),
        ));
    }
    build_invocation(
        input.mode,
        InvocationOrigin::ImplicitConfirmed,
        &input.request,
        Some(input.confirmation),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_invocation_preserves_request_and_classifies_resume() {
        for mode in ["task", "revise", "migration", "execute"] {
            assert_eq!(
                parse_invocation(&format!("$work {mode} -- review")).unwrap()["mode"],
                mode
            );
        }
        let invalid = parse_invocation("$work unsupported -- review").unwrap_err();
        assert_eq!(invalid.reason_code, "work_invocation_mode_invalid");
        assert_eq!(
            invalid.details["modes"],
            json!(["task", "revise", "migration", "execute"])
        );
        assert_eq!(
            parse_invocation("$work migration -- example").unwrap()["entry"],
            json!({"kind":"migration"})
        );
        assert_eq!(
            parse_invocation("$work revise -- 修改 example 的 TASK-001 goal").unwrap()["entry"],
            json!({"kind":"workflow"})
        );
        let parsed = parse_invocation("$work task --  resume issue55\n").unwrap();
        assert_eq!(parsed["request"], "  resume issue55\n");
        assert_eq!(
            parsed["entry"],
            json!({"kind":"progress_resume","requirement_id":"issue55"})
        );
        assert_eq!(
            parse_invocation("$work task -- issue55").unwrap()["entry"],
            json!({"kind":"task_planning","requirement_id":"issue55"})
        );
        assert_eq!(
            parse_invocation("$work task -- resume BAD")
                .unwrap_err()
                .reason_code,
            "invalid_requirement_id"
        );
    }

    #[test]
    fn python_invocation_preserves_opaque_requests_and_rejects_bad_headers() {
        let tail =
            "  建立 e\u{0301}\r\n\"quoted\" -- $HOME $(command)\n$work execute -- embedded\n";
        for mode in ["task", "revise", "execute"] {
            assert_eq!(
                parse_invocation(&format!("$work {mode} --{tail}")).unwrap(),
                json!({"schema":"work-invocation/v1","mode":mode,"origin":"explicit","request":tail,"entry":{"kind":"workflow"}})
            );
        }
        for (text, code) in [
            (
                "請使用 $work plan -- example",
                "work_invocation_not_explicit",
            ),
            ("`$work plan -- example`", "work_invocation_not_explicit"),
            ("$workflow plan -- example", "work_invocation_not_explicit"),
            ("$work", "work_invocation_mode_missing"),
            ("$work -- example", "work_invocation_mode_missing"),
            ("$work Plan -- example", "work_invocation_mode_invalid"),
            (
                "$work artifact-editor -- example",
                "work_invocation_mode_invalid",
            ),
            (
                "$work progress-saver -- example",
                "work_invocation_mode_invalid",
            ),
            (
                "$work task web/backend -- example",
                "work_invocation_delimiter",
            ),
            ("$work task --request", "work_invocation_delimiter"),
            ("$work execute", "work_invocation_delimiter"),
            ("$work task -- \n\t", "work_invocation_request_missing"),
        ] {
            assert_eq!(
                parse_invocation(text).unwrap_err().reason_code,
                code,
                "{text}"
            );
        }
        assert_eq!(
            parse_invocation("$work task -- resume example-12\n").unwrap()["entry"],
            json!({"kind":"progress_resume","requirement_id":"example-12"})
        );
        assert_eq!(
            parse_invocation("$work task -- example-12").unwrap()["entry"],
            json!({"kind":"task_planning","requirement_id":"example-12"})
        );
        for request in [
            "resume",
            "resume example more",
            "resume ../example",
            "resume EXAMPLE",
            "resume ..",
        ] {
            assert!(parse_invocation(&format!("$work task -- {request}")).is_err());
        }
    }
    #[test]
    fn explicit_and_implicit_share_entry_rules_without_reparsing_opaque_requests() {
        for (mode, request) in [
            ("task", "example"),
            ("task", "resume example"),
            ("task", "討論需求"),
            ("task", "$work execute -- embedded"),
            ("revise", "resume example"),
            ("migration", "example"),
            ("execute", "example"),
        ] {
            let explicit = parse_invocation(&format!("$work {mode} --{request}"));
            // Explicit syntax requires a separate delimiter token; implicit requests do not.
            assert!(explicit.is_err());
            let explicit = parse_invocation(&format!("$work {mode} -- {request}")).unwrap();
            let implicit = confirm_invocation(&json!({"mode":mode,"request":request,
                "confirmation":{"mode":mode,"request":request,"confirmed":true,"evidence":"Approved exact input."}})).unwrap();
            assert_eq!(implicit["schema"], "work-invocation/v1");
            assert_eq!(implicit["entry"], explicit["entry"]);
            assert_eq!(implicit["request"], request);
        }
        for request in ["", " \r\n", "resume", "resume BAD", "resume example extra"] {
            assert!(confirm_invocation(&json!({"mode":"task","request":request,
                "confirmation":{"mode":"task","request":request,"confirmed":true,"evidence":"Approved."}})).is_err());
        }
    }

    #[test]
    fn implicit_origin_requires_exact_confirmed_mode_request_and_preserves_original_text() {
        let request = "  建立 e\u{0301}\r\n\"quoted\" -- $HOME $(command)\n";
        let input = json!({"mode":"task","request":request,"confirmation":{"mode":"task","request":request,"confirmed":true,"evidence":"User confirmed task mode and this full request."}});
        let actual = confirm_invocation(&input).unwrap();
        assert_eq!(actual["origin"], "implicit_confirmed");
        assert_eq!(actual["request"], request);
        assert_eq!(actual["confirmation"], input["confirmation"]);
        let parsed: work_model::invocation::Invocation =
            serde_json::from_value(actual.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), actual);
        assert_eq!(
            parse_invocation("$work plan -- request")
                .unwrap_err()
                .reason_code,
            "work_invocation_mode_invalid"
        );
        for field in ["mode", "request"] {
            let mut mismatch = input.clone();
            mismatch["confirmation"][field] = json!(if field == "mode" {
                "execute"
            } else {
                "Changed request"
            });
            assert_eq!(
                confirm_invocation(&mismatch).unwrap_err().reason_code,
                "work_invocation_confirmation_mismatch"
            );
        }
        for change in [
            "unconfirmed",
            "no_evidence",
            "missing",
            "forged_origin",
            "legacy_mode",
        ] {
            let mut invalid = input.clone();
            match change {
                "unconfirmed" => invalid["confirmation"]["confirmed"] = json!(false),
                "no_evidence" => invalid["confirmation"]["evidence"] = json!(" "),
                "missing" => {
                    invalid.as_object_mut().unwrap().remove("confirmation");
                }
                "forged_origin" => invalid["origin"] = json!("explicit"),
                "legacy_mode" => invalid["mode"] = json!("plan"),
                _ => unreachable!(),
            }
            assert!(confirm_invocation(&invalid).is_err(), "{change}");
        }
        let mut forged = actual.clone();
        forged.as_object_mut().unwrap().remove("confirmation");
        assert!(serde_json::from_value::<work_model::invocation::Invocation>(forged).is_err());
        let mut forged = actual.clone();
        forged["origin"] = json!("explicit");
        assert!(serde_json::from_value::<work_model::invocation::Invocation>(forged).is_err());
        let mut missing_origin = parse_invocation("$work task -- request").unwrap();
        missing_origin.as_object_mut().unwrap().remove("origin");
        assert!(
            serde_json::from_value::<work_model::invocation::Invocation>(missing_origin).is_err()
        );
    }
}
