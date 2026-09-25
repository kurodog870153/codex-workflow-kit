//! Compatibility decisions for instruction source refresh and router migration.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// Canonical order for an embedded routing manifest in formal artifacts.
pub fn manifest_field_order(path: &[String]) -> Option<&'static [&'static str]> {
    if !path
        .iter()
        .any(|part| part == "routing_manifest" || part == "instruction_selection_manifest")
    {
        return None;
    }
    match path.last().map(String::as_str) {
        Some("routing_manifest" | "instruction_selection_manifest") => Some(&[
            "schema",
            "router_compatibility_revision",
            "routing_input",
            "routing_status",
            "sources",
            "confirmation_required",
            "selection_sha256",
        ]),
        Some("routing_input") => Some(&[
            "mode",
            "status",
            "operation",
            "artifact_lifecycle",
            "formal_events",
            "role",
            "authorization_state",
            "verified_state_sha256",
        ]),
        Some("sources") => Some(&[
            "logical_name",
            "path",
            "compatibility_revision",
            "canonical_sha256",
        ]),
        _ => Some(&[]),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compatibility {
    Valid,
    Refreshable,
    ReviewRequired,
}

impl Compatibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "VALID",
            Self::Refreshable => "REFRESHABLE",
            Self::ReviewRequired => "REVIEW_REQUIRED",
        }
    }
}

pub fn source_compatibility(
    stored: &Value,
    current_sources: &[Value],
) -> (Compatibility, Vec<String>) {
    let old = stored["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            Some((
                (
                    row["kind"].as_str()?.to_owned(),
                    row["logical_name"].as_str()?.to_owned(),
                ),
                row,
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let new = current_sources
        .iter()
        .filter_map(|row| {
            Some((
                (
                    row["kind"].as_str()?.to_owned(),
                    row["logical_name"].as_str()?.to_owned(),
                ),
                row,
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let old_keys = old.keys().cloned().collect::<BTreeSet<_>>();
    let new_keys = new.keys().cloned().collect::<BTreeSet<_>>();
    if old_keys != new_keys {
        return (
            Compatibility::ReviewRequired,
            old_keys
                .symmetric_difference(&new_keys)
                .map(|(_, name)| name.clone())
                .collect(),
        );
    }
    let mut changed = Vec::new();
    for key in old_keys {
        let before = old[&key];
        let after = new[&key];
        let revision = before.get("compatibility_revision");
        if before["canonical_sha256"] == after["canonical_sha256"] {
            if revision.is_none_or(Value::is_null) {
                changed.push(key.1);
            }
            continue;
        }
        changed.push(key.1);
        if revision.is_none_or(Value::is_null) || revision != after.get("compatibility_revision") {
            return (Compatibility::ReviewRequired, changed);
        }
    }
    (
        if changed.is_empty() {
            Compatibility::Valid
        } else {
            Compatibility::Refreshable
        },
        changed,
    )
}

pub fn routing_compatibility(
    stored: Option<&Value>,
    current: &Value,
) -> (Compatibility, Vec<String>) {
    let Some(stored) = stored.filter(|value| !value.is_null()) else {
        return (Compatibility::Refreshable, vec!["routing_manifest".into()]);
    };
    if stored["router_compatibility_revision"] != current["router_compatibility_revision"] {
        return (
            Compatibility::ReviewRequired,
            vec!["routing_manifest".into()],
        );
    }
    let old = stored["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| Some((row["logical_name"].as_str()?.to_owned(), row)))
        .collect::<BTreeMap<_, _>>();
    let new = current["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| Some((row["logical_name"].as_str()?.to_owned(), row)))
        .collect::<BTreeMap<_, _>>();
    let old_keys = old.keys().cloned().collect::<BTreeSet<_>>();
    let new_keys = new.keys().cloned().collect::<BTreeSet<_>>();
    if old_keys != new_keys {
        return (
            Compatibility::ReviewRequired,
            old_keys.symmetric_difference(&new_keys).cloned().collect(),
        );
    }
    let mut changed = Vec::new();
    for name in old_keys {
        if old[&name]["compatibility_revision"] != new[&name]["compatibility_revision"] {
            return (Compatibility::ReviewRequired, vec![name]);
        }
        if old[&name]["canonical_sha256"] != new[&name]["canonical_sha256"] {
            changed.push(name);
        }
    }
    if stored["selection_sha256"] != current["selection_sha256"] && changed.is_empty() {
        return (
            Compatibility::ReviewRequired,
            vec!["routing_manifest".into()],
        );
    }
    (
        if changed.is_empty() {
            Compatibility::Valid
        } else {
            Compatibility::Refreshable
        },
        changed,
    )
}

pub fn combined_compatibility(
    stored: &Value,
    current_sources: &[Value],
    current_manifest: &Value,
) -> (Compatibility, Vec<String>) {
    let (source, source_changed) = source_compatibility(stored, current_sources);
    let (routing, routing_changed) =
        routing_compatibility(stored.get("routing_manifest"), current_manifest);
    let changed = source_changed
        .into_iter()
        .chain(routing_changed)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let state =
        if source == Compatibility::ReviewRequired || routing == Compatibility::ReviewRequired {
            Compatibility::ReviewRequired
        } else if source == Compatibility::Refreshable || routing == Compatibility::Refreshable {
            Compatibility::Refreshable
        } else {
            Compatibility::Valid
        };
    (state, changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source(hash: &str, revision: Option<u64>) -> Value {
        let mut source = json!({"kind":"workflow","logical_name":"work.workflow.plan",
            "canonical_sha256":hash});
        if let Some(revision) = revision {
            source["compatibility_revision"] = json!(revision);
        }
        source
    }

    #[test]
    fn source_and_routing_compatibility_follow_python_cases() {
        let old = json!({"sources":[source("a", Some(1))]});
        assert_eq!(
            source_compatibility(&old, &[source("b", Some(1))]),
            (
                Compatibility::Refreshable,
                vec!["work.workflow.plan".into()]
            )
        );
        assert_eq!(
            source_compatibility(&old, &[source("b", Some(2))]).0,
            Compatibility::ReviewRequired
        );
        let legacy = json!({"sources":[source("a", None)]});
        assert_eq!(
            source_compatibility(&legacy, &[source("a", Some(1))]).0,
            Compatibility::Refreshable
        );
        assert_eq!(
            source_compatibility(&legacy, &[source("b", Some(1))]).0,
            Compatibility::ReviewRequired
        );
        let stored = json!({"router_compatibility_revision":3,"selection_sha256":"a",
            "sources":[{"logical_name":"work.workflow.plan","compatibility_revision":2,
                "canonical_sha256":"a"}]});
        let mut current = stored.clone();
        current["selection_sha256"] = json!("b");
        current["sources"][0]["canonical_sha256"] = json!("b");
        assert_eq!(
            routing_compatibility(Some(&stored), &current),
            (
                Compatibility::Refreshable,
                vec!["work.workflow.plan".into()]
            )
        );
        current["router_compatibility_revision"] = json!(2);
        assert_eq!(
            routing_compatibility(Some(&stored), &current).0,
            Compatibility::ReviewRequired
        );
    }
}
