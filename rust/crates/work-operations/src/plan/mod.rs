//! Shared plan artifact serialization order.

use serde::Serialize;
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde_json::Value;

use crate::protocol::WORKFLOW_MODES;

pub use work_model::plan::{PlanArtifact, PlanStatus};

const TOP_ORDER: &[&str] = &[
    "schema",
    "requirement_id",
    "status",
    "title",
    "summary",
    "artifacts",
    "hierarchy_selection",
    "work_instruction_selection",
    "skill_selection",
    "goals",
    "scope",
    "constraints",
    "dependencies",
    "risks",
    "milestones",
    "deliverables",
    "acceptance_criteria",
    "decisions",
    "changes",
];

struct Ordered<'a> {
    value: &'a Value,
    path: Vec<String>,
}

pub(crate) fn fields(path: &[String]) -> &'static [&'static str] {
    if let Some(order) = crate::instruction_refresh::manifest_field_order(path) {
        return order;
    }
    let Some(last) = path.last().map(String::as_str) else {
        return TOP_ORDER;
    };
    if path.len() >= 2 && path[path.len() - 2] == "mode_metadata" {
        return &["name", "description", "work_tags"];
    }
    match last {
        "artifacts" => &["plan", "task", "execution"],
        "hierarchy_selection" => &[
            "schema",
            "decision",
            "selected_paths",
            "entries",
            "catalog_sha256",
            "selection_sha256",
        ],
        "entries" => &[
            "path",
            "mode_support",
            "mode_metadata",
            "recommendation_reason",
        ],
        "mode_metadata" | "mode_support" => &WORKFLOW_MODES,
        "work_instruction_selection" => &[
            "selected_paths",
            "resolved_paths",
            "sources",
            "references",
            "instructions_sha256",
        ],
        "sources" => &[
            "kind",
            "logical_name",
            "canonical_sha256",
            "compatibility_revision",
        ],
        "skill_selection" => &["schema", "decision", "skills", "selection_sha256"],
        "skills" => &[
            "id",
            "name",
            "scope",
            "root",
            "source",
            "description",
            "mode_support",
            "allow_implicit_invocation",
            "dependency_status",
            "summary_sha256",
            "bundle_sha256",
            "recommendation_reason",
        ],
        "goals" => &["id", "statement"],
        "scope" => &["id", "kind", "statement", "goal_ids"],
        "constraints" | "dependencies" => &["id", "statement", "applies_to"],
        "risks" => &["id", "condition", "impact", "mitigation", "applies_to"],
        "milestones" => &["id", "statement", "deliverable_ids"],
        "deliverables" => &["id", "statement", "goal_ids", "acceptance_ids"],
        "acceptance_criteria" => &["id", "statement", "deliverable_ids"],
        "decisions" => &["id", "statement", "rationale", "applies_to"],
        "changes" => &[
            "id",
            "date",
            "location",
            "before",
            "after",
            "reason",
            "affected_ids",
        ],
        _ => &[],
    }
}

impl Serialize for Ordered<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if let Some(object) = self.value.as_object() {
            let order = fields(&self.path);
            let mut map = serializer.serialize_map(Some(object.len()))?;
            for key in order.iter().copied().chain(
                object
                    .keys()
                    .filter(|key| !order.contains(&key.as_str()))
                    .map(String::as_str),
            ) {
                if let Some(value) = object.get(key) {
                    let mut path = self.path.clone();
                    path.push(key.into());
                    map.serialize_entry(key, &Ordered { value, path })?;
                }
            }
            map.end()
        } else if let Some(values) = self.value.as_array() {
            let mut sequence = serializer.serialize_seq(Some(values.len()))?;
            for value in values {
                sequence.serialize_element(&Ordered {
                    value,
                    path: self.path.clone(),
                })?;
            }
            sequence.end()
        } else {
            self.value.serialize(serializer)
        }
    }
}

pub fn render_plan_value(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec_pretty(&Ordered {
        value,
        path: Vec::new(),
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::sha256_hex;
    use serde_json::Value;

    fn decode_base64(input: &str) -> Vec<u8> {
        let mut result = Vec::new();
        let mut bits = 0_u32;
        let mut count = 0_u8;
        for byte in input.bytes().take_while(|byte| *byte != b'=') {
            let digit = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => panic!("invalid fixture base64"),
            };
            bits = (bits << 6) | u32::from(digit);
            count += 6;
            if count >= 8 {
                count -= 8;
                result.push((bits >> count) as u8);
                bits &= (1 << count) - 1;
            }
        }
        result
    }

    #[test]
    fn migration_plan_artifact_matches_python_bytes() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../../crates/work-operations/fixtures.json"
        ))
        .unwrap();
        let plan: PlanArtifact =
            serde_json::from_value(fixtures["plan_artifact"]["input"].clone()).unwrap();
        let bytes = render_plan_value(&serde_json::to_value(&plan).unwrap()).unwrap();
        assert_eq!(
            bytes,
            decode_base64(fixtures["plan_artifact"]["bytes_base64"].as_str().unwrap())
        );
        assert_eq!(
            sha256_hex(&bytes),
            fixtures["plan_artifact"]["sha256"].as_str().unwrap()
        );
        assert_eq!(plan.schema, "work-plan/v1");
        assert_eq!(
            render_plan_value(&fixtures["plan_artifact"]["input"]).unwrap(),
            bytes
        );
    }

    #[test]
    fn ordering_places_top_artifacts_items_and_unknown_keys_like_python() {
        let value = serde_json::json!({
            "zzz":2,
            "goals":[{"statement":"Goal","id":"GOAL-001"}],
            "artifacts":{"execution":"execution/example","task":"tasks/example/task.json","plan":"plans/example.json"},
            "schema":"work-plan/v1",
            "aaa":1,
        });
        assert_eq!(
            String::from_utf8(render_plan_value(&value).unwrap()).unwrap(),
            "{\n  \"schema\": \"work-plan/v1\",\n  \"artifacts\": {\n    \"plan\": \"plans/example.json\",\n    \"task\": \"tasks/example/task.json\",\n    \"execution\": \"execution/example\"\n  },\n  \"goals\": [\n    {\n      \"id\": \"GOAL-001\",\n      \"statement\": \"Goal\"\n    }\n  ],\n  \"aaa\": 1,\n  \"zzz\": 2\n}\n"
        );
    }

    #[test]
    fn ordering_places_instruction_skill_and_change_details_like_python() {
        assert_eq!(
            &fields(&[])[6..9],
            &[
                "hierarchy_selection",
                "work_instruction_selection",
                "skill_selection"
            ]
        );
        assert_eq!(
            fields(&["work_instruction_selection".into()]),
            &[
                "selected_paths",
                "resolved_paths",
                "sources",
                "references",
                "instructions_sha256"
            ]
        );
        assert_eq!(
            fields(&["work_instruction_selection".into(), "sources".into()]),
            &[
                "kind",
                "logical_name",
                "canonical_sha256",
                "compatibility_revision"
            ]
        );
        assert_eq!(
            fields(&["skill_selection".into()]),
            &["schema", "decision", "skills", "selection_sha256"]
        );
        assert_eq!(
            &fields(&["skill_selection".into(), "skills".into()])[..2],
            &["id", "name"]
        );
        assert_eq!(
            &fields(&["changes".into()])[..7],
            &[
                "id",
                "date",
                "location",
                "before",
                "after",
                "reason",
                "affected_ids"
            ]
        );
    }
}

pub mod validation;
