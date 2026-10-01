//! Explicit Work invocation syntax and entry classification.

use serde_json::{Value, json};

use crate::identifiers::{IdentifierIssue, RequirementId};

const SYNTAX: &str = "$work <plan|task|revise|migration|execute> -- <request>";
const PUBLIC_MODES: [&str; 5] = ["plan", "task", "revise", "migration", "execute"];

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
            "Only plan, task, revise, migration and execute are public Work modes.",
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
    let request_words: Vec<_> = request.split_whitespace().collect();
    if request_words.is_empty() {
        return Err(usage(
            "work_invocation_request_missing",
            "Supply the request after --; surrounding conversation is not a request.",
            json!({}),
        ));
    }
    let mut entry = if mode == "migration" {
        json!({"kind":"migration"})
    } else {
        json!({"kind":"workflow"})
    };
    if matches!(mode, "plan" | "task") && request_words[0] == "resume" {
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
        entry = json!({"kind":"progress_resume","requirement_id":id.as_str()});
    } else if mode == "task" && request_words.len() == 1 {
        if let Ok(id) = request_words[0].parse::<RequirementId>() {
            entry = json!({"kind":"task_planning","requirement_id":id.as_str()});
        }
    }
    let invocation =
        json!({"schema":"work-invocation/v1","mode":mode,"request":request,"entry":entry});
    let _: work_model::invocation::Invocation =
        serde_json::from_value(invocation.clone()).expect("invocation matches its model");
    Ok(invocation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_invocation_preserves_request_and_classifies_resume() {
        for mode in ["plan", "task", "revise", "migration", "execute"] {
            assert_eq!(
                parse_invocation(&format!("$work {mode} -- review")).unwrap()["mode"],
                mode
            );
        }
        let invalid = parse_invocation("$work unsupported -- review").unwrap_err();
        assert_eq!(invalid.reason_code, "work_invocation_mode_invalid");
        assert_eq!(
            invalid.details["modes"],
            json!(["plan", "task", "revise", "migration", "execute"])
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
        for mode in ["plan", "task", "revise", "execute"] {
            assert_eq!(
                parse_invocation(&format!("$work {mode} --{tail}")).unwrap(),
                json!({"schema":"work-invocation/v1","mode":mode,"request":tail,"entry":{"kind":"workflow"}})
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
            ("$work plan --request", "work_invocation_delimiter"),
            ("$work execute", "work_invocation_delimiter"),
            ("$work plan -- \n\t", "work_invocation_request_missing"),
        ] {
            assert_eq!(
                parse_invocation(text).unwrap_err().reason_code,
                code,
                "{text}"
            );
        }
        for mode in ["plan", "task"] {
            assert_eq!(
                parse_invocation(&format!("$work {mode} -- resume example-12\n")).unwrap()["entry"],
                json!({"kind":"progress_resume","requirement_id":"example-12"})
            );
        }
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
            assert!(parse_invocation(&format!("$work plan -- {request}")).is_err());
        }
    }
}
