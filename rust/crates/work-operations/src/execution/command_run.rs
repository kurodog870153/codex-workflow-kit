//! Pure request and approval fingerprint rules for command execution.

use std::fmt::Write;

use serde_json::{Value, json};

use crate::canonical::sha256_hex;
use crate::derivation::identity::next_record_id;
use crate::execution::ExecutionIssue;
use crate::execution::authorization::{effective_task, require_record_scope};
use crate::execution::command_correction::validate_command_correction;

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details,
    }
}

pub fn validate_command_run_request(request: &Value) -> Result<(), ExecutionIssue> {
    let object = request.as_object().ok_or_else(|| {
        issue(
            "expected_object",
            "A JSON object is required.",
            json!({"location":"command_run"}),
        )
    })?;
    let missing: Vec<_> = ["schema", "timeout_seconds"]
        .into_iter()
        .filter(|field| !object.contains_key(*field))
        .collect();
    let unknown: Vec<_> = object
        .keys()
        .filter(|field| field.as_str() != "schema" && field.as_str() != "timeout_seconds")
        .collect();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(issue(
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location":"command_run","missing":missing,"unknown":unknown}),
        ));
    }
    if request["schema"] != "work-command-run-request" {
        return Err(issue(
            "command_run_schema",
            "Use work-command-run-request.",
            json!({}),
        ));
    }
    if request["timeout_seconds"]
        .as_i64()
        .is_none_or(|seconds| !(1..=3600).contains(&seconds))
    {
        return Err(issue(
            "command_run_timeout",
            "timeout_seconds must be an integer from 1 to 3600.",
            json!({}),
        ));
    }
    work_model::execution::request::verified::<work_model::execution::request::CommandRunRequest>(
        request,
    );
    Ok(())
}

pub struct ReservedCommand {
    pub attempt_id: String,
    pub record_id: String,
    pub base_record_id: String,
    pub command: Value,
}

pub fn select_reserved_argv_command(
    task: &Value,
    attempt: &Value,
    index: &Value,
    task_id: &str,
) -> Result<ReservedCommand, ExecutionIssue> {
    let lock = &index["lock"];
    let attempt_id = lock["attempt_id"].as_str().ok_or_else(|| {
        issue(
            "command_run_lock",
            "The active execution lock must reserve a CMD record.",
            json!({}),
        )
    })?;
    let record_id = lock["record_id"].as_str().ok_or_else(|| {
        issue(
            "command_run_lock",
            "The active execution lock must reserve a CMD record.",
            json!({}),
        )
    })?;
    let row = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["id"] == task_id);
    if lock["kind"] != "execution"
        || lock["task_id"] != task_id
        || lock["execute_instructions_sha256"] != attempt["execute_instructions_sha256"]
        || attempt["attempt_id"] != attempt_id
        || attempt["status"] != "in_progress"
        || row
            .is_none_or(|row| row["status"] != "in_progress" || row["latest_attempt"] != attempt_id)
    {
        return Err(issue(
            "command_run_lock",
            "The active Attempt and lock must reserve exactly one CMD.",
            json!({}),
        ));
    }
    let base = record_id.split('#').next().unwrap_or(record_id);
    if next_record_id(base, attempt)? != record_id {
        return Err(issue(
            "command_run_sequence",
            "The reserved CMD is not the next record instance.",
            json!({}),
        ));
    }
    require_record_scope(attempt, base)?;
    let effective = effective_task(task, attempt)?;
    let formal = effective["commands"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|command| command["id"] == base)
        .ok_or_else(|| {
            issue(
                "command_correction_command_not_found",
                "The reserved command is not defined by the target TASK.",
                json!({"record_id":base}),
            )
        })?;
    let mut command = json!({"mode":formal["mode"]});
    let field = if formal["mode"] == "argv" {
        "argv"
    } else {
        "script"
    };
    command[field] = formal[field].clone();
    if let Some(correction) = lock.get("command_correction") {
        validate_command_correction(correction, "command_correction")?;
        if correction["original_command"] != command {
            return Err(issue(
                "command_run_correction",
                "The saved correction differs from the formal CMD.",
                json!({}),
            ));
        }
        command = correction["actual_command"].clone();
    }
    if command["mode"] != "argv" {
        return Err(issue(
            "command_run_argv_only",
            "This executor supports argv CMDs only.",
            json!({}),
        ));
    }
    let argv = command["argv"].as_array().ok_or_else(|| {
        issue(
            "command_run_argv",
            "argv must be a nonempty string array.",
            json!({}),
        )
    })?;
    if argv.is_empty()
        || argv.iter().any(|argument| argument.as_str().is_none())
        || argv[0].as_str() == Some("")
    {
        return Err(issue(
            "command_run_argv",
            "argv must be a nonempty string array.",
            json!({}),
        ));
    }
    if argv
        .iter()
        .any(|argument| argument.as_str().is_some_and(|value| value.contains('\0')))
    {
        return Err(issue(
            "command_run_argv",
            "argv cannot contain NUL bytes.",
            json!({}),
        ));
    }
    Ok(ReservedCommand {
        attempt_id: attempt_id.into(),
        record_id: record_id.into(),
        base_record_id: base.into(),
        command,
    })
}

pub fn command_json_bytes(value: &Value) -> Vec<u8> {
    let rendered = serde_json::to_string_pretty(value).expect("JSON value serializes");
    let mut ascii = String::with_capacity(rendered.len() + 1);
    for character in rendered.chars() {
        if character.is_ascii() {
            ascii.push(character);
        } else {
            for unit in character.encode_utf16(&mut [0_u16; 2]).iter() {
                write!(&mut ascii, "\\u{unit:04x}").expect("writing to String cannot fail");
            }
        }
    }
    ascii.push('\n');
    ascii.into_bytes()
}

pub fn command_preview_approval_sha256(preview_without_approval: &Value) -> String {
    sha256_hex(&command_json_bytes(preview_without_approval))
}

pub struct CommandPreviewInput<'a> {
    pub request: &'a Value,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub record_id: &'a str,
    pub working_directory: &'a str,
    pub execution: &'a Value,
    pub invocation: &'a Value,
    pub receipt_prefix: &'a str,
    pub sources: &'a Value,
}

pub fn build_command_preview(input: CommandPreviewInput<'_>) -> Result<Value, ExecutionIssue> {
    validate_command_run_request(input.request)?;
    let mut preview = json!({"schema":"work-command-preview",
        "request":input.request,"task_id":input.task_id,
        "attempt_id":input.attempt_id,"record_id":input.record_id,
        "working_directory":input.working_directory,
        "execution":input.execution,"invocation":input.invocation,
        "receipt_prefix":input.receipt_prefix,"sources":input.sources});
    preview["approved_sha256"] = json!(command_preview_approval_sha256(&preview));
    Ok(work_model::execution::response::verified::<
        work_model::execution::response::LegacyCommandPreview,
    >(preview))
}

pub struct CommandReceiptPreviewInput<'a> {
    pub request: &'a Value,
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub record_id: &'a str,
    pub working_directory: &'a str,
    pub execution: &'a Value,
    pub invocation: &'a Value,
    pub sources: &'a Value,
}

pub fn build_command_preview_with_receipts(
    input: CommandReceiptPreviewInput<'_>,
) -> Result<Value, ExecutionIssue> {
    validate_command_run_request(input.request)?;
    let paths = crate::derivation::publication::command_receipt_paths(
        input.execution_dir,
        input.task_id,
        input.attempt_id,
        input.record_id,
    )
    .map_err(|_| {
        issue(
            "command_run_receipt_invalid",
            "The command receipt identity is invalid.",
            json!({}),
        )
    })?;
    let mut preview = json!({"schema":"work-command-preview",
        "request":input.request,"task_id":input.task_id,"attempt_id":input.attempt_id,
        "record_id":input.record_id,"working_directory":input.working_directory,
        "execution":input.execution,"invocation":input.invocation,
        "receipt_dir":paths.directory,"sources":input.sources});
    preview["approved_sha256"] = json!(command_preview_approval_sha256(&preview));
    serde_json::from_value::<work_model::execution::response::PreparedCommandPreview>(
        preview.clone(),
    )
    .map_err(|_| {
        issue(
            "command_run_preview_invalid",
            "The prepared command preview is invalid.",
            json!({}),
        )
    })?;
    Ok(preview)
}

pub fn validate_command_receipt_preview(
    preview: &Value,
    execution_dir: &str,
    approval: &str,
) -> Result<crate::derivation::publication::CommandReceiptPaths, ExecutionIssue> {
    let typed: work_model::execution::response::PreparedCommandPreview =
        serde_json::from_value(preview.clone()).map_err(|_| {
            issue(
                "command_run_preview_invalid",
                "The command preview requires the receipt-directory contract.",
                json!({}),
            )
        })?;
    if preview["schema"] != "work-command-preview" {
        return Err(issue(
            "command_run_preview_invalid",
            "The command preview schema is invalid.",
            json!({}),
        ));
    }
    validate_command_run_request(&preview["request"])?;
    let mut unsigned = preview.clone();
    unsigned
        .as_object_mut()
        .expect("typed preview is an object")
        .remove("approved_sha256");
    if preview["approved_sha256"] != approval
        || command_preview_approval_sha256(&unsigned) != approval
    {
        return Err(issue(
            "command_run_approval_changed",
            "The complete command preview changed after review.",
            json!({}),
        ));
    }
    let paths = crate::derivation::publication::command_receipt_paths(
        execution_dir,
        &typed.task_id,
        &typed.attempt_id,
        &typed.record_id,
    )
    .map_err(|_| {
        issue(
            "command_run_receipt_invalid",
            "The command receipt identity is invalid.",
            json!({}),
        )
    })?;
    if paths.directory != typed.receipt_dir {
        return Err(issue(
            "command_run_receipt_invalid",
            "The command receipt directory differs from the approved instance.",
            json!({"expected":paths.directory,"actual":typed.receipt_dir}),
        ));
    }
    Ok(paths)
}

pub fn build_command_started(preview: &Value, authorization_evidence: &str) -> Value {
    json!({"schema":"work-command-started","preview":preview,
        "authorization_evidence":authorization_evidence})
}

fn command_result_fields(preview: &Value, process: &Value, with_receipt: bool) -> Value {
    let mut result = json!({"schema":"work-command-result",
        "approved_sha256":preview["approved_sha256"],
        "record_id":preview["record_id"],
        "status":process["status"],"exit_code":process["exit_code"],
        "stdout_tail":process["stdout_tail"],
        "stdout_truncated":process["stdout_truncated"],
        "stderr_tail":process["stderr_tail"],
        "stderr_truncated":process["stderr_truncated"]});
    if with_receipt {
        result["record_finish_required"] = json!(true);
        if process["status"] == "exited" {
            let code = process["exit_code"].as_i64().unwrap_or(0);
            result["record_finish_request"] = json!({"schema":"work-record-finish-request",
                "record":{"exit_code":code,
                    "result":format!("Command exited with code {code}; inspect retained execution evidence.")}});
        }
    }
    result
}

pub fn build_command_result(preview: &Value, process: &Value, with_receipt: bool) -> Value {
    let mut result = command_result_fields(preview, process, with_receipt);
    if with_receipt {
        result["receipt_prefix"] = preview["receipt_prefix"].clone();
    }
    if preview["approved_sha256"].is_string() && preview["record_id"].is_string() {
        work_model::execution::response::verified::<
            work_model::execution::response::LegacyCommandResult,
        >(result)
    } else {
        result
    }
}

pub fn build_command_result_with_receipts(
    preview: &Value,
    process: &Value,
    with_receipt: bool,
) -> Value {
    let mut result = command_result_fields(preview, process, with_receipt);
    if with_receipt {
        result["receipt_dir"] = preview["receipt_dir"].clone();
    }
    work_model::execution::response::verified::<
        work_model::execution::response::PreparedCommandResult,
    >(result)
}

pub fn quote_windows_batch_argument(argument: &str) -> Result<String, ExecutionIssue> {
    if argument.contains(['\0', '\r', '\n']) {
        return Err(issue(
            "command_run_batch_argument",
            "Batch arguments cannot contain NUL or line breaks.",
            json!({}),
        ));
    }
    Ok(format!(
        "\"{}\"",
        argument.replace('%', "%%").replace('"', "\"\"")
    ))
}

pub fn windows_batch_command_line(
    script: &str,
    arguments: &[String],
) -> Result<(String, Vec<String>), ExecutionIssue> {
    let quoted = std::iter::once(script)
        .chain(arguments.iter().map(String::as_str))
        .map(quote_windows_batch_argument)
        .collect::<Result<Vec<_>, _>>()?;
    let command_line = quoted.join(" ");
    Ok((
        command_line.clone(),
        vec![
            "/d".into(),
            "/s".into(),
            "/v:off".into(),
            "/c".into(),
            command_line,
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_directory_preview_binds_full_identity_and_invalidates_legacy_approval() {
        let request = json!({"schema":"work-command-run-request","timeout_seconds":60});
        let execution = json!({"os":"macos","working_directory":"."});
        let invocation = json!({"kind":"direct","executable":"/usr/bin/printf","executable_sha256":"a".repeat(64),"argv":["printf","中文"]});
        let sources = json!({"task/index.json":"b".repeat(64)});
        let preview = build_command_preview_with_receipts(CommandReceiptPreviewInput {
            request: &request,
            execution_dir: "自訂 空白/執行",
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-002",
            record_id: "CMD-001#2",
            working_directory: "/project",
            execution: &execution,
            invocation: &invocation,
            sources: &sources,
        })
        .unwrap();
        let approval = preview["approved_sha256"].as_str().unwrap();
        let paths = validate_command_receipt_preview(&preview, "自訂 空白/執行", approval).unwrap();
        assert_eq!(
            preview["receipt_dir"],
            "自訂 空白/執行/TASK-001/ATTEMPT-002/receipts/CMD-001-retry-2"
        );
        assert!(preview.get("receipt_prefix").is_none());
        assert_eq!(paths.finished, format!("{}/finished.json", paths.directory));
        let mut unsigned = preview.clone();
        unsigned.as_object_mut().unwrap().remove("approved_sha256");
        assert_eq!(command_preview_approval_sha256(&unsigned), approval);
        assert_eq!(
            validate_command_receipt_preview(&preview, "other/execution", approval)
                .unwrap_err()
                .reason_code,
            "command_run_receipt_invalid"
        );
        let mut changed = preview.clone();
        changed["sources"]["task/index.json"] = json!("c".repeat(64));
        assert_eq!(
            validate_command_receipt_preview(&changed, "自訂 空白/執行", approval)
                .unwrap_err()
                .reason_code,
            "command_run_approval_changed"
        );
        let legacy = build_command_preview(CommandPreviewInput {
            request: &request,
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-002",
            record_id: "CMD-001#2",
            working_directory: "/project",
            execution: &execution,
            invocation: &invocation,
            receipt_prefix: paths.legacy_started.trim_end_matches(".started.json"),
            sources: &sources,
        })
        .unwrap();
        assert_ne!(preview["approved_sha256"], legacy["approved_sha256"]);
        assert!(
            validate_command_receipt_preview(
                &legacy,
                "自訂 空白/執行",
                legacy["approved_sha256"].as_str().unwrap()
            )
            .is_err()
        );
        let process = json!({"status":"exited","exit_code":7,"stdout_tail":"failed","stdout_truncated":false,"stderr_tail":"","stderr_truncated":false});
        let without_receipt = build_command_result_with_receipts(&preview, &process, false);
        assert!(without_receipt.get("receipt_dir").is_none());
        let result = build_command_result_with_receipts(&preview, &process, true);
        assert_eq!(result["receipt_dir"], preview["receipt_dir"]);
        assert!(result.get("receipt_prefix").is_none());
        assert_eq!(result["record_finish_request"]["record"]["exit_code"], 7);
    }

    #[test]
    fn reserved_command_keeps_scope_sequence_and_argv_boundary() {
        let task = json!({"commands":[{"id":"CMD-001","mode":"argv",
            "argv":["tool","--check"]}]});
        let attempt = json!({"attempt_id":"ATTEMPT-001","status":"in_progress",
            "execute_instructions_sha256":"a".repeat(64),"records":[],
            "authorization":{"commands":[{"id":"CMD-001"}]}});
        let mut index = json!({"tasks":[{"id":"TASK-001","status":"in_progress",
            "latest_attempt":"ATTEMPT-001"}],"lock":{"kind":"execution",
            "task_id":"TASK-001","attempt_id":"ATTEMPT-001","record_id":"CMD-001",
            "execute_instructions_sha256":"a".repeat(64)}});
        let mut selected =
            select_reserved_argv_command(&task, &attempt, &index, "TASK-001").unwrap();
        assert_eq!(selected.command["argv"], json!(["tool", "--check"]));
        assert_eq!(selected.record_id, "CMD-001");
        selected.command["argv"]
            .as_array_mut()
            .unwrap()
            .push(json!("changed"));
        assert_eq!(task["commands"][0]["argv"], json!(["tool", "--check"]));
        assert_eq!(selected.command["mode"], "argv");
        assert_eq!(
            select_reserved_argv_command(&json!({"commands":[]}), &attempt, &index, "TASK-001")
                .err()
                .unwrap()
                .reason_code,
            "command_correction_command_not_found"
        );
        index["lock"]["record_id"] = json!("CMD-001#2");
        assert_eq!(
            select_reserved_argv_command(&task, &attempt, &index, "TASK-001")
                .err()
                .unwrap()
                .reason_code,
            "command_run_sequence"
        );
        index["lock"]["record_id"] = json!("VAL-001");
        assert!(select_reserved_argv_command(&task, &attempt, &index, "TASK-001").is_err());
        index["lock"]["record_id"] = json!("CMD-001");
        index["lock"]["command_correction"] = json!({
            "original_command":{"mode":"argv","argv":["tool","--check"]},
            "actual_command":{"mode":"argv","argv":["tool","--fix"]},
            "reason":"Corrected argument","authorization_evidence":"Approved correction"});
        let corrected = select_reserved_argv_command(&task, &attempt, &index, "TASK-001").unwrap();
        assert_eq!(corrected.command["argv"], json!(["tool", "--fix"]));
    }

    #[test]
    fn request_rejects_invalid_fields_schema_and_timeout() {
        assert_eq!(
            validate_command_run_request(&json!({}))
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
        assert_eq!(
            validate_command_run_request(&json!({
                "schema":"work-command-run-request",
                "timeout_seconds":60,
                "record_id":"CMD-001"
            }))
            .unwrap_err()
            .reason_code,
            "invalid_object_fields"
        );
        assert_eq!(
            validate_command_run_request(&json!({"schema":"bad",
            "timeout_seconds":60}))
            .unwrap_err()
            .reason_code,
            "command_run_schema"
        );
        assert_eq!(
            validate_command_run_request(&json!({"schema":"work-command-run-request",
            "timeout_seconds":0}))
            .unwrap_err()
            .reason_code,
            "command_run_timeout"
        );
        validate_command_run_request(&json!({"schema":"work-command-run-request",
            "timeout_seconds":60}))
        .unwrap();
    }

    #[test]
    fn approval_hash_matches_current_contract_ascii_sorted_pretty_json() {
        let preview = json!({"schema":"work-command-preview",
            "request":{"schema":"work-command-run-request","timeout_seconds":60},
            "sources":{"任務/ß.json":"a".repeat(64)},"record_id":"CMD-001",
            "argv":["print","中文😀"]});
        assert_eq!(
            command_preview_approval_sha256(&preview),
            "d6dec18302ff23dae5a29fc4078e508cda27c4e72d6a0881e492407cf09613f7"
        );
        let raw = String::from_utf8(command_json_bytes(&preview)).unwrap();
        assert!(raw.contains("\\u4e2d\\u6587\\ud83d\\ude00"));
    }

    #[test]
    fn full_preview_approval_matches_current_contract_example() {
        let request = json!({"schema":"work-command-run-request","timeout_seconds":60});
        let execution = json!({"os":"macos","working_directory":"."});
        let invocation = json!({"kind":"direct","executable":"/usr/bin/printf",
            "executable_sha256":"a".repeat(64),"argv":["printf","中文"]});
        let sources = json!({"task/index.json":"b".repeat(64)});
        let preview = build_command_preview(CommandPreviewInput {
            request: &request,
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            record_id: "CMD-001",
            working_directory: "/project",
            execution: &execution,
            invocation: &invocation,
            receipt_prefix: "execution/TASK-001/ATTEMPT-001/.work-command-CMD-001",
            sources: &sources,
        })
        .unwrap();
        assert_eq!(
            preview["approved_sha256"],
            "796eefe879904280bc3564eca0eed98675b0573ad3a3cdc121ee126e67acacec"
        );
    }

    #[test]
    fn windows_batch_preview_keeps_discriminated_invocation() {
        let request = json!({"schema":"work-command-run-request","timeout_seconds":60});
        let execution = json!({"os":"windows","working_directory":"."});
        let invocation = json!({"kind":"windows_batch",
            "launcher":"C:/Windows/System32/cmd.exe","launcher_sha256":"1".repeat(64),
            "script":"C:/tools/test.cmd","script_sha256":"2".repeat(64),
            "arguments":["two words"],"command_line":"\"C:/tools/test.cmd\" \"two words\"",
            "launcher_arguments":["/d","/s","/v:off","/c","\"C:/tools/test.cmd\" \"two words\""]});
        let sources = json!({"task/index.json":"b".repeat(64)});
        let preview = build_command_preview(CommandPreviewInput {
            request: &request,
            task_id: "TASK-001",
            attempt_id: "ATTEMPT-001",
            record_id: "CMD-001",
            working_directory: "C:/project",
            execution: &execution,
            invocation: &invocation,
            receipt_prefix: "execution/TASK-001/ATTEMPT-001/.work-command-CMD-001",
            sources: &sources,
        })
        .unwrap();
        assert_eq!(preview["invocation"], invocation);
        assert_eq!(preview["invocation"]["kind"], "windows_batch");
        assert!(
            preview["approved_sha256"]
                .as_str()
                .is_some_and(|sha| sha.len() == 64)
        );
    }

    #[test]
    fn result_requires_finish_and_preserves_nonzero_exit() {
        let preview = json!({"approved_sha256":"a".repeat(64),"record_id":"CMD-001",
            "receipt_prefix":"execution/TASK-001/ATTEMPT-001/.work-command-CMD-001"});
        let process = json!({"status":"exited","exit_code":7,
            "stdout_tail":"failed","stdout_truncated":false,
            "stderr_tail":"","stderr_truncated":false});
        let receipt = build_command_result(&preview, &process, false);
        assert!(receipt.get("receipt_prefix").is_none());
        let response = build_command_result(&preview, &process, true);
        assert_eq!(response["record_finish_required"], true);
        assert_eq!(response["record_finish_request"]["record"]["exit_code"], 7);
        assert_eq!(
            response["record_finish_request"]["record"]["result"],
            "Command exited with code 7; inspect retained execution evidence."
        );
        let mut timeout = process;
        timeout["status"] = json!("timed_out");
        timeout["exit_code"] = Value::Null;
        assert!(
            build_command_result(&preview, &timeout, true)
                .get("record_finish_request")
                .is_none()
        );
    }

    #[test]
    fn windows_batch_quoting_matches_current_contract_without_running_windows() {
        let (line, argv) = windows_batch_command_line(
            "C:\\tools\\run.cmd",
            &["one two".into(), "100% \"ready\"".into()],
        )
        .unwrap();
        assert_eq!(
            line,
            "\"C:\\tools\\run.cmd\" \"one two\" \"100%% \"\"ready\"\"\""
        );
        assert_eq!(argv, ["/d", "/s", "/v:off", "/c", line.as_str()]);
        assert_eq!(
            quote_windows_batch_argument("line\nbreak")
                .unwrap_err()
                .reason_code,
            "command_run_batch_argument"
        );
    }
}
