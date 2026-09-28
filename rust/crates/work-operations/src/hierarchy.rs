//! Pure hierarchy path, selection, and authorization rules.

use std::collections::BTreeSet;

use serde::Serialize;
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde_json::{Value, json};

use crate::canonical::canonical_json_sha256;
use crate::protocol::WORKFLOW_MODES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchyIssue {
    pub reason_code: &'static str,
    pub message: &'static str,
    pub details: Value,
}

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> HierarchyIssue {
    HierarchyIssue {
        reason_code,
        message,
        details,
    }
}

pub use work_model::hierarchy::Hierarchy;

pub fn sorted_paths(paths: BTreeSet<String>) -> Vec<String> {
    let mut paths: Vec<_> = paths.into_iter().collect();
    paths.sort_by_key(|path| (path != "general", path.clone()));
    paths
}

pub fn valid_segment(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub fn build_hierarchy(
    work_directory: &str,
    selected_paths: &[String],
) -> Result<Hierarchy, HierarchyIssue> {
    if !matches!(work_directory, "plan" | "task" | "execute") {
        return Err(issue(
            "invalid_work_directory",
            "The work directory must be plan, task, or execute.",
            json!({"work_directory": work_directory}),
        ));
    }
    for path in selected_paths {
        if path.is_empty()
            || path
                .split('/')
                .any(|part| part == "general" || !valid_segment(part))
        {
            return Err(issue(
                "invalid_hierarchy_path",
                "A selected hierarchy path must contain lowercase kebab-case segments and cannot include general.",
                json!({"path": path}),
            ));
        }
    }
    let mut unique = std::collections::HashSet::new();
    if selected_paths.iter().any(|path| !unique.insert(path)) {
        return Err(issue(
            "duplicate_hierarchy_path",
            "Selected hierarchy paths must be unique.",
            json!({"selected_paths": selected_paths}),
        ));
    }
    for path in selected_paths {
        let prefix = format!("{path}/");
        let descendants: Vec<_> = selected_paths
            .iter()
            .filter(|candidate| candidate.starts_with(&prefix))
            .collect();
        if !descendants.is_empty() {
            return Err(issue(
                "redundant_hierarchy_path",
                "A selected hierarchy path cannot be an ancestor of another selected path.",
                json!({"path": path, "descendants": descendants}),
            ));
        }
    }
    let mut resolved_paths = vec!["general".to_owned()];
    for selected in selected_paths {
        let parts: Vec<_> = selected.split('/').collect();
        for depth in 1..=parts.len() {
            let path = parts[..depth].join("/");
            if !resolved_paths.contains(&path) {
                resolved_paths.push(path);
            }
        }
    }
    let required_paths: Vec<_> = resolved_paths
        .iter()
        .filter(|path| path.as_str() == "general" || selected_paths.contains(path))
        .cloned()
        .collect();
    let optional_paths: Vec<_> = resolved_paths
        .iter()
        .filter(|path| !required_paths.contains(path))
        .cloned()
        .collect();
    Ok(Hierarchy {
        schema: "work-hierarchy/v1".into(),
        work_directory: work_directory.into(),
        selected_paths: selected_paths.to_vec(),
        resolved_paths,
        required_paths,
        optional_paths,
    })
}

pub use work_model::hierarchy::CrossModeCatalog;

pub fn selection_sha256(
    decision: &str,
    selected_paths: &[String],
    entries: &[Value],
    catalog_sha256: &str,
) -> String {
    canonical_json_sha256(&json!({
        "decision": decision,
        "selected_paths": selected_paths,
        "entries": entries,
        "catalog_sha256": catalog_sha256,
    }))
    .expect("serializing JSON values cannot fail")
}

pub fn authorize_task_paths(
    selected_paths: &[String],
    confirmed_paths: &[String],
) -> Result<(), HierarchyIssue> {
    let mut allowed = std::collections::HashSet::new();
    for path in confirmed_paths {
        let parts: Vec<_> = path.split('/').collect();
        for depth in 1..=parts.len() {
            allowed.insert(parts[..depth].join("/"));
        }
    }
    let unauthorized: Vec<_> = selected_paths
        .iter()
        .filter(|path| {
            !allowed.contains(path.as_str())
                && !confirmed_paths
                    .iter()
                    .any(|confirmed| path.starts_with(&format!("{confirmed}/")))
        })
        .collect();
    if !unauthorized.is_empty() {
        return Err(issue(
            "task_hierarchy_path_not_authorized",
            "A TASK hierarchy path is not authorized by the source Plan.",
            json!({"paths": unauthorized}),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum OrderKind {
    Selection,
    Entry,
    Modes,
    Metadata,
    Plain,
}

struct Ordered<'a> {
    value: &'a Value,
    kind: OrderKind,
}

impl Serialize for Ordered<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let Some(object) = self.value.as_object() else {
            return self.value.serialize(serializer);
        };
        let priority: &[&str] = match self.kind {
            OrderKind::Selection => &[
                "schema",
                "decision",
                "selected_paths",
                "entries",
                "catalog_sha256",
                "selection_sha256",
            ],
            OrderKind::Entry => &[
                "path",
                "mode_support",
                "mode_metadata",
                "recommendation_reason",
            ],
            OrderKind::Modes => &WORKFLOW_MODES,
            OrderKind::Metadata => &["name", "description", "work_tags"],
            OrderKind::Plain => &[],
        };
        let mut map = serializer.serialize_map(Some(object.len()))?;
        for key in priority.iter().copied().chain(
            object
                .keys()
                .filter(|key| !priority.contains(&key.as_str()))
                .map(String::as_str),
        ) {
            let Some(value) = object.get(key) else {
                continue;
            };
            match (self.kind, key) {
                (OrderKind::Selection, "entries") => {
                    struct Entries<'a>(&'a Value);
                    impl Serialize for Entries<'_> {
                        fn serialize<S: Serializer>(
                            &self,
                            serializer: S,
                        ) -> Result<S::Ok, S::Error> {
                            let Some(values) = self.0.as_array() else {
                                return self.0.serialize(serializer);
                            };
                            let mut sequence = serializer.serialize_seq(Some(values.len()))?;
                            for value in values {
                                sequence.serialize_element(&Ordered {
                                    value,
                                    kind: OrderKind::Entry,
                                })?;
                            }
                            sequence.end()
                        }
                    }
                    map.serialize_entry(key, &Entries(value))?;
                }
                (OrderKind::Entry, "mode_metadata") => map.serialize_entry(
                    key,
                    &Ordered {
                        value,
                        kind: OrderKind::Modes,
                    },
                )?,
                (OrderKind::Modes, _) => map.serialize_entry(
                    key,
                    &Ordered {
                        value,
                        kind: OrderKind::Metadata,
                    },
                )?,
                _ => map.serialize_entry(
                    key,
                    &Ordered {
                        value,
                        kind: OrderKind::Plain,
                    },
                )?,
            }
        }
        map.end()
    }
}

pub fn ordered_selection_bytes(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec_pretty(&Ordered {
        value,
        kind: OrderKind::Selection,
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_path_fixture_matches_exact_order() {
        let selected = vec!["web/backend/java".into(), "web/frontend/react".into()];
        let value = build_hierarchy("task", &selected).unwrap();
        assert_eq!(
            value.resolved_paths,
            [
                "general",
                "web",
                "web/backend",
                "web/backend/java",
                "web/frontend",
                "web/frontend/react"
            ]
        );
        assert_eq!(
            value.required_paths,
            ["general", "web/backend/java", "web/frontend/react"]
        );
        assert_eq!(value.optional_paths, ["web", "web/backend", "web/frontend"]);
        assert_eq!(
            serde_json::to_value(&value).unwrap(),
            json!({
                "schema": "work-hierarchy/v1",
                "work_directory": "task",
                "selected_paths": ["web/backend/java", "web/frontend/react"],
                "resolved_paths": ["general", "web", "web/backend", "web/backend/java", "web/frontend", "web/frontend/react"],
                "required_paths": ["general", "web/backend/java", "web/frontend/react"],
                "optional_paths": ["web", "web/backend", "web/frontend"]
            })
        );
        let raw = serde_json::to_string(&value).unwrap();
        let positions: Vec<_> = [
            "schema",
            "work_directory",
            "selected_paths",
            "resolved_paths",
            "required_paths",
            "optional_paths",
        ]
        .into_iter()
        .map(|field| raw.find(&format!("\"{field}\":")).unwrap())
        .collect();
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn python_path_rejections_and_authorization_match() {
        for path in ["", "general", "Web/backend", "web//backend"] {
            assert_eq!(
                build_hierarchy("execute", &[path.into()])
                    .unwrap_err()
                    .reason_code,
                "invalid_hierarchy_path"
            );
        }
        assert_eq!(
            build_hierarchy("invalid", &[]).unwrap_err().reason_code,
            "invalid_work_directory"
        );
        assert_eq!(
            build_hierarchy("plan", &["web".into(), "web".into()])
                .unwrap_err()
                .reason_code,
            "duplicate_hierarchy_path"
        );
        assert_eq!(
            build_hierarchy("plan", &["web".into(), "web/backend".into()])
                .unwrap_err()
                .reason_code,
            "redundant_hierarchy_path"
        );
        assert!(
            authorize_task_paths(
                &["web/backend/java/jpa".into()],
                &["web/backend/java".into()]
            )
            .is_ok()
        );
        assert_eq!(
            authorize_task_paths(
                &["web/backend/javascript".into()],
                &["web/backend/java".into()]
            )
            .unwrap_err()
            .reason_code,
            "task_hierarchy_path_not_authorized"
        );
    }

    #[test]
    fn independent_application_framework_and_persistence_paths_share_java_once() {
        let paths = [
            "web/backend".into(),
            "java/spring-boot".into(),
            "java/persistence/jpa".into(),
        ];
        let selected = build_hierarchy("task", &paths).unwrap();
        assert_eq!(
            selected.resolved_paths,
            [
                "general",
                "web",
                "web/backend",
                "java",
                "java/spring-boot",
                "java/persistence",
                "java/persistence/jpa"
            ]
        );
        assert_eq!(
            selected
                .resolved_paths
                .iter()
                .filter(|path| *path == "java")
                .count(),
            1
        );
        assert_eq!(
            selected.required_paths,
            [
                "general",
                "web/backend",
                "java/spring-boot",
                "java/persistence/jpa"
            ]
        );
        assert_eq!(
            build_hierarchy("task", &["java".into(), "java/spring-boot".into()])
                .unwrap_err()
                .reason_code,
            "redundant_hierarchy_path"
        );
    }

    #[test]
    fn selection_order_matches_python_contract() {
        let value = json!({
            "selection_sha256": "a",
            "catalog_sha256": "b",
            "entries": [{
                "recommendation_reason": "Reason.",
                "mode_metadata": {"execute": {"work_tags": ["execute"], "description": "Text.", "name": "Execute"}, "plan": {"work_tags": ["plan"], "name": "Plan"}},
                "mode_support": ["plan", "execute"],
                "path": "web",
            }],
            "selected_paths": ["web"],
            "decision": "instruction_paths",
            "schema": "work-hierarchy-selection/v1",
        });
        let output = String::from_utf8(ordered_selection_bytes(&value).unwrap()).unwrap();
        assert!(output.find("\"schema\"").unwrap() < output.find("\"decision\"").unwrap());
        assert!(output.find("\"decision\"").unwrap() < output.find("\"selected_paths\"").unwrap());
        assert!(output.find("\"plan\"").unwrap() < output.find("\"execute\"").unwrap());
        assert!(output.find("\"name\"").unwrap() < output.find("\"work_tags\"").unwrap());
    }

    #[test]
    fn selection_orders_unknown_fields_and_preserves_nonobject_values() {
        let value = json!({
            "zzz": 2, "aaa": 1,
            "schema": "work-hierarchy-selection/v1",
            "decision": "instruction_paths",
            "selected_paths": ["web/backend"],
            "catalog_sha256": "b", "selection_sha256": "a",
            "entries": [{
                "extra": true,
                "path": "web/backend",
                "mode_support": ["plan", "execute"],
                "recommendation_reason": "Selected.",
                "mode_metadata": {
                    "zzz": {"name": "Unknown"},
                    "execute": {"work_tags": ["execute"], "description": "Execute", "name": "Execute"},
                    "plan": {"extra": true, "work_tags": ["plan"], "name": "Plan"}
                }
            }]
        });
        let output = String::from_utf8(ordered_selection_bytes(&value).unwrap()).unwrap();
        let positions = |indent: &str, fields: &[&str]| -> Vec<usize> {
            fields
                .iter()
                .map(|field| output.find(&format!("\n{indent}\"{field}\"")).unwrap())
                .collect()
        };
        for (indent, fields) in [
            (
                "  ",
                vec![
                    "schema",
                    "decision",
                    "selected_paths",
                    "entries",
                    "catalog_sha256",
                    "selection_sha256",
                    "aaa",
                    "zzz",
                ],
            ),
            (
                "      ",
                vec![
                    "path",
                    "mode_support",
                    "mode_metadata",
                    "recommendation_reason",
                    "extra",
                ],
            ),
            ("        ", vec!["plan", "execute", "zzz"]),
            ("          ", vec!["name", "work_tags", "extra"]),
        ] {
            let positions = positions(indent, &fields);
            assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        }
        assert_eq!(
            ordered_selection_bytes(&json!(["general"])).unwrap(),
            b"[\n  \"general\"\n]\n"
        );
    }
}
