//! Pure CLI operation effect and routing identity rules.

use serde_json::Value;

const EFFECTS: &str = include_str!("operation_effects.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationEffect {
    ReadOnly,
    Write,
    ExternalEffect,
}

impl OperationEffect {
    pub fn authorization_state(self) -> &'static str {
        if self == Self::ReadOnly {
            "read_only"
        } else {
            "authorized"
        }
    }

    pub fn side_effect_boundary(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Write => "authorized_atomic_write",
            Self::ExternalEffect => "authorized_external_effect",
        }
    }
}

pub fn operation_effect(command: &str, operation: &str) -> Option<OperationEffect> {
    let table: Value = serde_json::from_str(EFFECTS).expect("fixed operation effects");
    match table[command][operation].as_str()? {
        "read_only" => Some(OperationEffect::ReadOnly),
        "write" => Some(OperationEffect::Write),
        "external_effect" => Some(OperationEffect::ExternalEffect),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRouting {
    pub mode: &'static str,
    pub next_action: &'static str,
    pub formal_events: Vec<&'static str>,
    pub role: String,
}

pub fn routing_identity(
    command: &str,
    operation: &str,
    delegated_role: Option<&str>,
) -> Option<OperationRouting> {
    operation_effect(command, operation)?;
    let mut route = OperationRouting {
        mode: "execute",
        next_action: "continue_execution",
        formal_events: vec![],
        role: "main".into(),
    };
    match command {
        "source" => {
            route.mode = "task";
            route.next_action = if operation == "capture" {
                "capture_source"
            } else {
                "read_source"
            };
        }
        "migration" => {
            route.mode = "migration";
            route.next_action = "review_reconciliation";
            route.formal_events.push("migration");
        }
        "specification" => {
            route.mode = "revise";
            route.next_action = "review_reconciliation";
            route
                .formal_events
                .push(if operation.starts_with("reconciliation-") {
                    "reconciliation"
                } else {
                    "revision"
                });
        }
        "task" => {
            route.mode = "task";
            route.next_action = "choose_task";
        }
        "progress" => {
            route.mode = "task";
            route.next_action = if operation == "read" {
                "confirm_resume"
            } else {
                "confirm_review"
            };
            route.formal_events.push(if operation == "read" {
                "progress_read"
            } else {
                "progress_save"
            });
        }
        "handoff" => route.formal_events.push("handoff"),
        "delegation" => {
            route.formal_events.push("delegation");
            route.role = delegated_role.unwrap_or("main").into();
        }
        "execute" => {
            if matches!(operation, "preflight" | "worktree") {
                route.next_action = "select_task_for_execution";
            }
            let event = match operation {
                "attempt-start" => Some("attempt_start"),
                "recover-attempt-start" | "recover" | "recovery-prepare" => Some("recovery"),
                "attempt-close" => Some("attempt_close"),
                "correction-create" => Some("correction"),
                "command-correction" => Some("command_correction"),
                "command-run" => Some("command_execution"),
                _ => None,
            };
            if let Some(event) = event {
                route.formal_events.push(event);
            }
        }
        _ => return None,
    }
    Some(route)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_public_operation_has_an_explicit_effect() {
        let table: Value = serde_json::from_str(EFFECTS).unwrap();
        for (command, count) in [
            ("source", 3),
            ("task", 7),
            ("specification", 9),
            ("migration", 6),
            ("execute", 16),
            ("delegation", 2),
            ("progress", 4),
            ("handoff", 7),
        ] {
            assert_eq!(table[command].as_object().unwrap().len(), count);
            for (operation, effect) in table[command].as_object().unwrap() {
                let classified = operation_effect(command, operation).unwrap();
                assert_eq!(
                    classified.authorization_state(),
                    if effect == "read_only" {
                        "read_only"
                    } else {
                        "authorized"
                    }
                );
                assert!(routing_identity(command, operation, None).is_some());
            }
        }
        assert_eq!(
            operation_effect("execute", "command-run")
                .unwrap()
                .side_effect_boundary(),
            "authorized_external_effect"
        );
        assert_eq!(
            operation_effect("task", "save")
                .unwrap()
                .side_effect_boundary(),
            "authorized_atomic_write"
        );
        for operation in ["read", "validate"] {
            assert_eq!(
                operation_effect("source", operation),
                Some(OperationEffect::ReadOnly)
            );
            assert_eq!(
                routing_identity("source", operation, None)
                    .unwrap()
                    .next_action,
                "read_source"
            );
        }
        assert_eq!(operation_effect("task", "unknown"), None);
        assert_eq!(operation_effect("task", "draft-save"), None);
    }

    #[test]
    fn routing_identity_tracks_python_command_categories() {
        assert_eq!(
            routing_identity("migration", "apply", None).unwrap(),
            OperationRouting {
                mode: "migration",
                next_action: "review_reconciliation",
                formal_events: vec!["migration"],
                role: "main".into()
            }
        );
        assert_eq!(
            routing_identity("execute", "preflight", None)
                .unwrap()
                .next_action,
            "select_task_for_execution"
        );
        let delegated = routing_identity("delegation", "validate", Some("worker")).unwrap();
        assert_eq!(
            (delegated.role.as_str(), delegated.formal_events.as_slice()),
            ("worker", ["delegation"].as_slice())
        );
    }
}
