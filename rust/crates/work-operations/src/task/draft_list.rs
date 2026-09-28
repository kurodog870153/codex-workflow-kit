//! Pure TASK boundary list updates and affected discussion derivation.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::canonical::{JsonContractIssue, canonical_json, parse_json_contract, sha256_hex};
use crate::protocol::TASK_ID_PREFIX;
use crate::task::TaskIssue;
use crate::task::draft::{validate_planning_index, validate_task_draft};

fn issue(reason_code: &'static str, message: &'static str) -> TaskIssue {
    TaskIssue {
        reason_code,
        message,
        details: json!({}),
    }
}

pub struct PreparedListChange {
    pub index: Value,
    pub drafts: BTreeMap<String, Vec<u8>>,
    pub affected_task_ids: Vec<String>,
}

pub fn affected_historical_draft_ids(
    previous: &Value,
    proposed: &Value,
) -> Result<BTreeSet<String>, TaskIssue> {
    validate_planning_index(previous)?;
    validate_planning_index(proposed)?;
    let old: BTreeMap<String, &Value> = previous["tasks"]
        .as_array()
        .expect("validated tasks")
        .iter()
        .map(|row| (row["id"].as_str().expect("validated ID").to_owned(), row))
        .collect();
    let new: BTreeMap<String, &Value> = proposed["tasks"]
        .as_array()
        .expect("validated tasks")
        .iter()
        .map(|row| (row["id"].as_str().expect("validated ID").to_owned(), row))
        .collect();
    let boundaries = [
        "title",
        "goal",
        "scope",
        "skill_id",
        "dependencies",
        "instructions_sha256",
    ];
    let mut affected: BTreeSet<String> = old
        .keys()
        .filter(|id| !new.contains_key(*id))
        .cloned()
        .chain(new.keys().filter(|id| !old.contains_key(*id)).cloned())
        .chain(
            old.keys()
                .filter(|id| {
                    new.contains_key(*id)
                        && boundaries
                            .iter()
                            .any(|field| old[*id][*field] != new[*id][*field])
                })
                .cloned(),
        )
        .collect();
    loop {
        let downstream: BTreeSet<String> = old
            .iter()
            .chain(new.iter())
            .filter(|(_, entry)| {
                entry["dependencies"].as_array().is_some_and(|deps| {
                    deps.iter()
                        .any(|dep| dep.as_str().is_some_and(|id| affected.contains(id)))
                })
            })
            .map(|(id, _)| id.clone())
            .collect();
        let len = affected.len();
        affected.extend(downstream);
        if len == affected.len() {
            break;
        }
    }
    Ok(affected
        .into_iter()
        .filter(|id| {
            old.get(id)
                .is_some_and(|row| row.get("draft_ref").is_some())
                && new.contains_key(id)
        })
        .collect())
}

pub fn prepare_list_change(
    previous: &Value,
    proposed: &Value,
    reason: &str,
    historical_drafts: &BTreeMap<String, Vec<u8>>,
) -> Result<PreparedListChange, TaskIssue> {
    validate_planning_index(previous)?;
    validate_planning_index(proposed)?;
    if reason.trim().is_empty() {
        return Err(issue("empty_text_value", "A non-empty reason is required."));
    }
    if proposed["requirement_id"] != previous["requirement_id"]
        || proposed["source"] != previous["source"]
    {
        return Err(issue(
            "draft_scope_changed",
            "A list update cannot change its requirement or source snapshot.",
        ));
    }
    if proposed["revision"].as_u64() != previous["revision"].as_u64().map(|revision| revision + 1) {
        return Err(issue(
            "draft_revision_conflict",
            "The list revision must immediately follow the previous index.",
        ));
    }
    let old: BTreeMap<String, &Value> = previous["tasks"]
        .as_array()
        .expect("validated tasks")
        .iter()
        .map(|entry| {
            (
                entry["id"].as_str().expect("validated ID").to_owned(),
                entry,
            )
        })
        .collect();
    let new: BTreeMap<String, &Value> = proposed["tasks"]
        .as_array()
        .expect("validated tasks")
        .iter()
        .map(|entry| {
            (
                entry["id"].as_str().expect("validated ID").to_owned(),
                entry,
            )
        })
        .collect();
    for id in old.keys().filter(|id| new.contains_key(*id)) {
        if old[id].get("instruction_selection") != new[id].get("instruction_selection") {
            return Err(issue(
                "draft_selection_mismatch",
                "List updates must preserve existing instruction selections; use source-update to change them.",
            ));
        }
    }
    let retired: BTreeSet<String> = previous["retired_task_ids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect();
    let added: BTreeSet<String> = new
        .keys()
        .filter(|id| !old.contains_key(*id))
        .cloned()
        .collect();
    let removed: BTreeSet<String> = old
        .keys()
        .filter(|id| !new.contains_key(*id))
        .cloned()
        .collect();
    let highest = old
        .keys()
        .chain(retired.iter())
        .filter_map(|id| {
            id.strip_prefix(TASK_ID_PREFIX)
                .and_then(|number| number.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0);
    if added
        .iter()
        .any(|id| id[5..].parse::<u32>().is_ok_and(|number| number <= highest))
    {
        return Err(issue(
            "draft_task_id_reused",
            "New TASK IDs must exceed all previously allocated IDs.",
        ));
    }
    let boundaries = [
        "title",
        "goal",
        "scope",
        "skill_id",
        "dependencies",
        "instructions_sha256",
    ];
    let changed: BTreeSet<String> = old
        .keys()
        .filter(|id| {
            new.contains_key(*id)
                && boundaries
                    .iter()
                    .any(|field| old[*id][*field] != new[*id][*field])
        })
        .cloned()
        .collect();
    if added.is_empty() && removed.is_empty() && changed.is_empty() {
        return Err(issue(
            "draft_list_unchanged",
            "A list update requires a boundary, addition or removal change.",
        ));
    }
    let mut affected: BTreeSet<String> = added
        .union(&removed)
        .cloned()
        .chain(changed.iter().cloned())
        .collect();
    loop {
        let downstream: BTreeSet<String> = old
            .iter()
            .chain(new.iter())
            .filter(|(_, entry)| {
                entry["dependencies"].as_array().is_some_and(|deps| {
                    deps.iter()
                        .any(|dep| dep.as_str().is_some_and(|dep| affected.contains(dep)))
                })
            })
            .map(|(id, _)| id.clone())
            .collect();
        let old_len = affected.len();
        affected.extend(downstream);
        if affected.len() == old_len {
            break;
        }
    }
    let mut result = proposed.clone();
    let new_retired: BTreeSet<String> = retired.union(&removed).cloned().collect();
    result["retired_task_ids"] = json!(new_retired);
    let mut drafts = BTreeMap::new();
    let revision = result["revision"].as_u64().expect("validated revision");
    for entry in result["tasks"].as_array_mut().expect("validated tasks") {
        let id = entry["id"].as_str().expect("validated ID").to_owned();
        if added.contains(&id) {
            if entry["status"] != "planned"
                || entry["boundary_revision"] != 1
                || entry.get("draft_ref").is_some()
            {
                return Err(issue(
                    "invalid_new_draft_task",
                    "New TASKs must be planned at boundary revision 1 without a draft.",
                ));
            }
            continue;
        }
        let original = old[&id];
        if ["status", "boundary_revision", "draft_ref"]
            .iter()
            .any(|field| entry.get(*field) != original.get(*field))
        {
            return Err(issue(
                "draft_list_metadata_changed",
                "Preserve original progress metadata in the list proposal.",
            ));
        }
        if !affected.contains(&id) {
            continue;
        }
        let increment = u64::from(changed.contains(&id));
        entry["boundary_revision"] = json!(
            entry["boundary_revision"]
                .as_u64()
                .expect("validated boundary revision")
                + increment
        );
        let Some(reference) = original.get("draft_ref") else {
            continue;
        };
        let raw = historical_drafts.get(&id).ok_or_else(|| {
            issue(
                "draft_read_failed",
                "The affected historical draft could not be read.",
            )
        })?;
        if reference["sha256"] != sha256_hex(raw) {
            return Err(issue(
                "draft_content_integrity",
                "The affected historical draft differs from its fingerprint.",
            ));
        }
        let mut discussion = parse_json_contract(raw).map_err(|_| {
            issue(
                "invalid_json_contract",
                "The historical draft is invalid JSON.",
            )
        })?;
        if canonical_json(&discussion).expect("JSON value serializes") != *raw {
            return Err(issue(
                "noncanonical_draft_storage",
                "The stored JSON is not canonical.",
            ));
        }
        if discussion["task_id"] != id {
            return Err(issue(
                "draft_task_mismatch",
                "The historical draft belongs to another TASK.",
            ));
        }
        validate_task_draft(&discussion, previous)?;
        discussion["revision"] =
            json!(discussion["revision"].as_u64().expect("validated revision") + 1);
        discussion["boundary_revision"] = entry["boundary_revision"].clone();
        discussion["instructions_sha256"] = entry["instructions_sha256"].clone();
        discussion["status"] = json!("needs_review");
        discussion["notes"]
            .as_array_mut()
            .expect("validated notes")
            .push(json!(format!("TASK list changed: {reason}")));
        discussion["next_discussion_point"] = json!(format!(
            "Reconfirm the affected discussion after the TASK list change: {reason}"
        ));
        entry["status"] = json!("needs_review");
        entry
            .as_object_mut()
            .expect("validated entry")
            .remove("draft_ref");
        let rendered = canonical_json(&discussion).expect("JSON value serializes");
        entry["draft_ref"] = json!({"save_revision":revision,"revision":discussion["revision"],"sha256":sha256_hex(&rendered)});
        drafts.insert(id, rendered);
    }
    validate_planning_index(&result)?;
    for raw in drafts.values() {
        let discussion = parse_json_contract(raw).map_err(|issue| match issue {
            JsonContractIssue::DuplicateKey(_) => issue_task("duplicate_json_key"),
            _ => issue_task("invalid_json_contract"),
        })?;
        validate_task_draft(&discussion, &result)?;
    }
    let affected_task_ids: Vec<String> = affected
        .into_iter()
        .filter(|id| new.contains_key(id))
        .collect();
    Ok(PreparedListChange {
        index: result,
        drafts,
        affected_task_ids,
    })
}

fn issue_task(reason_code: &'static str) -> TaskIssue {
    issue(reason_code, "A stored draft could not be parsed.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_change_marks_saved_draft_and_downstream_task() {
        let source = json!({"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)});
        let discussion = json!({"schema":"work-task-draft/v1","requirement_id":"example","task_id":"TASK-001","revision":1,
            "boundary_revision":1,"source":source,"instructions_sha256":"d".repeat(64),"status":"refined",
            "notes":["Detail"],"confirmed_decisions":[{"statement":"Decision","rationale":"Reason"}],"tentative":[],"open_questions":[],"next_discussion_point":null});
        let raw = canonical_json(&discussion).unwrap();
        let previous = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":2,"source":source,
            "current_task_id":"TASK-001","tasks":[
                {"id":"TASK-001","title":"Task one","goal":"Original","scope":["Source"],"skill_id":null,"dependencies":[],
                 "status":"refined","boundary_revision":1,"instructions_sha256":"d".repeat(64),"draft_ref":{"save_revision":2,"revision":1,"sha256":sha256_hex(&raw)}},
                {"id":"TASK-002","title":"Task two","goal":"Follow up","scope":["Source"],"skill_id":null,"dependencies":["TASK-001"],
                 "status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64)}]});
        let mut proposed = previous.clone();
        proposed["revision"] = json!(3);
        proposed["tasks"][0]["goal"] = json!("Changed");
        assert_eq!(
            affected_historical_draft_ids(&previous, &proposed).unwrap(),
            BTreeSet::from(["TASK-001".into()])
        );
        let mut independent = previous.clone();
        independent["tasks"][1]["dependencies"] = json!([]);
        independent["tasks"][1]["status"] = json!("refined");
        independent["tasks"][1]["draft_ref"] =
            json!({"save_revision":2,"revision":1,"sha256":"e".repeat(64)});
        let mut independent_next = independent.clone();
        independent_next["revision"] = json!(3);
        independent_next["tasks"][0]["goal"] = json!("Changed");
        assert_eq!(
            affected_historical_draft_ids(&independent, &independent_next).unwrap(),
            BTreeSet::from(["TASK-001".into()])
        );
        let historical = BTreeMap::from([("TASK-001".into(), raw)]);
        let result =
            prepare_list_change(&previous, &proposed, "Confirmed list change.", &historical)
                .unwrap();
        assert_eq!(result.affected_task_ids, vec!["TASK-001", "TASK-002"]);
        assert_eq!(result.index["tasks"][0]["boundary_revision"], 2);
        assert_eq!(result.index["tasks"][0]["status"], "needs_review");
        assert_eq!(result.index["tasks"][1]["status"], "planned");
        let updated = parse_json_contract(&result.drafts["TASK-001"]).unwrap();
        assert_eq!(updated["confirmed_decisions"][0]["statement"], "Decision");
    }

    #[test]
    fn three_task_boundary_split_and_merge_keep_unaffected_drafts() {
        let source = json!({"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)});
        let mut drafts = BTreeMap::new();
        let mut tasks = Vec::new();
        for number in 1..=3 {
            let id = format!("TASK-{number:03}");
            let discussion = json!({"schema":"work-task-draft/v1","requirement_id":"example","task_id":id,
                "revision":1,"boundary_revision":1,"source":source,"instructions_sha256":"d".repeat(64),
                "status":"refined","notes":["Detail"],"confirmed_decisions":[{"statement":"Decision","rationale":"Reason"}],
                "tentative":[],"open_questions":[],"next_discussion_point":null});
            let raw = canonical_json(&discussion).unwrap();
            tasks.push(
                json!({"id":id,"title":"Task","goal":"Result","scope":["Source"],"skill_id":null,
                "dependencies":if number == 2 {vec!["TASK-001"]} else {vec![]},
                "status":"refined","boundary_revision":1,"instructions_sha256":"d".repeat(64),
                "draft_ref":{"save_revision":number+1,"revision":1,"sha256":sha256_hex(&raw)}}),
            );
            drafts.insert(id, raw);
        }
        let previous = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":4,
            "current_task_id":"TASK-001","source":source,"tasks":tasks});

        let mut boundary = previous.clone();
        boundary["revision"] = json!(5);
        boundary["tasks"][0]["goal"] = json!("Changed result");
        let updated =
            prepare_list_change(&previous, &boundary, "Confirmed list change.", &drafts).unwrap();
        assert_eq!(updated.affected_task_ids, vec!["TASK-001", "TASK-002"]);
        assert_eq!(updated.index["tasks"][2], previous["tasks"][2]);
        assert_eq!(updated.index["tasks"][0]["boundary_revision"], 2);
        assert_eq!(updated.index["tasks"][1]["boundary_revision"], 1);
        for id in ["TASK-001", "TASK-002"] {
            let discussion = parse_json_contract(&updated.drafts[id]).unwrap();
            assert_eq!(discussion["status"], "needs_review");
            assert_eq!(
                discussion["confirmed_decisions"][0]["statement"],
                "Decision"
            );
        }

        let mut split = previous.clone();
        split["revision"] = json!(5);
        let template = split["tasks"].as_array_mut().unwrap().remove(0);
        split["current_task_id"] = json!("TASK-004");
        split["tasks"][0]["dependencies"] = json!(["TASK-004", "TASK-005"]);
        for id in ["TASK-004", "TASK-005"] {
            let mut entry = template.clone();
            entry["id"] = json!(id);
            entry["status"] = json!("planned");
            entry["boundary_revision"] = json!(1);
            entry.as_object_mut().unwrap().remove("draft_ref");
            split["tasks"].as_array_mut().unwrap().push(entry);
        }
        let split_result =
            prepare_list_change(&previous, &split, "Confirmed split.", &drafts).unwrap();
        assert_eq!(split_result.index["retired_task_ids"], json!(["TASK-001"]));
        assert_eq!(split_result.index["tasks"][0]["status"], "needs_review");
        assert_eq!(split_result.index["tasks"][1], previous["tasks"][2]);
        assert_eq!(split_result.index["tasks"][2]["boundary_revision"], 1);
        assert_eq!(split_result.index["tasks"][3]["boundary_revision"], 1);

        let mut merged = previous.clone();
        merged["revision"] = json!(5);
        merged["tasks"].as_array_mut().unwrap().remove(0);
        merged["tasks"][0]["goal"] = json!("Combined result");
        merged["tasks"][0]["dependencies"] = json!([]);
        merged["current_task_id"] = json!("TASK-002");
        let merge_result =
            prepare_list_change(&previous, &merged, "Confirmed merge.", &drafts).unwrap();
        assert_eq!(merge_result.index["retired_task_ids"], json!(["TASK-001"]));
        assert_eq!(merge_result.index["tasks"][1], previous["tasks"][2]);
    }
}
