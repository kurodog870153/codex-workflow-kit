//! Canonical TASK document field order.

use serde::Serialize;
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde_json::Value;

#[derive(Clone, Copy)]
pub enum TaskDocumentKind {
    Collection,
    Index,
    Item,
    RepairRequest,
    SpecificationRequest,
}

const COLLECTION: &[&str] = &[
    "schema",
    "requirement_id",
    "spec_id",
    "status",
    "title",
    "summary",
    "artifacts",
    "source_plan",
    "instruction_selection",
    "execution_defaults",
    "decisions",
    "tasks",
    "changes",
    "readiness",
];
const INDEX: &[&str] = &[
    "schema",
    "requirement_id",
    "spec_id",
    "status",
    "title",
    "summary",
    "artifacts",
    "source_plan",
    "instruction_selection",
    "execution_defaults",
    "tasks",
    "decisions",
    "changes",
    "readiness",
];
const ITEM: &[&str] = &[
    "schema",
    "id",
    "title",
    "skill_id",
    "instruction_selection",
    "traceability",
    "dependencies",
    "inputs",
    "decisions",
    "goal",
    "files",
    "risks",
    "steps",
    "commands",
    "operations",
    "validations",
];
const REPAIR_REQUEST: &[&str] = &[
    "schema",
    "stage",
    "requirement_id",
    "artifacts",
    "expected",
    "decisions",
    "task_index",
    "task_items",
];

struct Ordered<'a> {
    value: &'a Value,
    path: Vec<String>,
    kind: TaskDocumentKind,
}

pub(crate) fn fields(path: &[String], kind: TaskDocumentKind) -> &'static [&'static str] {
    if matches!(kind, TaskDocumentKind::SpecificationRequest) {
        if path.is_empty() {
            return &[
                "schema",
                "reason",
                "expected",
                "plan",
                "task_index",
                "task_items",
            ];
        }
        if path[0] == "plan" {
            return crate::plan::fields(&path[1..]);
        }
        if path == ["expected"] {
            return &[
                "plan_sha256",
                "task_index_sha256",
                "execution_index_sha256",
                "task_item_sha256",
            ];
        }
        if path.len() == 2 && path[0] == "task_items" {
            return &[
                "schema",
                "id",
                "title",
                "skill_id",
                "instruction_selection",
                "traceability",
                "goal",
                "dependencies",
                "inputs",
                "decisions",
                "files",
                "risks",
                "steps",
                "commands",
                "operations",
                "validations",
            ];
        }
        if path == ["task_index", "changes", "edits"] {
            return &[
                "artifact",
                "operation",
                "path",
                "task_id",
                "before",
                "after",
            ];
        }
    }
    if let Some(order) = crate::instruction_refresh::manifest_field_order(path) {
        return order;
    }
    let nested = if path.first().is_some_and(|field| field == "task_index") {
        &path[1..]
    } else {
        path
    };
    if nested == ["changes", "edits", "before"] {
        return ITEM;
    }
    if nested.len() >= 3 && nested[0] == "changes" && nested[1] == "edits" && nested[2] == "after" {
        if nested.len() == 3 {
            return &[
                "schema",
                "id",
                "title",
                "goal",
                "skill_id",
                "instruction_selection",
                "traceability",
                "dependencies",
                "inputs",
                "decisions",
                "files",
                "risks",
                "steps",
                "commands",
                "operations",
                "validations",
            ];
        }
        if nested.len() == 4 {
            let order: Option<&[&str]> = match nested[3].as_str() {
                "steps" => Some(&["action", "references", "id"]),
                "validations" => Some(&[
                    "kind",
                    "command_ids",
                    "pass_condition",
                    "confirmer",
                    "criteria",
                    "id",
                    "acceptance_ids",
                ]),
                "files" => Some(&["action", "path", "source", "destination", "id"]),
                "commands" => Some(&["mode", "argv", "script", "execution", "id"]),
                "inputs" => Some(&["kind", "precondition", "source", "id"]),
                "decisions" => Some(&["statement", "rationale", "id"]),
                "risks" => Some(&["condition", "impact", "mitigation", "id"]),
                "operations" => Some(&[
                    "kind",
                    "action",
                    "target",
                    "validation_id",
                    "command_id",
                    "id",
                ]),
                _ => None,
            };
            if let Some(order) = order {
                return order;
            }
        }
    }
    if matches!(kind, TaskDocumentKind::RepairRequest) {
        if path.len() == 2 && path[0] == "task_items" {
            return ITEM;
        }
        if path == ["task_index", "tasks"] {
            return &["id", "path", "canonical_sha256"];
        }
        if path == ["decisions"] {
            return &["location", "decision"];
        }
    }
    let Some(last) = path.last().map(String::as_str) else {
        return match kind {
            TaskDocumentKind::Collection => COLLECTION,
            TaskDocumentKind::Index => INDEX,
            TaskDocumentKind::Item => ITEM,
            TaskDocumentKind::RepairRequest => REPAIR_REQUEST,
            TaskDocumentKind::SpecificationRequest => &[],
        };
    };
    match last {
        "artifacts" => &["plan", "task", "execution"],
        "source_plan" => &["canonical_sha256", "hierarchy_selection_sha256"],
        "instruction_selection" => &[
            "selected_paths",
            "resolved_paths",
            "sources",
            "references",
            "instructions_sha256",
            "routing_manifest",
        ],
        "sources" => &[
            "kind",
            "logical_name",
            "canonical_sha256",
            "compatibility_revision",
        ],
        "execution_defaults" | "execution" => &["working_directory", "os", "shell"],
        "tasks"
            if matches!(
                kind,
                TaskDocumentKind::Index | TaskDocumentKind::SpecificationRequest
            ) =>
        {
            &["id", "path", "canonical_sha256"]
        }
        "tasks" => ITEM,
        "decisions" if path.len() == 1 => &["id", "statement", "rationale", "task_ids"],
        "decisions" => &["id", "statement", "rationale"],
        "traceability" => &[
            "goal_ids",
            "deliverable_ids",
            "acceptance_ids",
            "milestone_ids",
        ],
        "inputs" => &["id", "kind", "source", "precondition"],
        "files" => &["id", "action", "path", "source", "destination"],
        "risks" => &["id", "condition", "impact", "mitigation"],
        "steps" => &["id", "action", "references"],
        "commands" => &["id", "mode", "argv", "script", "execution"],
        "operations" => &[
            "id",
            "kind",
            "action",
            "target",
            "command_id",
            "validation_id",
        ],
        "validations" => &[
            "id",
            "kind",
            "command_ids",
            "pass_condition",
            "confirmer",
            "criteria",
            "acceptance_ids",
        ],
        "changes" => &[
            "id",
            "spec_id",
            "date",
            "reason",
            "affected_ids",
            "plan_change_ids",
            "edits",
        ],
        "edits" => &[
            "artifact",
            "task_id",
            "operation",
            "path",
            "before",
            "after",
        ],
        "readiness" => &["status", "spec_id"],
        "task_index" => INDEX,
        "task_items" => ITEM,
        "expected" => &[],
        _ => &[],
    }
}

impl Serialize for Ordered<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(object) = self.value.as_object() {
            let priority = fields(&self.path, self.kind);
            let mut output = serializer.serialize_map(Some(object.len()))?;
            for key in priority.iter().copied().chain(
                object
                    .keys()
                    .filter(|key| !priority.contains(&key.as_str()))
                    .map(String::as_str),
            ) {
                if let Some(value) = object.get(key) {
                    let mut path = self.path.clone();
                    path.push(key.into());
                    output.serialize_entry(
                        key,
                        &Ordered {
                            value,
                            path,
                            kind: self.kind,
                        },
                    )?;
                }
            }
            output.end()
        } else if let Some(array) = self.value.as_array() {
            let mut output = serializer.serialize_seq(Some(array.len()))?;
            for value in array {
                output.serialize_element(&Ordered {
                    value,
                    path: self.path.clone(),
                    kind: self.kind,
                })?;
            }
            output.end()
        } else {
            self.value.serialize(serializer)
        }
    }
}

pub fn render_task(value: &Value, kind: TaskDocumentKind) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec_pretty(&Ordered {
        value,
        path: Vec::new(),
        kind,
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::sha256_hex;
    use serde_json::json;

    #[test]
    fn python_item_and_index_example_hashes() {
        let selection = json!({"sources": [{"kind": "instruction", "logical_name": "task.general", "canonical_sha256": "a".repeat(64)}], "references": [], "instructions_sha256": "b".repeat(64)});
        let item = json!({"schema": "work-task-item/v1", "id": "TASK-001", "title": "Example", "skill_id": null,
            "instruction_selection": {"selected_paths": [], "resolved_paths": ["general"], "sources": selection["sources"], "references": [], "instructions_sha256": "b".repeat(64)},
            "traceability": {"goal_ids": ["GOAL-001"], "deliverable_ids": ["DELIVERABLE-001"], "acceptance_ids": ["ACCEPTANCE-001"]},
            "goal": "Produce the result.", "steps": [{"id": "STEP-001", "action": "Validate.", "references": ["VAL-001"]}],
            "validations": [{"id": "VAL-001", "kind": "manual", "confirmer": "user", "criteria": "Approved."}]});
        let bytes = render_task(&item, TaskDocumentKind::Item).unwrap();
        assert_eq!(bytes.len(), 1007);
        assert_eq!(
            sha256_hex(&bytes),
            "3faacb1670d84a424411bed7735139814b85a48605ab9241efc00a32b9f93c49"
        );
        let index = json!({"schema": "work-task-index/v1", "requirement_id": "example", "spec_id": "TASK-SPEC-001", "status": "confirmed", "title": "Example", "summary": "Example tasks.",
            "artifacts": {"plan": "outputs/work/plans/example.json", "task": "outputs/work/tasks/example/index.json", "execution": "outputs/work/executions/example"},
            "source_plan": {"canonical_sha256": "c".repeat(64), "hierarchy_selection_sha256": "d".repeat(64)},
            "instruction_selection": selection,
            "tasks": [{"id": "TASK-001", "path": "tasks/TASK-001.json", "canonical_sha256": "e".repeat(64)}],
            "readiness": {"status": "passed", "spec_id": "TASK-SPEC-001"}});
        let bytes = render_task(&index, TaskDocumentKind::Index).unwrap();
        assert_eq!(bytes.len(), 1183);
        assert_eq!(
            sha256_hex(&bytes),
            "b69610683e732a49ce03ce1f828d0f355920830e10a797191aaf75192f92c1c8"
        );
    }

    #[test]
    fn collection_orders_known_and_unknown_fields_like_python() {
        let value = json!({"zzz":2,"readiness":{"spec_id":"TASK-SPEC-001","status":"ready"},
            "tasks":[],"schema":"work-task-collection-projection/v1","aaa":1});
        assert_eq!(
            String::from_utf8(render_task(&value, TaskDocumentKind::Collection).unwrap()).unwrap(),
            "{\n  \"schema\": \"work-task-collection-projection/v1\",\n  \"tasks\": [],\n  \"readiness\": {\n    \"status\": \"ready\",\n    \"spec_id\": \"TASK-SPEC-001\"\n  },\n  \"aaa\": 1,\n  \"zzz\": 2\n}\n"
        );
    }

    #[test]
    fn collection_nested_field_order_matches_python() {
        let kind = TaskDocumentKind::Collection;
        assert_eq!(
            fields(&[], TaskDocumentKind::Index)[8],
            "instruction_selection"
        );
        let document = json!({"instruction_selection": {
            "instructions_sha256": "a".repeat(64), "references": [], "sources": []
        }});
        let rendered =
            String::from_utf8(render_task(&document, TaskDocumentKind::Index).unwrap()).unwrap();
        let sources = rendered.find("\"sources\"").unwrap();
        let references = rendered.find("\"references\"").unwrap();
        let fingerprint = rendered.find("\"instructions_sha256\"").unwrap();
        assert!(sources < references && references < fingerprint);
        assert_eq!(
            &fields(&["tasks".into()], kind)[..5],
            &["schema", "id", "title", "skill_id", "instruction_selection"]
        );
        assert_eq!(
            &fields(&["tasks".into(), "instruction_selection".into()], kind)[..5],
            &[
                "selected_paths",
                "resolved_paths",
                "sources",
                "references",
                "instructions_sha256"
            ]
        );
        assert_eq!(
            &fields(&["tasks".into(), "commands".into()], kind)[..5],
            &["id", "mode", "argv", "script", "execution"]
        );
        assert_eq!(
            fields(
                &["tasks".into(), "commands".into(), "execution".into()],
                kind
            ),
            &["working_directory", "os", "shell"]
        );
        assert_eq!(
            &fields(&["tasks".into(), "validations".into()], kind)[..3],
            &["id", "kind", "command_ids"]
        );
        assert_eq!(
            &fields(&["changes".into()], kind)[..7],
            &[
                "id",
                "spec_id",
                "date",
                "reason",
                "affected_ids",
                "plan_change_ids",
                "edits"
            ]
        );
        assert_eq!(
            &fields(&["changes".into(), "edits".into()], kind)[..6],
            &[
                "artifact",
                "task_id",
                "operation",
                "path",
                "before",
                "after"
            ]
        );
    }
}
