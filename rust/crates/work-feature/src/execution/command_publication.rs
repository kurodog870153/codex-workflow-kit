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
    let status = match outcome.status {
        CommandStatus::Exited => "exited",
        CommandStatus::TimedOut => "timed_out",
        CommandStatus::LaunchFailed => "launch_failed",
    };
    let process = json!({"status":status,"exit_code":outcome.exit_code,
        "stdout_tail":outcome.stdout_tail,"stdout_truncated":outcome.stdout_truncated,
        "stderr_tail":outcome.stderr_tail,"stderr_truncated":outcome.stderr_truncated});
    CommandCompletion {
        finished: build_command_result(preview, &process, false),
        response: build_command_result(preview, &process, true),
        failed: outcome.status != CommandStatus::Exited || outcome.exit_code != Some(0),
    }
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
    let expected = format!(
        "{execution_dir}/{task_id}/{attempt_id}/.work-command-{}",
        record_id.replace('#', "-retry-")
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
