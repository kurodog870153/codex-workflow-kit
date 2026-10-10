//! Native execution boundaries; public production execution never falls back to a host runner.
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::{
    CommandOutcome, CommandRequest, CommandRunner, RequirementWriterContext,
};
use work_model::execution::{CommandIsolation, VerifiedEffectBoundary};
use work_operations::derivation::fingerprint;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

fn unavailable() -> WorkError {
    WorkError::new(
        ExitCode::WorkflowState,
        "command_isolation_unavailable",
        "The native execution boundary cannot be enforced; no host command was launched.",
        json!({}),
    )
}
fn backend() -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    if Path::new("/usr/bin/sandbox-exec").is_file() {
        return Some("macos_seatbelt_v1");
    }
    #[cfg(windows)]
    if windows::available() {
        return Some("windows_lpac_job_v1");
    }
    None
}
pub(crate) fn record_boundary(task: &Value, record: &str) -> VerifiedEffectBoundary {
    if backend().is_some()
        && record.starts_with("CMD-")
        && task["commands"].as_array().is_some_and(|commands| {
            commands
                .iter()
                .any(|c| c["id"] == record && c["mode"] == "argv")
        })
    {
        VerifiedEffectBoundary::Isolated
    } else {
        VerifiedEffectBoundary::Unknown
    }
}
pub(crate) fn policy(
    context: &RequirementWriterContext,
    receipt: &str,
) -> Result<CommandIsolation, WorkError> {
    let backend = backend().ok_or_else(unavailable)?;
    if !work_operations::derivation::identity::runtime_relative_path(receipt) {
        return Err(unavailable());
    }
    let relative = format!(
        "outputs/work/transactions/{}/command-isolation/{}/staging",
        context.requirement_id.as_str(),
        fingerprint::raw(receipt.as_bytes())
    );
    let root = context
        .canonical_project_root
        .canonicalize()
        .map_err(|_| unavailable())?;
    let writable = crate::files::resolve_runtime_path(&root, &relative)?;
    let view = if backend == "windows_lpac_job_v1" {
        writable
            .parent()
            .ok_or_else(unavailable)?
            .join("project-read-only")
    } else {
        root.clone()
    };
    Ok(CommandIsolation {
        backend: backend.into(),
        read_only_project_root: root.to_string_lossy().into_owned(),
        read_only_project_view: view.to_string_lossy().into_owned(),
        writable_directory: writable.to_string_lossy().into_owned(),
        network_access: false,
    })
}
fn validate(
    context: &RequirementWriterContext,
    policy: &CommandIsolation,
) -> Result<(), WorkError> {
    let root = context
        .canonical_project_root
        .canonicalize()
        .map_err(|_| unavailable())?;
    let writable = Path::new(&policy.writable_directory);
    let view = if policy.backend == "windows_lpac_job_v1" {
        writable
            .parent()
            .ok_or_else(unavailable)?
            .join("project-read-only")
    } else {
        root.clone()
    };
    let prefix = root.join(format!(
        "outputs/work/transactions/{}/command-isolation",
        context.requirement_id.as_str()
    ));
    let relative = writable.strip_prefix(&prefix).map_err(|_| unavailable())?;
    let components: Vec<_> = relative.components().collect();
    let identity = relative
        .parent()
        .and_then(Path::file_name)
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if policy.network_access
        || Some(policy.backend.as_str()) != backend()
        || Path::new(&policy.read_only_project_root) != root
        || Path::new(&policy.read_only_project_view) != view
        || components.len() != 2
        || relative.file_name().is_none_or(|name| name != "staging")
        || identity.len() != 64
        || !identity
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(unavailable());
    }
    let safe = crate::files::resolve_runtime_path(
        &root,
        writable
            .strip_prefix(&root)
            .map_err(|_| unavailable())?
            .to_str()
            .ok_or_else(unavailable)?,
    )?;
    if safe != writable {
        return Err(unavailable());
    }
    Ok(())
}
pub(crate) fn invocation_boundary(
    context: &RequirementWriterContext,
    actual: &Value,
) -> VerifiedEffectBoundary {
    let policy =
        serde_json::from_value::<CommandIsolation>(actual["execution"]["work_isolation"].clone());
    if policy.is_ok_and(|policy| validate(context, &policy).is_ok())
        && matches!(
            actual["invocation"]["kind"].as_str(),
            Some("direct" | "windows_batch")
        )
        && Path::new(actual["working_directory"].as_str().unwrap_or(""))
            .starts_with(&context.canonical_project_root)
    {
        VerifiedEffectBoundary::Isolated
    } else {
        VerifiedEffectBoundary::Unknown
    }
}

pub(crate) struct IsolatedCommandRunner {
    context: RequirementWriterContext,
    approved: RefCell<Option<(CommandIsolation, CommandRequest)>>,
}
impl IsolatedCommandRunner {
    pub(crate) fn new(context: RequirementWriterContext) -> Self {
        Self {
            context,
            approved: RefCell::new(None),
        }
    }
    pub(crate) fn bind(&self, preview: &Value) -> Result<(), WorkError> {
        let observed: CommandIsolation =
            serde_json::from_value(preview["execution"]["work_isolation"].clone())
                .map_err(|_| unavailable())?;
        let expected = policy(
            &self.context,
            preview["receipt_dir"].as_str().ok_or_else(unavailable)?,
        )?;
        if observed != expected {
            return Err(unavailable());
        }
        validate(&self.context, &observed)?;
        let request = work_feature::execution::command_publication::build_command_request(preview)?;
        *self.approved.borrow_mut() = Some((observed, request));
        Ok(())
    }
}
impl CommandRunner for IsolatedCommandRunner {
    fn run(&self, request: &CommandRequest) -> CommandOutcome {
        let run = || -> Result<super::ProcessCapture, WorkError> {
            let (policy, approved) = self.approved.borrow_mut().take().ok_or_else(unavailable)?;
            validate(&self.context, &policy)?;
            if request.argv != approved.argv
                || request.cwd != approved.cwd
                || request.timeout != approved.timeout
                || !request
                    .cwd
                    .starts_with(&self.context.canonical_project_root)
                || request.argv.is_empty()
            {
                return Err(unavailable());
            }
            let writable = PathBuf::from(&policy.writable_directory);
            // A previous sandbox allocation is retained evidence, never silently reused.
            if writable.parent().is_none_or(|p| p.exists()) {
                return Err(unavailable());
            }
            std::fs::create_dir_all(writable.parent().unwrap().parent().unwrap())
                .map_err(|_| unavailable())?;
            std::fs::create_dir(writable.parent().unwrap()).map_err(|_| unavailable())?;
            std::fs::create_dir(&writable).map_err(|_| unavailable())?;
            #[cfg(target_os = "macos")]
            return macos::run(request, &policy);
            #[cfg(windows)]
            return windows::run(request, &policy);
            #[cfg(not(any(target_os = "macos", windows)))]
            Err(unavailable())
        };
        let capture = run().unwrap_or_else(|_| super::launch_failed());
        CommandOutcome {
            status: capture.status,
            exit_code: capture.exit_code,
            stdout_tail: String::from_utf8_lossy(&capture.stdout).into_owned(),
            stdout_truncated: capture.stdout_truncated,
            stderr_tail: String::from_utf8_lossy(&capture.stderr).into_owned(),
            stderr_truncated: capture.stderr_truncated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn context() -> RequirementWriterContext {
        static NEXT_CONTEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "work-isolation-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_CONTEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        RequirementWriterContext {
            canonical_project_root: root.canonicalize().unwrap(),
            requirement_id: "example".parse().unwrap(),
        }
    }
    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn executor_policy_is_bound_to_receipt_and_rejects_caller_widening() {
        let context = context();
        let mut policy = policy(
            &context,
            "outputs/work/e/TASK-001/ATTEMPT-001/receipts/CMD-001",
        )
        .unwrap();
        validate(&context, &policy).unwrap();
        policy.network_access = true;
        assert!(validate(&context, &policy).is_err());
        policy.network_access = false;
        policy.writable_directory = context
            .canonical_project_root
            .to_string_lossy()
            .into_owned();
        assert!(validate(&context, &policy).is_err());
        let task = json!({"commands":[{"id":"CMD-001","mode":"argv"}], "work_isolation":{"network_access":true}});
        assert_eq!(
            record_boundary(&task, "OP-001"),
            VerifiedEffectBoundary::Unknown
        );
        let runner = IsolatedCommandRunner::new(context.clone());
        let request = CommandRequest {
            argv: vec!["must-never-launch".into()],
            cwd: context.canonical_project_root,
            timeout: std::time::Duration::from_secs(1),
        };
        assert_eq!(
            runner.run(&request).status,
            work_feature::ports::CommandStatus::LaunchFailed
        );
    }
    #[cfg(any(target_os = "macos", windows))]
    #[test]
    fn a_changed_invocation_is_rejected_before_allocating_or_launching() {
        let context = context();
        let receipt = "outputs/work/e/TASK-001/ATTEMPT-001/receipts/CMD-001";
        let p = policy(&context, receipt).unwrap();
        let preview = json!({"receipt_dir":receipt,"execution":{"work_isolation":p},
            "working_directory":context.canonical_project_root,"request":{"timeout_seconds":1},
            "invocation":{"kind":"direct","executable":"must-never-launch","argv":["must-never-launch"]}});
        for field in ["argv", "cwd", "timeout"] {
            let runner = IsolatedCommandRunner::new(context.clone());
            runner.bind(&preview).unwrap();
            let mut request =
                work_feature::execution::command_publication::build_command_request(&preview)
                    .unwrap();
            match field {
                "argv" => request.argv.push("changed".into()),
                "cwd" => request.cwd = context.canonical_project_root.join("changed"),
                _ => request.timeout += std::time::Duration::from_secs(1),
            }
            assert_eq!(
                runner.run(&request).status,
                work_feature::ports::CommandStatus::LaunchFailed
            );
            assert!(!Path::new(&p.writable_directory).exists());
        }
        let runner = IsolatedCommandRunner::new(context);
        let mut foreign = preview;
        foreign["receipt_dir"] = json!("outputs/work/e/TASK-001/ATTEMPT-002/receipts/CMD-001");
        assert!(runner.bind(&foreign).is_err());
    }
}
