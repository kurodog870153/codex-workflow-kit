//! Pure TASK planning source refresh decisions.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::canonical::{canonical_json, parse_json_contract, sha256_hex};
use crate::task::TaskIssue;
use crate::task::draft::{validate_planning_index, validate_task_draft};
use crate::task::draft_list::PreparedListChange;

fn issue(reason_code: &'static str, message: &'static str) -> TaskIssue {
    TaskIssue {
        reason_code,
        message,
        details: json!({}),
    }
}

pub struct SourceEffects {
    pub changed: BTreeSet<String>,
    pub affected: BTreeSet<String>,
    pub historical_drafts: BTreeSet<String>,
}

pub fn source_change_effects(
    previous: &Value,
    new_source: &Value,
    selections: &BTreeMap<String, Value>,
    instruction_hashes: &BTreeMap<String, String>,
    reason: &str,
) -> Result<SourceEffects, TaskIssue> {
    validate_planning_index(previous)?;
    if reason.trim().is_empty() {
        return Err(issue("empty_text_value", "A non-empty reason is required."));
    }
    let old_rows = previous["tasks"].as_array().expect("validated tasks");
    let ids: BTreeSet<String> = old_rows
        .iter()
        .map(|row| row["id"].as_str().expect("validated ID").to_owned())
        .collect();
    if selections.keys().cloned().collect::<BTreeSet<_>>() != ids
        || instruction_hashes.keys().cloned().collect::<BTreeSet<_>>() != ids
    {
        return Err(issue(
            "invalid_object_fields",
            "Source selections must identify every active TASK exactly once.",
        ));
    }
    let global_change = *new_source != previous["source"];
    let changed: BTreeSet<String> = old_rows
        .iter()
        .filter(|row| {
            let id = row["id"].as_str().expect("validated ID");
            row["instructions_sha256"] != instruction_hashes[id]
                || row
                    .get("instruction_selection")
                    .is_some_and(|old| *old != selections[id])
        })
        .map(|row| row["id"].as_str().expect("validated ID").to_owned())
        .collect();
    let mut affected = if global_change {
        ids.clone()
    } else {
        changed.clone()
    };
    if affected.is_empty() {
        return Err(issue(
            "draft_sources_unchanged",
            "No source fingerprint changed.",
        ));
    }
    loop {
        let downstream: BTreeSet<String> = old_rows
            .iter()
            .filter(|row| {
                row["dependencies"].as_array().is_some_and(|dependencies| {
                    dependencies
                        .iter()
                        .any(|dep| dep.as_str().is_some_and(|id| affected.contains(id)))
                })
            })
            .map(|row| row["id"].as_str().expect("validated ID").to_owned())
            .collect();
        let len = affected.len();
        affected.extend(downstream);
        if len == affected.len() {
            break;
        }
    }
    let historical_drafts = old_rows
        .iter()
        .filter(|row| row.get("draft_ref").is_some())
        .filter_map(|row| {
            row["id"]
                .as_str()
                .filter(|id| affected.contains(*id))
                .map(str::to_owned)
        })
        .collect();
    Ok(SourceEffects {
        changed,
        affected,
        historical_drafts,
    })
}

pub fn prepare_source_change(
    previous: &Value,
    new_source: &Value,
    selections: &BTreeMap<String, Value>,
    instruction_hashes: &BTreeMap<String, String>,
    reason: &str,
    historical_drafts: &BTreeMap<String, Vec<u8>>,
) -> Result<PreparedListChange, TaskIssue> {
    let effects =
        source_change_effects(previous, new_source, selections, instruction_hashes, reason)?;
    let old_rows = previous["tasks"].as_array().expect("validated tasks");
    let mut index = previous.clone();
    index["revision"] = json!(previous["revision"].as_u64().expect("validated revision") + 1);
    index["source"] = new_source.clone();
    let revision = index["revision"].as_u64().expect("new revision");
    let mut drafts = BTreeMap::new();
    for row in index["tasks"].as_array_mut().expect("validated tasks") {
        let id = row["id"].as_str().expect("validated ID").to_owned();
        let old = old_rows
            .iter()
            .find(|old| old["id"] == id)
            .expect("same ID");
        row["instructions_sha256"] = json!(instruction_hashes[&id]);
        row["instruction_selection"] = selections[&id].clone();
        let boundary = row["boundary_revision"]
            .as_u64()
            .expect("validated boundary")
            + u64::from(effects.changed.contains(&id));
        row["boundary_revision"] = json!(boundary);
        if !effects.affected.contains(&id) || old.get("draft_ref").is_none() {
            continue;
        }
        let raw = historical_drafts.get(&id).ok_or_else(|| {
            issue(
                "draft_read_failed",
                "The historical draft could not be read.",
            )
        })?;
        if old["draft_ref"]["sha256"] != sha256_hex(raw) {
            return Err(issue(
                "draft_content_integrity",
                "The historical draft differs from its fingerprint.",
            ));
        }
        let mut draft = parse_json_contract(raw).map_err(|_| {
            issue(
                "invalid_json_contract",
                "The historical draft is invalid JSON.",
            )
        })?;
        if canonical_json(&draft).expect("JSON serializes") != *raw {
            return Err(issue(
                "noncanonical_draft_storage",
                "The historical draft is not canonical.",
            ));
        }
        validate_task_draft(&draft, previous)?;
        draft["source"] = new_source.clone();
        draft["revision"] = json!(draft["revision"].as_u64().expect("validated revision") + 1);
        draft["boundary_revision"] = json!(boundary);
        draft["instructions_sha256"] = row["instructions_sha256"].clone();
        draft["status"] = json!("needs_review");
        draft["notes"]
            .as_array_mut()
            .expect("validated notes")
            .push(json!(format!("Sources updated: {reason}")));
        draft["next_discussion_point"] =
            json!("Reconfirm affected decisions against the updated sources.");
        row["status"] = json!("needs_review");
        let rendered = canonical_json(&draft).expect("JSON serializes");
        row["draft_ref"] = json!({"save_revision":revision,"revision":draft["revision"],"sha256":sha256_hex(&rendered)});
        drafts.insert(id, rendered);
    }
    validate_planning_index(&index)?;
    for raw in drafts.values() {
        let draft = parse_json_contract(raw).map_err(|_| {
            issue(
                "invalid_json_contract",
                "The prepared draft is invalid JSON.",
            )
        })?;
        validate_task_draft(&draft, &index)?;
    }
    Ok(PreparedListChange {
        index,
        drafts,
        affected_task_ids: effects.affected.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_change_marks_only_changed_and_downstream_tasks() {
        let source = json!({"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)});
        let previous = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":1,"source":source,
            "current_task_id":"TASK-001","tasks":[
                {"id":"TASK-001","title":"First","goal":"Goal","scope":["Scope"],"skill_id":null,"dependencies":[],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64)},
                {"id":"TASK-002","title":"Second","goal":"Goal","scope":["Scope"],"skill_id":null,"dependencies":["TASK-001"],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64)}]});
        let selections = BTreeMap::from([
            (
                "TASK-001".into(),
                json!({"selected_paths":[],"references":[]}),
            ),
            (
                "TASK-002".into(),
                json!({"selected_paths":[],"references":[]}),
            ),
        ]);
        let hashes = BTreeMap::from([
            ("TASK-001".into(), "e".repeat(64)),
            ("TASK-002".into(), "d".repeat(64)),
        ]);
        let result = prepare_source_change(
            &previous,
            &source,
            &selections,
            &hashes,
            "Reviewed.",
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(result.affected_task_ids, ["TASK-001", "TASK-002"]);
        assert_eq!(result.index["tasks"][0]["boundary_revision"], 2);
        assert_eq!(result.index["tasks"][1]["boundary_revision"], 1);

        let mut independent = previous.clone();
        independent["tasks"][1]["dependencies"] = json!([]);
        independent["tasks"][0]["instruction_selection"] =
            json!({"selected_paths":[],"references":[]});
        independent["tasks"][1]["instruction_selection"] =
            json!({"selected_paths":[],"references":[]});
        let mut changed_selection = selections.clone();
        changed_selection.insert(
            "TASK-001".into(),
            json!({"selected_paths":[],"references":["task.general.task-records"]}),
        );
        let local = prepare_source_change(
            &independent,
            &source,
            &changed_selection,
            &hashes,
            "Local instruction reviewed.",
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(local.affected_task_ids, ["TASK-001"]);
        assert_eq!(local.index["tasks"][1], independent["tasks"][1]);
        assert_eq!(local.index["tasks"][0]["boundary_revision"], 2);
        assert_eq!(
            local.index["tasks"][0]["instruction_selection"]["references"],
            json!(["task.general.task-records"])
        );
    }

    #[test]
    fn unchanged_source_does_not_prepare_a_write() {
        let source = json!({"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)});
        let selection = json!({"selected_paths":[],"references":[]});
        let previous = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":2,"source":source,
            "current_task_id":"TASK-001","tasks":[{"id":"TASK-001","title":"Task","goal":"Goal","scope":["Scope"],
            "skill_id":null,"dependencies":[],"status":"planned","boundary_revision":1,
            "instructions_sha256":"d".repeat(64),"instruction_selection":selection}]});
        let selections = BTreeMap::from([("TASK-001".into(), selection)]);
        let hashes = BTreeMap::from([("TASK-001".into(), "d".repeat(64))]);
        let Err(error) = prepare_source_change(
            &previous,
            &source,
            &selections,
            &hashes,
            "Reviewed.",
            &BTreeMap::new(),
        ) else {
            panic!("unchanged sources must be rejected");
        };
        assert_eq!(error.reason_code, "draft_sources_unchanged");
    }

    #[test]
    fn plan_refresh_preserves_saved_discussion_and_marks_review() {
        let fixture = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/fixtures/task-assembly"
        ));
        let mut previous: Value =
            serde_json::from_slice(&std::fs::read(fixture.join("index.json")).unwrap()).unwrap();
        let mut draft: Value =
            serde_json::from_slice(&std::fs::read(fixture.join("draft.json")).unwrap()).unwrap();
        draft["notes"] = json!(["Original evidence"]);
        draft["confirmed_decisions"] = json!([{"statement":"Confirmed","rationale":"Reason"}]);
        let raw = canonical_json(&draft).unwrap();
        previous["tasks"][0]["draft_ref"]["sha256"] = json!(sha256_hex(&raw));
        assert_eq!(
            previous["tasks"][0]["draft_ref"]["sha256"],
            sha256_hex(&raw)
        );
        let mut changed_source = previous["source"].clone();
        changed_source["plan_sha256"] = json!("0".repeat(64));
        let selection = previous["tasks"][0]["instruction_selection"].clone();
        let selections = BTreeMap::from([("TASK-001".to_owned(), selection)]);
        let hashes = BTreeMap::from([(
            "TASK-001".to_owned(),
            previous["tasks"][0]["instructions_sha256"]
                .as_str()
                .unwrap()
                .to_owned(),
        )]);
        let historical = BTreeMap::from([("TASK-001".to_owned(), raw)]);
        let result = prepare_source_change(
            &previous,
            &changed_source,
            &selections,
            &hashes,
            "Reviewed Plan change.",
            &historical,
        )
        .unwrap();
        assert_eq!(result.affected_task_ids, ["TASK-001"]);
        assert_eq!(result.index["source"], changed_source);
        assert_eq!(result.index["tasks"][0]["status"], "needs_review");
        let updated = parse_json_contract(&result.drafts["TASK-001"]).unwrap();
        assert_eq!(updated["status"], "needs_review");
        assert_eq!(updated["source"], changed_source);
        assert_eq!(updated["notes"][0], draft["notes"][0]);
        assert_eq!(updated["confirmed_decisions"], draft["confirmed_decisions"]);
    }
}
