//! Pure TASK dependency rules.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskIssue {
    pub reason_code: &'static str,
    pub message: &'static str,
    pub details: Value,
}

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> TaskIssue {
    TaskIssue {
        reason_code,
        message,
        details,
    }
}

pub struct DependencyOrder {
    pub order: Vec<String>,
    pub ancestors: BTreeMap<String, BTreeSet<String>>,
}

pub fn resolve_dependencies(
    task_ids: &[String],
    dependencies: &BTreeMap<String, Vec<String>>,
) -> Result<DependencyOrder, TaskIssue> {
    fn visit(
        task_id: &str,
        dependencies: &BTreeMap<String, Vec<String>>,
        visiting: &mut HashSet<String>,
        ancestors: &mut BTreeMap<String, BTreeSet<String>>,
    ) -> Result<BTreeSet<String>, TaskIssue> {
        if visiting.contains(task_id) {
            return Err(issue(
                "cyclic_task_dependency",
                "TASK dependencies must not contain a cycle.",
                json!({"task_id": task_id}),
            ));
        }
        if let Some(known) = ancestors.get(task_id) {
            return Ok(known.clone());
        }
        let Some(direct) = dependencies.get(task_id) else {
            return Err(issue(
                "invalid_task_dependency",
                "A TASK dependency is invalid.",
                json!({"task_id": task_id}),
            ));
        };
        visiting.insert(task_id.into());
        let mut result = BTreeSet::new();
        for dependency in direct {
            result.insert(dependency.clone());
            result.extend(visit(dependency, dependencies, visiting, ancestors)?);
        }
        visiting.remove(task_id);
        ancestors.insert(task_id.into(), result.clone());
        Ok(result)
    }
    let mut ancestors = BTreeMap::new();
    let mut visiting = HashSet::new();
    for task_id in task_ids {
        visit(task_id, dependencies, &mut visiting, &mut ancestors)?;
    }
    for (task_id, direct) in dependencies {
        for dependency in direct {
            if direct.iter().any(|other| {
                other != dependency
                    && ancestors
                        .get(other)
                        .is_some_and(|values| values.contains(dependency))
            }) {
                return Err(issue(
                    "indirect_task_dependency",
                    "Only direct TASK dependencies may be listed.",
                    json!({"task_id": task_id, "dependency": dependency}),
                ));
            }
        }
    }
    let mut order = Vec::new();
    let mut pending: HashSet<_> = task_ids.iter().collect();
    while !pending.is_empty() {
        let ready: Vec<_> = task_ids
            .iter()
            .filter(|task_id| {
                pending.contains(task_id)
                    && dependencies
                        .get(*task_id)
                        .is_some_and(|deps| deps.iter().all(|dep| order.contains(dep)))
            })
            .cloned()
            .collect();
        if ready.is_empty() {
            return Err(issue(
                "cyclic_task_dependency",
                "TASK dependency cycle.",
                json!({}),
            ));
        }
        for task_id in &ready {
            pending.remove(task_id);
        }
        order.extend(ready);
    }
    Ok(DependencyOrder { order, ancestors })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_dependency_cases_match() {
        let ids: Vec<_> = (1..=4).map(|n| format!("TASK-{n:03}")).collect();
        let dependencies = BTreeMap::from([
            (ids[0].clone(), vec![]),
            (ids[1].clone(), vec![ids[0].clone()]),
            (ids[2].clone(), vec![ids[0].clone()]),
            (ids[3].clone(), vec![ids[1].clone(), ids[2].clone()]),
        ]);
        let resolved = resolve_dependencies(&ids, &dependencies).unwrap();
        assert_eq!(resolved.order, ids);
        assert!(resolved.ancestors[&ids[0]].is_empty());
        assert_eq!(
            resolved.ancestors[&ids[3]],
            BTreeSet::from([ids[0].clone(), ids[1].clone(), ids[2].clone()])
        );
        let ready_order = vec![ids[1].clone(), ids[0].clone()];
        let independent = BTreeMap::from([(ids[1].clone(), vec![]), (ids[0].clone(), vec![])]);
        assert_eq!(
            resolve_dependencies(&ready_order, &independent)
                .unwrap()
                .order,
            ready_order
        );
        let indirect = BTreeMap::from([
            (ids[0].clone(), vec![]),
            (ids[1].clone(), vec![ids[0].clone()]),
            (ids[2].clone(), vec![ids[0].clone(), ids[1].clone()]),
        ]);
        let issue = resolve_dependencies(&ids[..3], &indirect).err().unwrap();
        assert_eq!(issue.reason_code, "indirect_task_dependency");
        assert_eq!(issue.details["task_id"], ids[2]);
        assert_eq!(issue.details["dependency"], ids[0]);
        let cycle = BTreeMap::from([
            (ids[0].clone(), vec![ids[1].clone()]),
            (ids[1].clone(), vec![ids[0].clone()]),
        ]);
        assert_eq!(
            resolve_dependencies(&ids[..2], &cycle)
                .err()
                .unwrap()
                .reason_code,
            "cyclic_task_dependency"
        );
    }
}

pub mod candidate;
pub mod changes;
pub mod collection;
pub mod create;
pub mod draft;
pub mod draft_list;
pub mod draft_source;
pub mod index;
pub mod item;
pub mod ordering;
pub mod repair;
pub mod semantic;
