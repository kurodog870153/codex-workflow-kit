//! Handoff command flows.

use serde_json::Value;
use serde_json::json;
use work_feature::artifact_paths::ArtifactPathRepository;
use work_feature::error::{ExitCode, WorkError};
use work_feature::handoff::{HandoffCommandRepository, HandoffStorageAction};

pub fn discussion_build(request: &Value) -> Result<Value, WorkError> {
    work_feature::handoff::build_discussion(request)
}

pub fn validate(
    repository: &impl ArtifactPathRepository,
    request: &Value,
) -> Result<Value, WorkError> {
    work_feature::handoff::validate_task_handoff(repository, request)
}

pub fn validate_task(
    repository: &impl work_feature::artifact_paths::ArtifactPathRepository,
    request: &Value,
) -> Result<Value, WorkError> {
    work_feature::handoff::validate_task_handoff(repository, request)
}

pub struct HandoffArgs<'a> {
    pub task_path: Option<&'a str>,
    pub task_id: Option<&'a str>,
    pub attempt_id: Option<&'a str>,
    pub preflight: bool,
}

fn required(value: Option<&str>) -> Result<&str, WorkError> {
    value.ok_or_else(|| {
        WorkError::new(
            ExitCode::InternalError,
            "unreachable_command",
            "The parsed command could not be dispatched.",
            json!({}),
        )
    })
}

pub fn run<P: HandoffCommandRepository>(
    command: &str,
    paths: &impl ArtifactPathRepository,
    create_port: impl FnOnce() -> Result<P, WorkError>,
    args: HandoffArgs<'_>,
    request: &Value,
) -> Result<Value, WorkError> {
    if command == "build-discussion" {
        return discussion_build(request);
    }
    if command == "validate" || command == "render" {
        let validated = validate(paths, request)?;
        return if command == "render" {
            Ok(request.clone())
        } else {
            Ok(validated)
        };
    }
    let action = match command {
        "build-task-to-execute" => HandoffStorageAction::TaskToExecute {
            verify: false,
            task_path: required(args.task_path)?,
            task_id: required(args.task_id)?,
        },
        "verify-task-to-execute" => HandoffStorageAction::TaskToExecute {
            verify: true,
            task_path: required(args.task_path)?,
            task_id: required(args.task_id)?,
        },
        "build-execute-to-task" | "verify-execute-to-task" => {
            let verify = command.starts_with("verify-");
            let direction = "execute_to_task";
            HandoffStorageAction::ExecuteReturn {
                verify,
                preflight: args.preflight,
                direction,
                task_path: required(args.task_path)?,
                task_id: required(args.task_id)?,
                attempt_id: if args.preflight {
                    None
                } else {
                    Some(required(args.attempt_id)?)
                },
            }
        }
        _ => {
            return Err(WorkError::new(
                ExitCode::InternalError,
                "unreachable_command",
                "The parsed command could not be dispatched.",
                json!({}),
            ));
        }
    };
    create_port()?.execute(action, request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use work_feature::artifact_paths::ArtifactPaths;
    use work_model::identifiers::RequirementId;

    struct NoPaths;
    impl ArtifactPathRepository for NoPaths {
        fn resolve(&self, _: &str) -> Result<std::path::PathBuf, WorkError> {
            panic!("unused")
        }
        fn exists(&self, _: &str) -> Result<bool, WorkError> {
            panic!("unused")
        }
        fn read_raw(&self, _: &str) -> Result<Vec<u8>, WorkError> {
            panic!("unused")
        }
        fn create_new(&self, _: &str, _: &[u8]) -> Result<(), WorkError> {
            panic!("unused")
        }
        fn validate_paths(&self, _: &RequirementId, _: &ArtifactPaths) -> Result<(), WorkError> {
            panic!("unused")
        }
    }

    struct FakePort;
    impl HandoffCommandRepository for FakePort {
        fn execute(&self, action: HandoffStorageAction<'_>, _: &Value) -> Result<Value, WorkError> {
            match action {
                HandoffStorageAction::ExecuteReturn {
                    verify: true,
                    preflight: false,
                    direction: "execute_to_task",
                    task_path: "task/index.json",
                    task_id: "TASK-001",
                    attempt_id: Some("ATTEMPT-001"),
                } => Ok(json!({"routed":true})),
                _ => panic!("wrong handoff action"),
            }
        }
    }

    #[test]
    fn closed_return_dispatches_to_the_selected_port_action() {
        let result = run(
            "verify-execute-to-task",
            &NoPaths,
            || Ok(FakePort),
            HandoffArgs {
                task_path: Some("task/index.json"),
                task_id: Some("TASK-001"),
                attempt_id: Some("ATTEMPT-001"),
                preflight: false,
            },
            &json!({}),
        )
        .unwrap();
        assert_eq!(result["routed"], true);
    }
}
