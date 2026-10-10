//! Public Session contract records and command definitions.

use crate::contract::ContractRecord;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const COMMANDS: &[&str] = &[
    "init",
    "read",
    "status",
    "update",
    "question",
    "answer",
    "history",
    "recover",
    "preview",
    "apply",
    "recover-publication",
];

pub fn records() -> BTreeMap<String, ContractRecord> {
    let session = json!({"schema":"work-discussion-session","requirement_id":"example","revision":1,
        "context":{"original_references":["source.txt"],"goal":"Deliver the requirement","scope":["Confirmed scope"],"constraints":[],"acceptance_criteria":[],"confirmed_source":null,"work_type":"task","revision":1},
        "authorization":{"requirement_id":"example","project_root":"/project","directory":"outputs/work/discussions/example","allowed_actions":["add_decision","update_decision","update_planning","prepare_question","answer_question","update_context","update_continuation"],"evidence":"User authorized saves within this requirement","revoked_reason":null},
        "tasks":[],"retired_task_ids":[],"decisions":[],"next_task_number":1,"next_decision_number":1,
        "continuation":{"current_task_id":null,"question":null},"commit":{"operation_id":"init","operation_sha256":"a".repeat(64),"previous_sha256":null,"content_sha256":"b".repeat(64)}});
    let request = json!({"schema":"work-discussion-request","requirement_id":"example","command":{"kind":"read"}});
    let result = json!({"schema":"work-discussion-result","command":"read","requirement_id":"example","data":{"revision":1}});
    let mut records = BTreeMap::new();
    for (id, kind, constructible, example, order) in [
        (
            "work-discussion-session",
            "artifact",
            false,
            session,
            vec![
                "schema",
                "requirement_id",
                "revision",
                "context",
                "authorization",
                "tasks",
                "retired_task_ids",
                "decisions",
                "next_task_number",
                "next_decision_number",
                "continuation",
                "commit",
            ],
        ),
        (
            "work-discussion-request",
            "semantic_request",
            true,
            request,
            vec!["schema", "requirement_id", "command"],
        ),
        (
            "work-discussion-result",
            "response",
            false,
            result,
            vec!["schema", "command", "requirement_id", "data"],
        ),
    ] {
        let fields:Vec<_>=order.iter().map(|field| {
            let constraints=match *field {
                "command" if constructible => json!({"discriminator":"kind","variants":COMMANDS,"additional_properties":false,"read_optional_fields":{"decision_id":"string: return only this current decision, including options, resolution and evidence"}}),
                "revision" | "next_task_number" | "next_decision_number" => json!({"minimum":1}),
                "authorization" => json!({"bounded_requirement":true,"canonical_project_root":true,"exact_directory":"outputs/work/discussions/<requirement-id>","evidence_required":true,"publication_authorized":false,"execute_authorized":false}),
                "decisions" => json!({"statuses":["pending","confirmed","deferred","blocked","needs_review","withdrawn"],"stable_id":"D001","unknown_fields":"reject"}),
                "tasks" => json!({"stable_id":"TASK-001","domain_records":true,"task_candidate":"forbidden","review":{"granularity_required_for_publication":true,"split_decisions":["single_outcome","indivisible","split_required","needs_confirmation"],"planning_sha256":"context, relevant decisions and upstream plans excluding reviews","semantic":{"required_for_publication":true,"outcomes":"explicit statement, acceptance_ids, file_keys, scope, independently_acceptable, needs_confirmation and evidence","indivisible":"coupled_outcome_ids and concrete separation_consequence","natural_language_truth":"reviewed authority; deterministic validator checks complete mappings and rejects multiple independent outcomes"}}}),
                "continuation" => json!({"required_nullable":["current_task_id","question"],"question_mapping_saved_before_asking":true}),
                "commit" => json!({"immutable_revision":true,"sole_commit_point":"current session.json","required_nullable":["previous_sha256"],"content_sha256":"excludes only itself"}),
                _ => json!({}),
            };
            json!({"name":field,"type":match example[*field] { Value::String(_) => "string",Value::Number(_) => "integer",Value::Array(_) if *field == "retired_task_ids" => "array<string>",Value::Array(_) => "array<object>",_ => "object" },
                "required":true,"reference":null,"constraints":constraints})
        }).collect();
        let description = json!({"schema":"work-contract-description","id":id,"kind":kind,"caller_constructible":constructible,
            "required":order,"optional":[],"canonical_order":order,"fields":fields,"example":example});
        let record = if constructible {
            json!({"description":description,"scaffold":{"schema":"work-contract-scaffold","id":id,"canonical_order":order,"scaffold":example,"example":example}})
        } else {
            json!({"description":description,"scaffold_error":{"reason_code":"contract_scaffold_requires_request","message":"Only semantic request contracts have public input scaffolds.","details":{"contract_id":id,"kind":kind}}})
        };
        records.insert(
            id.into(),
            serde_json::from_value(record).expect("public record matches model"),
        );
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_records_round_trip_with_exact_schema() {
        let records = records();
        serde_json::from_value::<super::super::DiscussionSession>(
            records["work-discussion-session"]
                .description
                .example
                .clone(),
        )
        .unwrap();
        serde_json::from_value::<super::super::request::DiscussionRequest>(
            records["work-discussion-request"]
                .description
                .example
                .clone(),
        )
        .unwrap();
        assert_eq!(records.len(), 3);
        serde_json::from_value::<super::super::response::DiscussionResult>(
            records["work-discussion-result"]
                .description
                .example
                .clone(),
        )
        .unwrap();
        assert!(records["work-discussion-request"].scaffold.is_some());
        assert_eq!(COMMANDS.len(), 11);
    }
}
