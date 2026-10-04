//! Public contract query mapping backed by Rust model data.

use std::sync::OnceLock;

use serde_json::{Value, json};
use work_flow::contract::ContractRegistry;
use work_flow::error::{ExitCode, WorkError};

pub struct EmbeddedRegistry;

impl ContractRegistry for EmbeddedRegistry {
    fn list(&self) -> Value {
        list()
    }
    fn describe(&self, contract_id: &str) -> Result<Value, WorkError> {
        describe(contract_id)
    }
    fn scaffold(&self, contract_id: &str) -> Result<Value, WorkError> {
        scaffold(contract_id)
    }
}

fn registry() -> &'static Value {
    static REGISTRY: OnceLock<Value> = OnceLock::new();
    REGISTRY.get_or_init(work_model::contract_data::registry_value)
}

pub fn scaffold_order(contract_id: &str) -> Option<&'static Value> {
    static ORDERS: OnceLock<Value> = OnceLock::new();
    let orders = ORDERS.get_or_init(work_model::contract_data::scaffold_orders);
    orders.get(contract_id)
}

pub fn list() -> Value {
    let catalog = registry()["catalog"].clone();
    let _: work_model::contract::ContractCatalog =
        serde_json::from_value(catalog.clone()).expect("catalog matches its model");
    catalog
}

pub fn describe(contract_id: &str) -> Result<Value, WorkError> {
    let record = registry()["items"].get(contract_id).ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "unknown_contract_id",
            "The contract ID is not registered.",
            json!({"contract_id":contract_id}),
        )
    })?;
    let description = record["description"].clone();
    let _: work_model::contract::ContractDescription =
        serde_json::from_value(description.clone()).expect("description matches its model");
    Ok(description)
}

pub fn scaffold(contract_id: &str) -> Result<Value, WorkError> {
    let record = registry()["items"].get(contract_id).ok_or_else(|| {
        WorkError::new(
            ExitCode::Contract,
            "unknown_contract_id",
            "The contract ID is not registered.",
            json!({"contract_id":contract_id}),
        )
    })?;
    if let Some(scaffold) = record.get("scaffold") {
        let _: work_model::contract::ContractScaffold =
            serde_json::from_value(scaffold.clone()).expect("scaffold matches its model");
        return Ok(scaffold.clone());
    }
    let issue = &record["scaffold_error"];
    Err(WorkError::new(
        ExitCode::Contract,
        issue["reason_code"].as_str().expect("snapshot reason"),
        issue["message"].as_str().expect("snapshot message"),
        issue["details"].clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_catalog_keeps_public_descriptions_and_scaffolds() {
        assert_eq!(list()["contracts"].as_array().unwrap().len(), 112);
        assert_eq!(
            describe("work-task-semantic-request/v1").unwrap()["kind"],
            "semantic_request"
        );
        assert_eq!(
            scaffold("work-task-semantic-request/v1").unwrap()["schema"],
            "work-contract-scaffold/v1"
        );
        assert_eq!(
            scaffold("work-attempt/v1").unwrap_err().reason_code,
            "contract_scaffold_requires_request"
        );
    }

    #[test]
    fn all_frozen_examples_follow_declared_canonical_fields() {
        for (id, record) in registry()["items"].as_object().unwrap() {
            let description = &record["description"];
            let order = description["canonical_order"].as_array().unwrap();
            let fields: Vec<_> = description["fields"]
                .as_array()
                .unwrap()
                .iter()
                .map(|field| field["name"].clone())
                .collect();
            assert_eq!(order, &fields, "{id}: canonical order");
            let example = description["example"].as_object().unwrap();
            for required in description["required"].as_array().unwrap() {
                assert!(
                    example.contains_key(required.as_str().unwrap()),
                    "{id}: required"
                );
            }
            let projected: Vec<_> = order
                .iter()
                .filter(|field| example.contains_key(field.as_str().unwrap()))
                .collect();
            assert_eq!(projected.len(), example.len(), "{id}: example fields");
            if let Some(schema) = example.get("schema") {
                assert_eq!(schema, id, "{id}: schema");
            }
        }
    }

    #[test]
    fn reconciliation_prepare_example_uses_semantic_positions() {
        let id = "work-spec-reconciliation-prepare-request/v1";
        let description = describe(id).unwrap();
        let example = &description["example"];
        assert_eq!(example["schema"], id);
        assert_eq!(example["task_position"], 1);
        assert_eq!(example["attempt_position"], 1);
        for key in ["attempt_path", "plan_path", "deviation_ids"] {
            assert!(example.get(key).is_none(), "{key}");
        }
        assert_eq!(scaffold(id).unwrap()["example"], *example);
    }

    #[test]
    fn execution_deviation_examples_validate_with_declared_field_order() {
        for id in [
            "work-execution-deviation-proposal/v1",
            "work-execution-deviation/v1",
            "work-execution-deviation-preview/v1",
            "work-execution-deviation-record/v1",
            "work-execution-deviation-semantic-request/v1",
        ] {
            let description = describe(id).unwrap();
            let example = &description["example"];
            work_infrastructure::fixture_support::validate_contract_example(id, example)
                .unwrap_or_else(|error| panic!("{id}: {error}"));
            let order = description["canonical_order"].as_array().unwrap();
            let projected = order
                .iter()
                .filter(|field| example.get(field.as_str().unwrap()).is_some())
                .count();
            assert_eq!(projected, example.as_object().unwrap().len(), "{id}");
        }
        let artifact = describe("work-execution-deviation/v1").unwrap()["example"].clone();
        let authorization = &artifact["supplemental_authorization"];
        assert_eq!(
            authorization["schema"],
            "work-execution-deviation-authorization/v1"
        );
        assert_eq!(
            authorization["preview_sha256"],
            artifact["approved_preview_sha256"]
        );
        assert_eq!(authorization["action"], artifact["proposal"]["action"]);
        assert_eq!(authorization.as_object().unwrap().len(), 5);
        work_infrastructure::fixture_support::validate_contract_example(
            "work-execution-deviation/v1",
            &artifact,
        )
        .unwrap();
    }

    #[test]
    fn public_registry_catalog_scaffolds_and_errors_match_python() {
        let catalog = list();
        assert_eq!(catalog["schema"], "work-contract-catalog/v1");
        let entries = catalog["contracts"].as_array().unwrap();
        let ids: Vec<_> = entries
            .iter()
            .map(|entry| entry["id"].as_str().unwrap())
            .collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted);
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len());
        let kinds: std::collections::BTreeSet<_> = entries
            .iter()
            .map(|entry| entry["kind"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            std::collections::BTreeSet::from([
                "semantic_request",
                "generated_request",
                "response",
                "artifact",
                "envelope"
            ])
        );
        assert_eq!(ids.len(), registry()["items"].as_object().unwrap().len());
        assert!(ids.contains(&"work-contract-description/v1"));
        assert!(ids.contains(&"work-contract-scaffold/v1"));
        for entry in entries {
            let id = entry["id"].as_str().unwrap();
            let description = describe(id).unwrap();
            assert_eq!(description["id"], id);
            assert_eq!(description["kind"], entry["kind"]);
            if entry["kind"] == "semantic_request" {
                let scaffold = scaffold(id).unwrap();
                assert_eq!(scaffold["id"], id);
                assert_eq!(scaffold["canonical_order"], description["canonical_order"]);
                let fields: std::collections::BTreeSet<_> = scaffold["scaffold"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect();
                let expected: std::collections::BTreeSet<_> = description["canonical_order"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|field| field.as_str().unwrap())
                    .collect();
                assert_eq!(fields, expected, "{id}: scaffold fields");
                assert_eq!(scaffold["example"], description["example"]);
            }
        }
        let task = scaffold("work-task-semantic-request/v1").unwrap();
        assert!(task["example"]["upsert"][0].get("id").is_none());
        let generated = "work-spec-update-request/v1";
        let description = describe(generated).unwrap();
        assert_eq!(description["kind"], "generated_request");
        assert_eq!(description["caller_constructible"], false);
        assert_eq!(
            scaffold(generated).unwrap_err().reason_code,
            "generated_request_not_caller_constructible"
        );
        assert_eq!(
            scaffold("work-contract-catalog/v1")
                .unwrap_err()
                .reason_code,
            "contract_scaffold_requires_request"
        );
        let unknown = describe("work-unknown/v1").unwrap_err();
        assert_eq!(unknown.reason_code, "unknown_contract_id");
        assert_eq!(unknown.details, json!({"contract_id":"work-unknown/v1"}));
    }

    #[test]
    fn command_correction_description_discriminates_strict_command_modes() {
        let description = describe("work-command-correction-request/v1").unwrap();
        let command = description["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["name"] == "actual_command")
            .unwrap();
        let schema = &command["constraints"]["schema"];
        assert_eq!(schema["discriminator"]["propertyName"], "mode");
        assert_eq!(schema["oneOf"].as_array().unwrap().len(), 2);
        for (name, required) in [
            ("SemanticArgvCommandModel", json!(["mode", "argv"])),
            ("SemanticShellCommandModel", json!(["mode", "script"])),
        ] {
            assert_eq!(schema["$defs"][name]["required"], required);
            assert_eq!(schema["$defs"][name]["additionalProperties"], false);
        }
    }

    #[test]
    fn hierarchy_instruction_and_skill_examples_keep_canonical_fields() {
        for (id, fields) in [
            (
                "work-hierarchy/v1",
                vec![
                    "schema",
                    "work_directory",
                    "selected_paths",
                    "resolved_paths",
                    "required_paths",
                    "optional_paths",
                ],
            ),
            (
                "work-instruction-catalog/v1",
                vec![
                    "schema",
                    "mode",
                    "paths",
                    "children",
                    "metadata",
                    "catalog_sha256",
                ],
            ),
            (
                "work-skill-bundle/v1",
                vec!["schema", "files", "bundle_sha256"],
            ),
            (
                "work-skill-catalog/v1",
                vec!["schema", "skills", "unavailable"],
            ),
            (
                "work-skill-selection/v1",
                vec!["schema", "decision", "skills", "selection_sha256"],
            ),
            (
                "work-skill-selection-validation/v1",
                vec!["schema", "status", "skill_selection"],
            ),
            ("work-skill-snapshot/v1", vec!["schema", "skill", "bundle"]),
        ] {
            let description = describe(id).unwrap();
            assert_eq!(description["canonical_order"], json!(fields), "{id}");
            assert_eq!(description["required"], json!(fields), "{id}");
            assert_eq!(description["example"]["schema"], id, "{id}");
            let example = description["example"].as_object().unwrap();
            assert_eq!(example.len(), fields.len(), "{id}");
            for field in fields {
                assert!(example.contains_key(field), "{id}: {field}");
            }
        }
    }

    #[test]
    fn delegation_examples_keep_canonical_order_and_optional_projection() {
        for (id, fields) in [
            (
                "work-delegation-build-request/v1",
                vec![
                    "schema",
                    "role",
                    "mode",
                    "request",
                    "planning_source",
                    "task_path",
                    "task_id",
                    "source_progress_path",
                    "content",
                    "confirmed_request",
                    "decisions",
                    "affected_task_ids",
                    "repository_evidence",
                    "saved_discussion",
                    "continuation_point",
                    "save_approval",
                ],
            ),
            (
                "work-delegation-envelope/v1",
                vec![
                    "schema",
                    "marker",
                    "skill",
                    "role",
                    "sender",
                    "mode",
                    "project_root",
                    "skill_root",
                    "request",
                    "context",
                ],
            ),
            (
                "work-delegation-validation/v1",
                vec![
                    "schema",
                    "status",
                    "role",
                    "mode",
                    "scope",
                    "source_validation",
                    "sender_authentication",
                    "grants_authorization",
                ],
            ),
        ] {
            let description = describe(id).unwrap();
            assert_eq!(description["canonical_order"], json!(fields), "{id}");
            assert_eq!(description["example"]["schema"], id, "{id}");
            let example = description["example"].as_object().unwrap();
            let projected: Vec<_> = fields
                .iter()
                .copied()
                .filter(|field| example.contains_key(*field))
                .collect();
            assert_eq!(projected.len(), example.len(), "{id}");
            for required in description["required"].as_array().unwrap() {
                assert!(example.contains_key(required.as_str().unwrap()), "{id}");
            }
        }
        assert_eq!(
            describe("work-delegation-build-request/v1").unwrap()["example"]
                .as_object()
                .unwrap()
                .len(),
            4
        );
    }

    #[test]
    fn operation_result_example_preserves_context_and_success_status() {
        let description = describe("work-operation-result/v1").unwrap();
        let example = &description["example"];
        assert_eq!(example["schema"], "work-operation-result/v1");
        assert_eq!(example["status"], "success");
        assert_eq!(example["context_sha256"], "0".repeat(64));
        for required in description["required"].as_array().unwrap() {
            assert!(example.get(required.as_str().unwrap()).is_some());
        }
    }

    #[test]
    fn specification_request_descriptions_validate_examples_and_nested_references() {
        let prepare = describe("work-spec-prepare-request/v1").unwrap();
        assert_eq!(
            prepare["required"],
            json!(["schema", "requirement_id", "reason", "edits"])
        );
        work_infrastructure::fixture_support::validate_contract_example(
            "work-spec-prepare-request/v1",
            &prepare["example"],
        )
        .unwrap();
        let verify = describe("work-spec-verification-request/v1").unwrap();
        assert_eq!(
            verify["required"],
            json!(["schema", "requirement_id", "artifacts", "record_id"])
        );
        work_infrastructure::fixture_support::validate_contract_example(
            "work-spec-verification-request/v1",
            &verify["example"],
        )
        .unwrap();
        let update = describe("work-spec-update-request/v1").unwrap();
        let references = update["fields"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|field| {
                field["reference"]
                    .as_str()
                    .map(|reference| (field["name"].as_str().unwrap(), reference))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            references,
            std::collections::BTreeMap::from([
                ("task_index", "work-task-index/v1"),
                ("task_items", "work-task-item/v1"),
            ])
        );
        let example = &update["example"];
        assert_eq!(example["schema"], "work-spec-update-request/v1");
        assert!(example.get("plan").is_none());
        assert_eq!(example["task_index"]["schema"], "work-task-index/v1");
        assert_eq!(
            example["task_items"]["TASK-001"]["schema"],
            "work-task-item/v1"
        );
        for value in [
            prepare["example"].clone(),
            verify["example"].clone(),
            example.clone(),
        ] {
            let raw = work_infrastructure::codec::canonical_json(&value).unwrap();
            assert_eq!(
                work_infrastructure::codec::parse_json_contract(&raw).unwrap(),
                value
            );
        }
    }
}
