//! Public hierarchy selection flow over a memory-only catalog.

use std::collections::BTreeMap;

use serde_json::json;
use work_feature::error::WorkError;
use work_feature::hierarchy::{
    HierarchyCatalogRepository, build_selection, validate_selection, validate_task_paths,
};
use work_operations::hierarchy::CrossModeCatalog;

struct FakeCatalog {
    digest: String,
}

impl HierarchyCatalogRepository for FakeCatalog {
    fn cross_mode_catalog(&self) -> Result<CrossModeCatalog, WorkError> {
        Ok(CrossModeCatalog {
            paths: vec!["general".into(), "web".into()],
            children: BTreeMap::from([
                ("general".into(), vec!["web".into()]),
                ("web".into(), vec![]),
            ]),
            metadata: BTreeMap::from([(
                "web".into(),
                json!({"mode_support": ["task", "execute"], "modes": {}}),
            )]),
            catalog_sha256: self.digest.clone(),
        })
    }

    fn mode_paths(&self, mode: &str) -> Result<Vec<String>, WorkError> {
        assert!(matches!(mode, "task" | "execute"));
        Ok(vec!["general".into(), "web".into()])
    }
}

#[test]
fn general_only_round_trip() {
    let repository = FakeCatalog {
        digest: "a".repeat(64),
    };
    let selection = build_selection(
        &repository,
        &json!({"decision":"general_only","selections":[]}),
    )
    .unwrap();
    let validation = validate_selection(&repository, &selection).unwrap();
    assert_eq!(validation["status"], "valid");
    assert_eq!(validation["hierarchy_selection"], selection);
}

#[test]
fn selection_round_trip_and_drift_rejection_use_fake_repository() {
    let repository = FakeCatalog {
        digest: "a".repeat(64),
    };
    let request = json!({
        "decision": "instruction_paths",
        "selections": [{"path": "web", "recommendation_reason": "Needed."}]
    });
    let selection = build_selection(&repository, &request).unwrap();
    assert_eq!(
        validate_selection(&repository, &selection).unwrap()["status"],
        "valid"
    );
    let changed = FakeCatalog {
        digest: "b".repeat(64),
    };
    assert_eq!(
        validate_selection(&changed, &selection)
            .unwrap_err()
            .reason_code,
        "instruction_catalog_snapshot_mismatch"
    );
    let mut tampered = selection.clone();
    tampered["selection_sha256"] = json!("0".repeat(64));
    assert_eq!(
        validate_selection(&repository, &tampered)
            .unwrap_err()
            .reason_code,
        "hierarchy_selection_fingerprint_mismatch"
    );
    validate_task_paths(&repository, &["web".into()], &selection, "TASK-001").unwrap();
}

#[test]
fn request_rejects_unknown_fields_and_missing_catalog_paths() {
    let repository = FakeCatalog {
        digest: "a".repeat(64),
    };
    let invalid = json!({"decision": "general_only", "selections": [], "extra": true});
    assert_eq!(
        build_selection(&repository, &invalid)
            .unwrap_err()
            .reason_code,
        "invalid_object_fields"
    );
    let missing = json!({"decision": "instruction_paths", "selections": [{"path": "unknown", "recommendation_reason": "Reason."}]});
    assert_eq!(
        build_selection(&repository, &missing)
            .unwrap_err()
            .reason_code,
        "hierarchy_selection_path_missing"
    );
}

struct BranchCatalog {
    missing_mode: Option<&'static str>,
}

impl HierarchyCatalogRepository for BranchCatalog {
    fn cross_mode_catalog(&self) -> Result<CrossModeCatalog, WorkError> {
        let paths = branch_paths();
        let metadata = paths
            .iter()
            .filter(|path| path.as_str() != "general")
            .map(|path| {
                let modes = if path.ends_with("/astro") {
                    json!({"task":{"name":"Astro"},"execute":{"name":"Astro"}})
                } else {
                    json!({"task":{"name":"Test"},"task":{"name":"Test"},"execute":{"name":"Test"}})
                };
                let support = if path.ends_with("/astro") {
                    json!(["task", "execute"])
                } else {
                    json!(["task", "task", "execute"])
                };
                (path.clone(), json!({"mode_support":support,"modes":modes}))
            })
            .collect();
        Ok(CrossModeCatalog {
            paths,
            children: BTreeMap::new(),
            metadata,
            catalog_sha256: "a".repeat(64),
        })
    }

    fn mode_paths(&self, mode: &str) -> Result<Vec<String>, WorkError> {
        let mut paths = branch_paths();
        if self.missing_mode == Some(mode) {
            paths.retain(|path| path != "web/frontend/component/astro");
        }
        Ok(paths)
    }
}

fn branch_paths() -> Vec<String> {
    [
        "general",
        "web",
        "web/frontend",
        "web/frontend/component",
        "web/frontend/component/astro",
        "web/frontend/component-extra",
        "web/frontend/css",
        "web/backend",
        "web/backend/java",
        "web/backend/java/jpa",
        "web/backend/java/mybatis",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn choose(repository: &BranchCatalog, path: &str) -> serde_json::Value {
    build_selection(
        repository,
        &json!({"decision":"instruction_paths","selections":[
            {"path":path,"recommendation_reason":"The selected implementation uses this path."}]}),
    )
    .unwrap()
}

#[test]
fn cross_mode_leaf_and_intermediate_selection_match_current_contract() {
    let repository = BranchCatalog { missing_mode: None };
    let leaf = choose(&repository, "web/frontend/component/astro");
    assert_eq!(
        leaf["selected_paths"],
        json!(["web/frontend/component/astro"])
    );
    assert_eq!(
        leaf["entries"][0]["mode_support"],
        json!(["task", "execute"])
    );
    assert_eq!(
        leaf["entries"][0]["mode_metadata"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["execute", "task"]
    );
    assert!(work_operations::protocol::valid_sha256(
        leaf["catalog_sha256"].as_str().unwrap()
    ));
    assert!(work_operations::protocol::valid_sha256(
        leaf["selection_sha256"].as_str().unwrap()
    ));
    let middle = choose(&repository, "web/frontend/component");
    assert_eq!(middle["selected_paths"], json!(["web/frontend/component"]));
    assert_eq!(
        validate_selection(&repository, &middle).unwrap()["status"],
        "valid"
    );
}

#[test]
fn task_path_authorization_checks_ancestors_siblings_and_both_modes() {
    let repository = BranchCatalog { missing_mode: None };
    let leaf = choose(&repository, "web/frontend/component/astro");
    validate_task_paths(
        &repository,
        &["web/frontend/component".into()],
        &leaf,
        "TASK-001",
    )
    .unwrap();
    assert_eq!(
        validate_task_paths(&repository, &["web/frontend/css".into()], &leaf, "TASK-001")
            .unwrap_err()
            .reason_code,
        "task_hierarchy_path_not_authorized"
    );
    let middle = choose(&repository, "web/frontend/component");
    assert_eq!(
        validate_task_paths(
            &repository,
            &["web/frontend/component-extra".into()],
            &middle,
            "TASK-001"
        )
        .unwrap_err()
        .reason_code,
        "task_hierarchy_path_not_authorized"
    );
    let java = choose(&repository, "web/backend/java");
    assert_eq!(
        validate_selection(&repository, &java).unwrap()["status"],
        "valid"
    );
    for path in [
        "web/backend/java",
        "web/backend",
        "web/backend/java/jpa",
        "web/backend/java/mybatis",
    ] {
        validate_task_paths(&repository, &[path.into()], &java, "TASK-001").unwrap();
    }
    for missing_mode in ["task", "execute"] {
        let missing = BranchCatalog {
            missing_mode: Some(missing_mode),
        };
        assert_eq!(
            validate_task_paths(
                &missing,
                &["web/frontend/component/astro".into()],
                &middle,
                "TASK-001"
            )
            .unwrap_err()
            .reason_code,
            "instruction_hierarchy_path_missing"
        );
    }
}

#[test]
fn task_selection_requires_both_task_and_execute_paths() {
    let request = json!({"decision":"instruction_paths","selections":[
        {"path":"web/frontend/component/astro","recommendation_reason":"Uses Astro."}]});
    let valid = BranchCatalog { missing_mode: None };
    let confirmed = build_selection(&valid, &request).unwrap();
    for mode in ["task", "execute"] {
        let missing = BranchCatalog {
            missing_mode: Some(mode),
        };
        for error in [
            build_selection(&missing, &request).unwrap_err(),
            validate_selection(&missing, &confirmed).unwrap_err(),
        ] {
            assert_eq!(error.reason_code, "instruction_hierarchy_path_missing");
            assert_eq!(error.details["mode"], mode);
        }
    }
}
