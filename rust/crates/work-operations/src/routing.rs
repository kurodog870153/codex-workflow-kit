//! Pure routing selection and instruction manifest rules.

use crate::canonical::canonical_json_sha256;
use serde_json::{Value, json};

const CATALOG: &str = include_str!("routing_catalog.json");
const BOOTSTRAP: &str = "work.instruction-loading";

#[derive(Debug, Clone)]
pub struct RoutingRequest<'a> {
    pub status: &'a str,
    pub operation: &'a str,
    pub confirmation: bool,
    pub mode: Option<&'a str>,
    pub artifact_lifecycle: &'a str,
    pub formal_events: &'a [&'a str],
    pub role: &'a str,
    pub authorization_state: Option<&'a str>,
    pub verified_state_sha256: &'a str,
}

pub fn select<F, E>(request: &RoutingRequest<'_>, mut fingerprint: F) -> Result<Value, E>
where
    F: FnMut(&str, &str) -> Result<String, E>,
{
    let catalog: Value = serde_json::from_str(CATALOG).expect("fixed routing catalog");
    let mode = request
        .mode
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| infer_mode(request.operation));
    let authorization = request
        .authorization_state
        .filter(|value| !value.is_empty())
        .unwrap_or(if request.confirmation {
            "confirmation_required"
        } else {
            "read_only"
        });
    let mut events: Vec<&str> = request.formal_events.to_vec();
    events.sort_unstable();
    events.dedup();
    let mut reasons = Vec::<String>::new();
    if ![
        "plan",
        "task",
        "execute",
        "progress",
        "repair",
        "specification",
    ]
    .contains(&mode)
    {
        reasons.push(format!("unknown_mode:{mode}"));
    }
    if ![
        "main",
        "explorer",
        "worker",
        "reviewer",
        "monitor",
        "plan",
        "task-coordinator",
        "execute",
        "task-skill",
        "artifact-editor",
        "progress-saver",
    ]
    .contains(&request.role)
    {
        reasons.push(format!("unknown_role:{}", request.role));
    }
    if !["read_only", "confirmation_required", "authorized"].contains(&authorization) {
        reasons.push(format!("unknown_authorization_state:{authorization}"));
    }
    let known_events = [
        "invocation",
        "handoff",
        "progress_read",
        "progress_save",
        "migration",
        "revision",
        "reconciliation",
        "recovery",
        "correction",
        "command_correction",
        "command_execution",
        "attempt_start",
        "attempt_close",
        "delegation",
        "invalid_artifact",
        "skill_load",
        "safety_rejection",
        "file_failure",
    ];
    for event in &events {
        if !known_events.contains(event) {
            reasons.push(format!("unknown_event:{event}"));
        }
    }
    let terminal: Vec<&str> = events
        .iter()
        .copied()
        .filter(|event| ["handoff", "recovery", "safety_rejection"].contains(event))
        .collect();
    if terminal.len() > 1 {
        reasons.push(format!(
            "conflicting_terminal_events:{}",
            terminal.join(",")
        ));
    }
    if request.role != "main" && !events.contains(&"delegation") {
        reasons.push("delegated_role_without_delegation_event".into());
    }
    let base = catalog["decisions"][request.operation].as_array();
    if authorization == "authorized" && base.is_none() {
        reasons.push("authorized_unknown_operation".into());
    }
    if request.status.is_empty()
        || request.operation.is_empty()
        || request.artifact_lifecycle.is_empty()
    {
        reasons.push("incomplete_routing_input".into());
    }
    if base.is_none() {
        reasons.push(format!("unmatched_operation:{}", request.operation));
    }
    let valid = reasons.is_empty();
    let mut names = Vec::<String>::new();
    if valid {
        for group in [base, catalog["modes"][mode].as_array()]
            .into_iter()
            .flatten()
        {
            for name in group {
                let name = name.as_str().expect("catalog name");
                if !names.iter().any(|item| item == name) {
                    names.push(name.into());
                }
            }
        }
        for event in &events {
            for name in catalog["events"][event].as_array().expect("known event") {
                let name = name.as_str().expect("catalog name");
                if !names.iter().any(|item| item == name) {
                    names.push(name.into());
                }
            }
        }
        reasons = vec![
            format!("operation:{}", request.operation),
            format!("workflow_status:{}", request.status),
            format!("mode:{mode}"),
        ];
        reasons.extend(events.iter().map(|event| format!("event:{event}")));
    } else {
        names.push(BOOTSTRAP.into());
    }
    names.sort_by_key(|name| catalog["sources"][name][2].as_u64().expect("source order"));
    let sources: Result<Vec<Value>, E> = names.iter().map(|name| {
        let entry = &catalog["sources"][name];
        let path = entry[0].as_str().expect("source path");
        Ok(json!({"logical_name": name, "path": path, "compatibility_revision": entry[1], "canonical_sha256": fingerprint(name, path)?}))
    }).collect();
    let routing_status = if valid { "VALID" } else { "REVIEW_REQUIRED" };
    let mut manifest = json!({
        "schema": "work-instruction-selection-manifest/v1",
        "router_compatibility_revision": 3,
        "routing_input": {
            "mode": mode, "status": request.status, "operation": request.operation,
            "artifact_lifecycle": request.artifact_lifecycle, "formal_events": events,
            "role": request.role, "authorization_state": authorization,
            "verified_state_sha256": request.verified_state_sha256,
        },
        "routing_status": routing_status, "sources": sources?,
        "confirmation_required": request.confirmation,
    });
    let selection_sha256 = canonical_json_sha256(&manifest).expect("routing manifest serializes");
    manifest["selection_sha256"] = json!(selection_sha256);
    let _: work_model::instruction::InstructionSelectionManifest =
        serde_json::from_value(manifest.clone()).expect("routing manifest matches its model");
    Ok(json!({
        "routing_status": routing_status, "router_compatibility_revision": 3,
        "required_instruction_sources": names, "source_order": names,
        "selection_sha256": selection_sha256, "routing_reasons": reasons,
        "selection_manifest": manifest,
    }))
}

fn infer_mode(operation: &str) -> &'static str {
    match operation {
        "prepare_plan" => "plan",
        "confirm_task_list"
        | "choose_task"
        | "confirm_start"
        | "confirm_resume"
        | "confirm_review"
        | "assemble_for_review" => "task",
        "inspect_recovery" => "repair",
        "review_reconciliation" => "specification",
        _ => "execute",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::canonical_sha256;
    use std::{fs, path::Path};

    #[test]
    fn installed_routing_matches_python_manifest_hashes() {
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/legacy-work-skill"
        ));
        for (status, operation, confirmation, lifecycle, expected) in [
            (
                "plan_required",
                "prepare_plan",
                true,
                "missing",
                "11313e0d95824bb9a6778696474f2816ee1afb15158c42d6d28536e4b9d65348",
            ),
            (
                "task_list_pending",
                "confirm_task_list",
                true,
                "current",
                "bf9caa2fe07912180b1163a1daf67837c9247fb68ae26919892081e82283412b",
            ),
        ] {
            let request = RoutingRequest {
                status,
                operation,
                confirmation,
                mode: None,
                artifact_lifecycle: lifecycle,
                formal_events: &[],
                role: "main",
                authorization_state: None,
                verified_state_sha256: "",
            };
            let result = select(&request, |_, path| {
                canonical_sha256(&fs::read(root.join(path)).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())
            })
            .unwrap();
            assert_eq!(result["routing_status"], "VALID");
            assert_eq!(result["selection_sha256"], expected);
        }
    }

    #[test]
    fn invalid_input_only_routes_bootstrap() {
        let request = RoutingRequest {
            status: "x",
            operation: "unknown",
            confirmation: false,
            mode: Some("bad"),
            artifact_lifecycle: "current",
            formal_events: &["nope"],
            role: "main",
            authorization_state: None,
            verified_state_sha256: "",
        };
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/legacy-work-skill"
        ));
        let result = select(&request, |_, path| {
            canonical_sha256(&fs::read(root.join(path)).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(
            result["selection_sha256"],
            "d2c148606f2ae4080c077bff96e2b53af33f05a5fc9842aa0f72fee89f85d52e"
        );
        assert_eq!(result["source_order"], json!([BOOTSTRAP]));
    }

    fn route_fixture(
        operation: &str,
        mode: Option<&str>,
        events: &[&str],
        role: &str,
        authorized: bool,
        changed: Option<&str>,
        missing: Option<&str>,
    ) -> Result<Value, &'static str> {
        let request = RoutingRequest {
            status: "verified",
            operation,
            confirmation: operation != "continue_execution",
            mode,
            artifact_lifecycle: "current",
            formal_events: events,
            role,
            authorization_state: authorized.then_some("authorized"),
            verified_state_sha256: "",
        };
        select(&request, |name, _| {
            if missing == Some(name) {
                return Err("missing source");
            }
            Ok(if changed == Some(name) { "b" } else { "a" }.repeat(64))
        })
    }

    #[test]
    fn routing_plan_unknown_events_and_fingerprints_match_python() {
        let plan = route_fixture("prepare_plan", None, &[], "main", false, None, None).unwrap();
        assert_eq!(plan["routing_status"], "VALID");
        assert_eq!(
            plan["required_instruction_sources"],
            json!([
                "work.instruction-loading",
                "work.shared.invocation",
                "work.shared.skill-discovery",
                "work.shared.skill-selection",
                "work.shared.source-loading",
                "work.shared.artifact-paths",
                "work.shared.fingerprints",
                "work.workflow.plan",
                "work.workflow.plan.apply-confirmed-skills",
                "work.workflow.plan.complete-the-request",
                "work.workflow.plan.use-the-deterministic-plan-contract"
            ])
        );
        for unrelated in ["work.workflow.task", "work.workflow.execute"] {
            assert!(
                !plan["required_instruction_sources"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(unrelated))
            );
        }
        let unknown =
            route_fixture("unknown_action", None, &[], "main", false, None, None).unwrap();
        assert_eq!(unknown["routing_status"], "REVIEW_REQUIRED");
        assert_eq!(unknown["required_instruction_sources"], json!([BOOTSTRAP]));

        let base = route_fixture(
            "continue_execution",
            Some("execute"),
            &[],
            "main",
            false,
            None,
            None,
        )
        .unwrap();
        let delegated = route_fixture(
            "continue_execution",
            Some("execute"),
            &["delegation"],
            "worker",
            true,
            None,
            None,
        )
        .unwrap();
        assert_ne!(base["selection_sha256"], delegated["selection_sha256"]);
        assert!(
            delegated["required_instruction_sources"]
                .as_array()
                .unwrap()
                .contains(&json!("work.shared.internal-envelope"))
        );
        for (mode, events, role) in [
            (Some("execute"), vec!["handoff", "recovery"], "main"),
            (Some("execute"), vec!["unknown"], "main"),
            (Some("execute"), vec![], "worker"),
            (Some("unknown"), vec![], "main"),
        ] {
            let invalid =
                route_fixture("continue_execution", mode, &events, role, false, None, None)
                    .unwrap();
            assert_eq!(invalid["routing_status"], "REVIEW_REQUIRED");
            assert_eq!(invalid["required_instruction_sources"], json!([BOOTSTRAP]));
        }
        assert_eq!(
            route_fixture(
                "prepare_plan",
                None,
                &[],
                "main",
                false,
                None,
                Some("work.shared.invocation")
            )
            .unwrap_err(),
            "missing source"
        );
        let unrelated = route_fixture(
            "prepare_plan",
            None,
            &[],
            "main",
            false,
            Some("work.workflow.execute"),
            None,
        )
        .unwrap();
        assert_eq!(plan["selection_sha256"], unrelated["selection_sha256"]);
        let changed = route_fixture(
            "prepare_plan",
            None,
            &[],
            "main",
            false,
            Some("work.shared.invocation"),
            None,
        )
        .unwrap();
        assert_ne!(plan["selection_sha256"], changed["selection_sha256"]);
    }

    #[test]
    fn routing_catalog_operations_events_and_source_reachability_match_python() {
        use std::collections::BTreeSet;
        let catalog: Value = serde_json::from_str(CATALOG).unwrap();
        let root = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/legacy-work-skill"
        ));
        let workflow_entries: BTreeSet<_> = catalog["sources"]
            .as_object()
            .unwrap()
            .keys()
            .filter(|name| name.starts_with("work.workflow.") && name.matches('.').count() == 2)
            .cloned()
            .collect();
        let mut reached = BTreeSet::new();
        for (operation, expected) in catalog["decisions"].as_object().unwrap() {
            let first = route_fixture(operation, None, &[], "main", false, None, None).unwrap();
            let second = route_fixture(operation, None, &[], "main", false, None, None).unwrap();
            assert_eq!(first, second, "{operation}");
            assert_eq!(first["routing_status"], "VALID", "{operation}");
            let names: BTreeSet<_> = first["required_instruction_sources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_owned())
                .collect();
            let expected: BTreeSet<_> = expected
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_owned())
                .collect();
            assert_eq!(names, expected, "{operation}");
            assert!(names.intersection(&workflow_entries).count() < workflow_entries.len());
            for name in &names {
                assert!(
                    root.join(catalog["sources"][name][0].as_str().unwrap())
                        .is_file()
                );
            }
            reached.extend(names);
        }
        let base = route_fixture(
            "continue_execution",
            Some("execute"),
            &[],
            "main",
            false,
            None,
            None,
        )
        .unwrap();
        let base_names: BTreeSet<_> = base["required_instruction_sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect();
        for (event, declared) in catalog["events"].as_object().unwrap() {
            let role = if event == "delegation" {
                "worker"
            } else {
                "main"
            };
            let mode = if event.starts_with("progress_") {
                "progress"
            } else {
                "execute"
            };
            let routed = route_fixture(
                "continue_execution",
                Some(mode),
                &[event],
                role,
                false,
                None,
                None,
            )
            .unwrap();
            assert_eq!(routed["routing_status"], "VALID", "{event}");
            let selected: BTreeSet<_> = routed["required_instruction_sources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap().to_owned())
                .collect();
            if mode == "execute" {
                let declared: BTreeSet<_> = declared
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap().to_owned())
                    .collect();
                assert_eq!(
                    selected
                        .difference(&base_names)
                        .cloned()
                        .collect::<BTreeSet<_>>(),
                    declared.difference(&base_names).cloned().collect()
                );
            }
            reached.extend(selected);
        }
        assert_eq!(
            reached,
            catalog["sources"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect()
        );
    }
}
