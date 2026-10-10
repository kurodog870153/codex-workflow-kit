//! Deterministic cross-Task access analysis; semantic independence remains reviewed evidence.

use super::{TaskIssue, issue, resolve_dependencies};
use crate::canonical::portable_path_identity;
use crate::derivation::fingerprint;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use work_model::common::Nullable;
use work_model::discussion::DiscussionSession;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileConflict {
    pub task_ids: [String; 2],
    pub path: String,
    pub actions: [String; 2],
}

fn problem(reason: &'static str, details: Value) -> TaskIssue {
    issue(
        reason,
        "File access requires an ordered dependency or current confirmed independence review.",
        details,
    )
}

pub fn conflicts(contract: &Value) -> Result<Vec<FileConflict>, TaskIssue> {
    let tasks: Vec<_> = contract["tasks"].as_array().into_iter().flatten().collect();
    let ids: Vec<_> = tasks
        .iter()
        .map(|t| t["id"].as_str().unwrap_or("").to_owned())
        .collect();
    let deps: BTreeMap<_, _> = tasks
        .iter()
        .map(|t| {
            (
                t["id"].as_str().unwrap_or("").to_owned(),
                t["dependencies"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
            )
        })
        .collect();
    let graph = resolve_dependencies(&ids, &deps)?;
    let mut access: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for task in &tasks {
        let id = task["id"].as_str().unwrap_or("");
        for file in task["files"].as_array().into_iter().flatten() {
            let action = file["action"].as_str().unwrap_or("");
            for field in ["path", "source", "destination"] {
                if let Some(path) = file[field].as_str() {
                    access
                        .entry(portable_path_identity(path))
                        .or_default()
                        .push((id.into(), action.into()));
                }
            }
        }
        for input in task["inputs"].as_array().into_iter().flatten() {
            let source = input["source"].as_str().unwrap_or("");
            if input["kind"] == "task_output" {
                let Some((producer, file_id)) = source.split_once('/') else {
                    return Err(problem(
                        "task_output_source_invalid",
                        json!({"task_id":id,"source":source}),
                    ));
                };
                if !graph.ancestors[id].contains(producer) {
                    return Err(problem(
                        "task_output_dependency_missing",
                        json!({"task_id":id,"producer":producer}),
                    ));
                }
                let file = tasks
                    .iter()
                    .find(|t| t["id"] == producer)
                    .and_then(|t| t["files"].as_array())
                    .and_then(|files| files.iter().find(|f| f["id"] == file_id))
                    .ok_or_else(|| {
                        problem(
                            "task_output_source_invalid",
                            json!({"task_id":id,"source":source}),
                        )
                    })?;
                let path = file["path"]
                    .as_str()
                    .or_else(|| file["destination"].as_str())
                    .ok_or_else(|| {
                        problem("task_output_source_invalid", json!({"source":source}))
                    })?;
                access
                    .entry(portable_path_identity(path))
                    .or_default()
                    .push((id.into(), "read".into()));
            } else if input["kind"] == "project_state"
                && crate::derivation::identity::runtime_relative_path(source)
            {
                access
                    .entry(portable_path_identity(source))
                    .or_default()
                    .push((id.into(), "read".into()));
            }
        }
    }
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for (path, accesses) in access {
        for (n, a) in accesses.iter().enumerate() {
            for b in &accesses[n + 1..] {
                if a.0 == b.0
                    || (a.1 == "read" && b.1 == "read")
                    || graph.ancestors[&a.0].contains(&b.0)
                    || graph.ancestors[&b.0].contains(&a.0)
                {
                    continue;
                }
                let (a, b) = if a.0 < b.0 { (a, b) } else { (b, a) };
                if seen.insert((a.clone(), b.clone(), path.clone())) {
                    result.push(FileConflict {
                        task_ids: [a.0.clone(), b.0.clone()],
                        path: path.clone(),
                        actions: [a.1.clone(), b.1.clone()],
                    });
                }
            }
        }
    }
    Ok(result)
}

pub fn matches_plan(planned: &Value, actual: &Value) -> bool {
    [
        "id",
        "title",
        "goal",
        "skill_id",
        "dependencies",
        "inputs",
        "files",
        "risks",
        "steps",
        "commands",
        "operations",
        "validations",
        "decisions",
        "traceability",
        "acceptance_criteria",
    ]
    .iter()
    .all(|key| actual.get(*key) == planned.get(*key))
        && ["selected_paths", "references", "instructions_sha256"]
            .iter()
            .all(|key| {
                actual["instruction_selection"].get(*key)
                    == planned["instruction_selection"].get(*key)
            })
}

pub fn matches_reviewed_ancestry(contract: &Value, records: &[Value], task_id: &str) -> bool {
    let mut pending = vec![task_id.to_owned()];
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(actual) = contract["tasks"]
            .as_array()
            .and_then(|tasks| tasks.iter().find(|task| task["id"] == id))
        else {
            return false;
        };
        let Some(planned) = records.iter().find(|task| task["id"] == id) else {
            return false;
        };
        if !matches_plan(planned, actual) {
            return false;
        }
        pending.extend(
            actual["dependencies"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str().map(str::to_owned)),
        );
    }
    true
}

pub fn matches_reviewed_source(session: &DiscussionSession, contract: &Value) -> bool {
    // Partial pure-rule inputs omit context; formal Feature boundaries supply the complete collection.
    if contract.get("source").is_none() {
        return true;
    }
    let Nullable::Value(source) = &session.context.confirmed_source else {
        return false;
    };
    let expected = json!({"source":{"kind":"snapshot","manifest":source.snapshot},"hierarchy_selection":source.hierarchy_selection,"skill_selection":source.skill_selection,"acceptance_criteria":source.acceptance_criteria});
    [
        "source",
        "hierarchy_selection",
        "skill_selection",
        "acceptance_criteria",
    ]
    .iter()
    .all(|field| contract.get(*field) == expected.get(*field))
}

pub fn validate(contract: &Value, session: Option<&DiscussionSession>) -> Result<(), TaskIssue> {
    let pending = conflicts(contract)?;
    if pending.is_empty() {
        return Ok(());
    }
    let session = session.ok_or_else(|| problem("task_file_independence_confirmation_required", json!({"task_ids":pending[0].task_ids,"path":pending[0].path,"actions":pending[0].actions})))?;
    crate::discussion::verify_integrity(session).map_err(|e| problem(e.0, json!({})))?;
    let records =
        crate::discussion::assembly::task_records(session).map_err(|e| problem(e.0, json!({})))?;
    if !matches_reviewed_source(session, contract) {
        return Err(problem(
            "task_file_review_binding_mismatch",
            json!({"location":"source_context"}),
        ));
    }
    for conflict in pending {
        let details =
            json!({"task_ids":conflict.task_ids,"path":conflict.path,"actions":conflict.actions});
        if conflict.actions != ["modify", "modify"] {
            return Err(problem("task_file_dependency_required", details));
        }
        for id in &conflict.task_ids {
            let planned = records
                .iter()
                .find(|t| t["id"] == *id)
                .ok_or_else(|| problem("task_file_review_binding_mismatch", details.clone()))?;
            let actual = contract["tasks"]
                .as_array()
                .and_then(|ts| ts.iter().find(|t| t["id"] == *id))
                .ok_or_else(|| problem("task_file_review_binding_mismatch", details.clone()))?;
            if !matches_plan(planned, actual) || !matches_reviewed_ancestry(contract, &records, id)
            {
                return Err(problem("task_file_review_binding_mismatch", details));
            }
        }
        let bindings = conflict
            .task_ids
            .clone()
            .map(|id| fingerprint::discussion_planning(session, &id));
        let verified = session.tasks.iter().any(|t| match &t.review {
            Nullable::Value(r) if !r.needs_review => r.file_independence.iter().any(|e| {
                e.task_ids == conflict.task_ids
                    && portable_path_identity(&e.path) == conflict.path
                    && e.actions == conflict.actions
                    && e.planning_sha256 == bindings
                    && e.confirmed
                    && !e.evidence.trim().is_empty()
            }),
            _ => false,
        });
        if !verified {
            return Err(problem(
                "task_file_independence_confirmation_required",
                details,
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan(action: &str) -> Value {
        json!({"tasks":[
            {"id":"TASK-001","files":[{"id":"FILE-001","action":"create","path":"Café.txt"}]},
            {"id":"TASK-002","files":[{"id":"FILE-001","action":action,"path":"cafe\u{301}.txt"}]},
            {"id":"TASK-003","dependencies":["TASK-002"]}
        ]})
    }
    #[test]
    fn portable_write_conflicts_require_order_and_readers_require_producer() {
        let mut p = plan("modify");
        assert_eq!(conflicts(&p).unwrap().len(), 1);
        assert_eq!(
            validate(&p, None).unwrap_err().reason_code,
            "task_file_independence_confirmation_required"
        );
        p["tasks"][1]["dependencies"] = json!(["TASK-001"]);
        p["tasks"][2]["inputs"] = json!([{"kind":"task_output","source":"TASK-001/FILE-001"}]);
        assert!(validate(&p, None).is_ok());
        p["tasks"][2]["dependencies"] = json!([]);
        assert_eq!(
            conflicts(&p).unwrap_err().reason_code,
            "task_output_dependency_missing"
        );
        p["tasks"][2]["inputs"][0]["source"] = json!("TASK-001/MISSING");
        p["tasks"][2]["dependencies"] = json!(["TASK-002"]);
        assert_eq!(
            conflicts(&p).unwrap_err().reason_code,
            "task_output_source_invalid"
        );
    }
    #[test]
    fn moves_touch_both_paths_and_shared_readers_are_independent() {
        let p = json!({"tasks":[
            {"id":"TASK-001","files":[{"action":"move","source":"a","destination":"b"}]},
            {"id":"TASK-002","files":[{"action":"modify","path":"a"}]},
            {"id":"TASK-003","inputs":[{"kind":"project_state","source":"b"}]}
        ]});
        assert_eq!(conflicts(&p).unwrap().len(), 2);
        let reads = json!({"tasks":[{"id":"TASK-001","inputs":[{"kind":"project_state","source":"a"}]},
            {"id":"TASK-002","inputs":[{"kind":"project_state","source":"a"}]}]});
        assert!(conflicts(&reads).unwrap().is_empty());
    }

    #[test]
    fn revised_instruction_selection_invalidates_reviewed_plan_and_ancestors() {
        let planned = json!({"id":"TASK-001","instruction_selection":{
            "selected_paths":["general"],"references":[],"instructions_sha256":"reviewed"}});
        let mut actual = planned.clone();
        actual["instruction_selection"]["resolved_paths"] = json!(["general"]);
        assert!(matches_plan(&planned, &actual));
        let child = json!({"id":"TASK-002","dependencies":["TASK-001"]});
        let records = vec![planned.clone(), child.clone()];
        for (field, value) in [
            ("selected_paths", json!(["programming-language/rust"])),
            ("references", json!(["task.general.task-records"])),
            ("instructions_sha256", json!("revised")),
        ] {
            let mut revised = actual.clone();
            revised["instruction_selection"][field] = value;
            assert!(!matches_plan(&planned, &revised));
            let collection = json!({"tasks":[revised,child]});
            assert!(!matches_reviewed_ancestry(
                &collection,
                &records,
                "TASK-002"
            ));
        }
        let collection = json!({"tasks":[actual,child]});
        assert!(matches_reviewed_ancestry(&collection, &records, "TASK-002"));
    }
}
