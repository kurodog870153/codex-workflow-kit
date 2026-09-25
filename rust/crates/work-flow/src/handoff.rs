//! Handoff command flows.

use serde_json::Value;
use serde_json::json;
use work_feature::error::{ExitCode, WorkError};
use work_feature::handoff::{HandoffCommandRepository, HandoffStorageAction};
use work_feature::plan::PlanPathRepository;

pub fn discussion_build(request: &Value) -> Result<Value, WorkError> {
    work_feature::handoff::build_discussion(request)
}

pub fn validate(repository: &impl PlanPathRepository, request: &Value) -> Result<Value, WorkError> {
    work_feature::handoff::validate_handoff(repository, request)
}

pub struct HandoffArgs<'a> {
    pub plan_path: Option<&'a str>,
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
    paths: &impl PlanPathRepository,
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
        "build-plan-to-task" => HandoffStorageAction::PlanToTask {
            verify: false,
            plan_path: required(args.plan_path)?,
        },
        "verify-plan-to-task" => HandoffStorageAction::PlanToTask {
            verify: true,
            plan_path: required(args.plan_path)?,
        },
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
        "build-task-to-plan" => HandoffStorageAction::TaskToPlan {
            verify: false,
            plan_path: None,
            task_path: required(args.task_path)?,
            task_id: args.task_id,
        },
        "verify-task-to-plan" => HandoffStorageAction::TaskToPlan {
            verify: true,
            plan_path: Some(required(args.plan_path)?),
            task_path: required(args.task_path)?,
            task_id: args.task_id,
        },
        "build-execute-to-task"
        | "build-execute-to-plan"
        | "verify-execute-to-task"
        | "verify-execute-to-plan" => {
            let verify = command.starts_with("verify-");
            let direction = if command.ends_with("-task") {
                "execute_to_task"
            } else {
                "execute_to_plan"
            };
            HandoffStorageAction::ExecuteReturn {
                verify,
                preflight: args.preflight,
                direction,
                plan_path: if verify {
                    Some(required(args.plan_path)?)
                } else {
                    None
                },
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
    use work_model::identifiers::RequirementId;

    struct NoPaths;
    impl PlanPathRepository for NoPaths {
        fn default_paths(&self, _: &RequirementId) -> Result<Value, WorkError> {
            panic!("unused")
        }
        fn validate_paths(
            &self,
            _: &RequirementId,
            _: &Value,
            _: &str,
            _: bool,
        ) -> Result<(), WorkError> {
            panic!("unused")
        }
        fn exists(&self, _: &str) -> Result<bool, WorkError> {
            panic!("unused")
        }
        fn create_exclusive(&self, _: &str, _: &[u8]) -> Result<(), WorkError> {
            panic!("unused")
        }
        fn read(&self, _: &str) -> Result<Vec<u8>, WorkError> {
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
                    direction: "execute_to_plan",
                    plan_path: Some("plan.json"),
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
            "verify-execute-to-plan",
            &NoPaths,
            || Ok(FakePort),
            HandoffArgs {
                plan_path: Some("plan.json"),
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
