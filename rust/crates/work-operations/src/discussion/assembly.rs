//! Derive semantic TASK records directly from a verified discussion revision.

use super::{DiscussionIssue, Result, ready_to_generate};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use work_model::common::Nullable;
use work_model::discussion::{DecisionStatus, DiscussionSession};

pub fn task_records(session: &DiscussionSession) -> Result<Vec<Value>> {
    ready_to_generate(session)?;
    let dependency_files: BTreeMap<String, BTreeMap<String, String>> = session
        .tasks
        .iter()
        .map(|task| {
            (
                task.id.clone(),
                task.files
                    .iter()
                    .enumerate()
                    .map(|(position, file)| (file.key.clone(), format!("FILE-{:03}", position + 1)))
                    .collect(),
            )
        })
        .collect();
    let acceptance: Vec<_> = session
        .context
        .acceptance_criteria
        .iter()
        .map(|a| a.id.clone())
        .collect();
    let mut records = Vec::new();
    let mut tasks: Vec<_> = session.tasks.iter().collect();
    tasks.sort_by_key(|task| &task.id);
    for task in tasks {
        let decisions: Vec<_> = session.decisions.iter().filter(|d|
            d.status == DecisionStatus::Confirmed && (task.decision_ids.contains(&d.id) || d.task_ids.contains(&task.id))
        ).map(|d| {
            let Nullable::Value(resolution) = &d.resolution else { unreachable!("verified confirmation") };
            let option = d.options.iter().find(|o| o.id == resolution.option_id).expect("verified option");
            json!({"key":d.id.to_ascii_lowercase(),"statement":format!("{}: {} — {}", d.question, option.label, option.explanation),"rationale":resolution.rationale})
        }).collect();
        let mut candidate = json!({"acceptance_ids":task.acceptance_ids,"acceptance_criteria":task.acceptance_criteria,
            "steps":task.steps,"validations":task.validations});
        for step in candidate["steps"].as_array_mut().expect("serialized steps") {
            for reference in step["references"]
                .as_array_mut()
                .expect("serialized references")
            {
                if reference["kind"] == "decisions" {
                    if let Some(id) = reference["key"].as_str() {
                        if session.decisions.iter().any(|d| d.id == id) {
                            reference["key"] = json!(id.to_ascii_lowercase());
                        }
                    }
                }
            }
        }
        for (name, value) in [
            ("inputs", json!(task.inputs)),
            ("decisions", json!(decisions)),
            ("files", json!(task.files)),
            ("risks", json!(task.risks)),
            ("commands", json!(task.commands)),
            ("operations", json!(task.operations)),
        ] {
            if !value.as_array().expect("array").is_empty() {
                candidate[name] = value;
            }
        }
        let mut record = crate::task::candidate::build_semantic_candidate(
            &candidate,
            &acceptance,
            &task.dependencies,
            &dependency_files,
        )
        .map_err(|e| DiscussionIssue(e.reason_code))?;
        for (name, value) in [
            ("id", json!(task.id)),
            ("title", json!(task.title)),
            ("goal", json!(task.goal)),
            ("skill_id", json!(task.skill_id)),
        ] {
            record[name] = value;
        }
        if !task.dependencies.is_empty() {
            record["dependencies"] = json!(task.dependencies);
        }
        record["traceability"] = json!({"acceptance_ids":task.acceptance_ids});
        record["acceptance_criteria"] = json!(task.acceptance_criteria);
        records.push(record);
    }
    Ok(records)
}

pub fn trace(session: &DiscussionSession) -> work_model::task::discussion_trace::DiscussionTrace {
    work_model::task::discussion_trace::DiscussionTrace {
        session_path: format!(
            "outputs/work/discussions/{}/session.json",
            session.requirement_id
        ),
        revision: session.revision,
        content_sha256: session.commit.content_sha256.clone(),
        decision_versions: session
            .decisions
            .iter()
            .filter(|d| d.status == DecisionStatus::Confirmed)
            .map(|d| (d.id.clone(), d.question_version))
            .collect(),
        task_decisions: session
            .tasks
            .iter()
            .map(|task| {
                (
                    task.id.clone(),
                    session
                        .decisions
                        .iter()
                        .filter(|d| {
                            d.status == DecisionStatus::Confirmed
                                && (task.decision_ids.contains(&d.id)
                                    || d.task_ids.contains(&task.id))
                        })
                        .map(|d| d.id.clone())
                        .collect(),
                )
            })
            .collect(),
    }
}
