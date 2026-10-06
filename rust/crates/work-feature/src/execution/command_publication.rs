//! Approval and receipt identity checks before command execution.

use crate::error::{ExitCode, WorkError};
use crate::ports::{CommandOutcome, CommandRequest, CommandStatus};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;
use work_operations::execution::command_run::{
    build_command_result, command_preview_approval_sha256, validate_command_run_request,
};

pub struct CommandCompletion {
    pub finished: Value,
    response: Value,
    failed: bool,
}

impl CommandCompletion {
    pub fn into_result(self) -> Result<Value, WorkError> {
        if self.failed {
            Err(WorkError::new(
                ExitCode::WorkflowState,
                "command_run_failed",
                "The command did not succeed. Review evidence and effects before continuing; never retry automatically.",
                self.response,
            ))
        } else {
            Ok(self.response)
        }
    }
}

pub fn command_completion(preview: &Value, outcome: &CommandOutcome) -> CommandCompletion {
    command_completion_scoped(preview, outcome, false)
}

pub fn command_completion_with_receipts(
    preview: &Value,
    outcome: &CommandOutcome,
) -> CommandCompletion {
    command_completion_scoped(preview, outcome, true)
}

fn command_completion_scoped(
    preview: &Value,
    outcome: &CommandOutcome,
    receipt_directory: bool,
) -> CommandCompletion {
    let status = match outcome.status {
        CommandStatus::Exited => "exited",
        CommandStatus::TimedOut => "timed_out",
        CommandStatus::LaunchFailed => "launch_failed",
    };
    let process = json!({"status":status,"exit_code":outcome.exit_code,
        "stdout_tail":outcome.stdout_tail,"stdout_truncated":outcome.stdout_truncated,
        "stderr_tail":outcome.stderr_tail,"stderr_truncated":outcome.stderr_truncated});
    let build_result = if receipt_directory {
        work_operations::execution::command_run::build_command_result_with_receipts
    } else {
        build_command_result
    };
    CommandCompletion {
        finished: build_result(preview, &process, false),
        response: build_result(preview, &process, true),
        failed: outcome.status != CommandStatus::Exited || outcome.exit_code != Some(0),
    }
}

pub fn approved_receipt_paths(
    preview: &Value,
    execution_dir: &str,
    approved_sha256: &str,
) -> Result<work_operations::derivation::publication::CommandReceiptPaths, WorkError> {
    work_operations::execution::command_run::validate_command_receipt_preview(
        preview,
        execution_dir,
        approved_sha256,
    )
    .map_err(|issue| {
        WorkError::new(
            match issue.reason_code {
                "command_run_receipt_invalid" => ExitCode::ArtifactIntegrity,
                "command_run_preview_invalid" => ExitCode::Contract,
                _ => ExitCode::WorkflowState,
            },
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })
}

pub fn approved_receipt<'a>(
    preview: &'a Value,
    execution_dir: &str,
    approved_sha256: &str,
) -> Result<&'a str, WorkError> {
    let mut unsigned = preview.clone();
    unsigned
        .as_object_mut()
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::Contract,
                "command_run_preview_invalid",
                "The command preview is invalid.",
                json!({}),
            )
        })?
        .remove("approved_sha256");
    if command_preview_approval_sha256(&unsigned) != approved_sha256
        || preview["approved_sha256"] != approved_sha256
    {
        return Err(WorkError::new(
            ExitCode::WorkflowState,
            "command_run_approval_changed",
            "Sources or command parameters changed after review.",
            json!({}),
        ));
    }
    validate_command_run_request(&preview["request"]).map_err(|issue| {
        WorkError::new(
            ExitCode::WorkflowState,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let receipt = preview["receipt_prefix"].as_str().ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "command_run_receipt_invalid",
            "The command receipt path is invalid.",
            json!({}),
        )
    })?;
    let record_id = preview["record_id"].as_str().unwrap_or("");
    let task_id = preview["task_id"].as_str().unwrap_or("");
    let attempt_id = preview["attempt_id"].as_str().unwrap_or("");
    let expected = work_operations::derivation::legacy_layout::command_receipt_prefix(
        execution_dir,
        task_id,
        attempt_id,
        record_id,
    );
    if receipt != expected {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "command_run_receipt_invalid",
            "The command receipt path is invalid.",
            json!({"expected":expected,"actual":receipt}),
        ));
    }
    Ok(receipt)
}

fn invocation_invalid() -> WorkError {
    WorkError::new(
        ExitCode::Contract,
        "command_run_invocation_invalid",
        "The command invocation is invalid.",
        json!({}),
    )
}

pub fn build_command_request(preview: &Value) -> Result<CommandRequest, WorkError> {
    let invocation = &preview["invocation"];
    let argv: Vec<String> = match invocation["kind"].as_str() {
        Some("direct") => {
            let executable = invocation["executable"].as_str().unwrap_or("");
            let arguments = invocation["argv"]
                .as_array()
                .filter(|values| !values.is_empty() && values.iter().all(Value::is_string))
                .ok_or_else(invocation_invalid)?;
            std::iter::once(executable.to_owned())
                .chain(
                    arguments
                        .iter()
                        .skip(1)
                        .map(|value| value.as_str().expect("checked string"))
                        .map(str::to_owned),
                )
                .collect()
        }
        Some("windows_batch") => {
            let launcher = invocation["launcher"].as_str().unwrap_or("");
            let arguments = invocation["launcher_arguments"]
                .as_array()
                .filter(|values| values.iter().all(Value::is_string))
                .ok_or_else(invocation_invalid)?;
            std::iter::once(launcher.to_owned())
                .chain(
                    arguments
                        .iter()
                        .map(|value| value.as_str().expect("checked string"))
                        .map(str::to_owned),
                )
                .collect()
        }
        _ => return Err(invocation_invalid()),
    };
    if argv.first().is_none_or(String::is_empty) {
        return Err(invocation_invalid());
    }
    let cwd = preview["working_directory"].as_str().ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "command_run_cwd",
            "The command working directory is invalid.",
            json!({}),
        )
    })?;
    let timeout = preview["request"]["timeout_seconds"].as_u64().unwrap_or(0);
    Ok(CommandRequest {
        argv,
        cwd: PathBuf::from(cwd),
        timeout: Duration::from_secs(timeout),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_directory_completion_preserves_failure_and_never_invents_legacy_alias() {
        let preview = json!({"approved_sha256":"a".repeat(64),"record_id":"CMD-001#2",
            "receipt_dir":"custom/TASK-001/ATTEMPT-002/receipts/CMD-001-retry-2"});
        for (status, exit_code, failed) in [
            (CommandStatus::Exited, Some(0), false),
            (CommandStatus::Exited, Some(7), true),
            (CommandStatus::TimedOut, None, true),
            (CommandStatus::LaunchFailed, None, true),
        ] {
            let outcome = CommandOutcome {
                status,
                exit_code,
                stdout_tail: "observed output".into(),
                stdout_truncated: false,
                stderr_tail: "observed error".into(),
                stderr_truncated: true,
            };
            let completion = command_completion_with_receipts(&preview, &outcome);
            assert!(completion.finished.get("receipt_dir").is_none());
            assert!(completion.finished.get("receipt_prefix").is_none());
            let response = if failed {
                let error = completion.into_result().unwrap_err();
                assert_eq!(error.reason_code, "command_run_failed");
                error.details
            } else {
                completion.into_result().unwrap()
            };
            assert_eq!(response["receipt_dir"], preview["receipt_dir"]);
            assert!(response.get("receipt_prefix").is_none());
            assert_eq!(response["exit_code"], json!(exit_code));
            assert_eq!(response["stderr_truncated"], true);
            assert_eq!(response["record_finish_required"], true);
            if status != CommandStatus::Exited {
                assert!(response.get("record_finish_request").is_none());
            }
        }
    }

    #[test]
    fn non_object_preview_is_rejected_before_approval() {
        let error = approved_receipt(&Value::Null, "outputs/work/e", "")
            .expect_err("invalid preview must fail");
        assert_eq!(error.reason_code, "command_run_preview_invalid");
    }

    #[test]
    fn direct_invocation_preserves_executable_and_arguments() {
        let request = build_command_request(&json!({"invocation":{"kind":"direct",
            "executable":"/bin/echo","argv":["echo","hello"]},
            "working_directory":".","request":{"timeout_seconds":3}}))
        .unwrap();
        assert_eq!(request.argv, ["/bin/echo", "hello"]);
        assert_eq!(request.timeout, Duration::from_secs(3));
    }

    #[test]
    fn failed_command_result_requires_review() {
        let completion = command_completion(
            &json!({}),
            &CommandOutcome {
                status: CommandStatus::Exited,
                exit_code: Some(1),
                stdout_tail: String::new(),
                stdout_truncated: false,
                stderr_tail: String::new(),
                stderr_truncated: false,
            },
        );
        assert_eq!(
            completion
                .into_result()
                .expect_err("failure must stop")
                .reason_code,
            "command_run_failed"
        );
    }
}
