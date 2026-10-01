//! Typed dependency topology and evidence policies for Work derivation.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use work_model::task::index::TaskItemReference;

use crate::canonical::parse_json_contract;
use crate::derivation::fingerprint;
use crate::task::ordering::{TaskDocumentKind, render_task};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArtifactNode {
    InstructionSource(String),
    PlanInstructionSelection,
    TaskIndexInstructionSelection,
    TaskInstructionSelection(String),
    PlanBytes,
    PlanFingerprint,
    TaskItemBytes(String),
    TaskItemFingerprint(String),
    TaskIndexBytes,
    TaskIndexFingerprint,
    TaskCollectionFingerprint,
    ExecutionIndexBytes,
    TransactionSnapshot(String),
    TransactionApproval,
    TransactionIdentity,
    JournalBytes,
    CompletionMarker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivationPolicy {
    Recompute,
    Snapshot,
    ValidateOnly,
    GenerateOnce,
    Allocate,
    ApprovalBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DerivationIssue {
    UnknownNode(ArtifactNode),
    Cycle,
    MissingEvidence(ArtifactNode),
    EvidenceMismatch(ArtifactNode),
    Compute(ArtifactNode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactBindingIssue {
    InvalidUtf8,
    InvalidTaskIndex,
    MissingTaskItem(String),
    InvalidTaskItem(String),
    Render,
}

impl ArtifactBindingIssue {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidUtf8 => "invalid_utf8",
            Self::InvalidTaskIndex | Self::MissingTaskItem(_) => "invalid_task_index",
            Self::InvalidTaskItem(_) => "invalid_task_item",
            Self::Render => "invalid_contract_value",
        }
    }
}

/// Reconcile the current Plan → TASK → Execute bindings for any changed artifact roots.
pub fn reconcile_artifact_bindings(
    plan_raw: &[u8],
    index: &mut Value,
    items: &BTreeMap<String, Vec<u8>>,
    execution: Option<&mut Value>,
    changed_roots: &BTreeSet<ArtifactNode>,
) -> Result<Vec<u8>, ArtifactBindingIssue> {
    let task_ids = items.keys().cloned().collect::<Vec<_>>();
    let sources = changed_roots
        .iter()
        .filter_map(|node| match node {
            ArtifactNode::InstructionSource(name) => Some(name.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let graph = DerivationGraph::artifact(&task_ids, &sources);
    let affected = graph
        .descendants(changed_roots)
        .map_err(|_| ArtifactBindingIssue::InvalidTaskIndex)?;
    let index_raw = if affected.contains(&ArtifactNode::TaskIndexBytes)
        || changed_roots.contains(&ArtifactNode::TaskIndexBytes)
    {
        rebind_task_index(plan_raw, index, items)?
    } else {
        render_task(index, TaskDocumentKind::Index).map_err(|_| ArtifactBindingIssue::Render)?
    };
    if let Some(execution) = execution {
        if affected.contains(&ArtifactNode::ExecutionIndexBytes)
            || changed_roots.contains(&ArtifactNode::ExecutionIndexBytes)
        {
            rebind_execution_index(&index_raw, index, items, execution)?;
        }
    }
    Ok(index_raw)
}

/// Rebuild the current TASK index bindings from the actual Plan and item bytes.
pub fn rebind_task_index(
    plan_raw: &[u8],
    index: &mut Value,
    items: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<u8>, ArtifactBindingIssue> {
    bind_plan_source(plan_raw, index)?;
    let references = index["tasks"]
        .as_array_mut()
        .ok_or(ArtifactBindingIssue::InvalidTaskIndex)?;
    for reference in references {
        let id = reference["id"]
            .as_str()
            .ok_or(ArtifactBindingIssue::InvalidTaskIndex)?;
        let raw = items
            .get(id)
            .ok_or_else(|| ArtifactBindingIssue::MissingTaskItem(id.into()))?;
        reference["canonical_sha256"] =
            json!(fingerprint::canonical(raw).map_err(|_| ArtifactBindingIssue::InvalidUtf8)?);
    }
    render_task(index, TaskDocumentKind::Index).map_err(|_| ArtifactBindingIssue::Render)
}

/// Bind the current Plan bytes before a caller records the source change.
pub fn bind_plan_source(plan_raw: &[u8], index: &mut Value) -> Result<(), ArtifactBindingIssue> {
    let plan = parse_json_contract(plan_raw).map_err(|_| ArtifactBindingIssue::InvalidUtf8)?;
    index["source_plan"]["canonical_sha256"] =
        json!(fingerprint::canonical(plan_raw).map_err(|_| ArtifactBindingIssue::InvalidUtf8)?);
    index["source_plan"]["hierarchy_selection_sha256"] =
        plan["hierarchy_selection"]["selection_sha256"].clone();
    Ok(())
}

/// Rebuild current Execute bindings from the newly rendered TASK index and items.
pub fn rebind_execution_index(
    index_raw: &[u8],
    index: &Value,
    items: &BTreeMap<String, Vec<u8>>,
    execution: &mut Value,
) -> Result<(), ArtifactBindingIssue> {
    let index_sha =
        fingerprint::canonical(index_raw).map_err(|_| ArtifactBindingIssue::InvalidUtf8)?;
    let references = index["tasks"]
        .as_array()
        .ok_or(ArtifactBindingIssue::InvalidTaskIndex)?
        .iter()
        .map(|reference| {
            serde_json::from_value::<TaskItemReference>(reference.clone())
                .map_err(|_| ArtifactBindingIssue::InvalidTaskIndex)
        })
        .collect::<Result<Vec<_>, _>>()?;
    execution["task_index_sha256"] = json!(index_sha);
    execution["task_collection_sha256"] =
        json!(fingerprint::task_collection(&index_sha, &references));
    execution["task_instructions_sha256"] =
        index["instruction_selection"]["instructions_sha256"].clone();
    let rows = execution["tasks"]
        .as_array_mut()
        .ok_or(ArtifactBindingIssue::InvalidTaskIndex)?;
    for reference in references {
        let raw = items
            .get(&reference.id)
            .ok_or_else(|| ArtifactBindingIssue::MissingTaskItem(reference.id.clone()))?;
        let item = parse_json_contract(raw)
            .map_err(|_| ArtifactBindingIssue::InvalidTaskItem(reference.id.clone()))?;
        let row = rows
            .iter_mut()
            .find(|row| row["id"] == reference.id)
            .ok_or(ArtifactBindingIssue::InvalidTaskIndex)?;
        row["task_item_sha256"] = json!(reference.canonical_sha256);
        row["instructions_sha256"] = item["instruction_selection"]["instructions_sha256"].clone();
    }
    Ok(())
}

/// Apply validated collection bindings without letting a caller assign derived fields.
pub fn rebind_validated_execution(
    execution: &mut Value,
    validation: &Value,
) -> Result<(), ArtifactBindingIssue> {
    let ids = validation["task_ids"]
        .as_array()
        .ok_or(ArtifactBindingIssue::InvalidTaskIndex)?;
    let rows = execution["tasks"]
        .as_array_mut()
        .ok_or(ArtifactBindingIssue::InvalidTaskIndex)?;
    if rows.len() != ids.len()
        || rows
            .iter()
            .any(|row| !ids.iter().any(|id| *id == row["id"]))
    {
        return Err(ArtifactBindingIssue::InvalidTaskIndex);
    }
    execution["task_spec_id"] = validation["spec_id"].clone();
    execution["task_collection_sha256"] = validation["task_collection_sha256"].clone();
    execution["task_index_sha256"] = validation["task_index_sha256"].clone();
    execution["task_instructions_sha256"] = validation["instructions_sha256"].clone();
    execution["hierarchy_selection_sha256"] = validation["hierarchy_selection_sha256"].clone();
    execution["skill_selection_sha256"] = validation["skill_selection_sha256"].clone();
    for row in execution["tasks"]
        .as_array_mut()
        .expect("TASK rows were checked")
    {
        let id = row["id"]
            .as_str()
            .ok_or(ArtifactBindingIssue::InvalidTaskIndex)?
            .to_owned();
        if row["skill_id"] != validation["task_skill_ids"][&id] {
            return Err(ArtifactBindingIssue::InvalidTaskIndex);
        }
        row["task_item_sha256"] = validation["task_item_sha256"][&id].clone();
        row["instructions_sha256"] = validation["task_instructions_sha256"][&id].clone();
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct DerivationGraph {
    dependencies: BTreeMap<ArtifactNode, BTreeSet<ArtifactNode>>,
    policies: BTreeMap<ArtifactNode, DerivationPolicy>,
}

impl DerivationGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, node: ArtifactNode, policy: DerivationPolicy) {
        self.dependencies.entry(node.clone()).or_default();
        self.policies.insert(node, policy);
    }

    pub fn add_edge(
        &mut self,
        source: ArtifactNode,
        dependent: ArtifactNode,
    ) -> Result<(), DerivationIssue> {
        if !self.dependencies.contains_key(&source) {
            return Err(DerivationIssue::UnknownNode(source));
        }
        let dependencies = self
            .dependencies
            .get_mut(&dependent)
            .ok_or_else(|| DerivationIssue::UnknownNode(dependent.clone()))?;
        dependencies.insert(source.clone());
        if self.topological_order().is_err() {
            self.dependencies
                .get_mut(&dependent)
                .expect("dependent exists")
                .remove(&source);
            return Err(DerivationIssue::Cycle);
        }
        Ok(())
    }

    pub fn artifact(task_ids: &[String], instruction_sources: &[String]) -> Self {
        use ArtifactNode as N;
        use DerivationPolicy::Recompute;

        let mut graph = Self::new();
        for node in [
            N::PlanInstructionSelection,
            N::PlanBytes,
            N::PlanFingerprint,
            N::TaskIndexInstructionSelection,
            N::TaskIndexBytes,
            N::TaskIndexFingerprint,
            N::TaskCollectionFingerprint,
            N::ExecutionIndexBytes,
        ] {
            graph.add_node(node, Recompute);
        }
        for (source, dependent) in [
            (N::PlanInstructionSelection, N::PlanBytes),
            (N::PlanBytes, N::PlanFingerprint),
            (N::PlanFingerprint, N::TaskIndexBytes),
            (N::TaskIndexInstructionSelection, N::TaskIndexBytes),
            (N::TaskIndexBytes, N::TaskIndexFingerprint),
            (N::TaskIndexFingerprint, N::TaskCollectionFingerprint),
            (N::TaskCollectionFingerprint, N::ExecutionIndexBytes),
        ] {
            graph.add_edge(source, dependent).expect("artifact DAG");
        }
        for task_id in task_ids {
            let bytes = N::TaskItemBytes(task_id.clone());
            let fingerprint = N::TaskItemFingerprint(task_id.clone());
            let selection = N::TaskInstructionSelection(task_id.clone());
            graph.add_node(selection.clone(), Recompute);
            graph.add_node(bytes.clone(), Recompute);
            graph.add_node(fingerprint.clone(), Recompute);
            graph
                .add_edge(selection, bytes.clone())
                .expect("artifact DAG");
            graph
                .add_edge(bytes, fingerprint.clone())
                .expect("artifact DAG");
            graph
                .add_edge(fingerprint, N::TaskIndexBytes)
                .expect("artifact DAG");
        }
        for source_name in instruction_sources {
            let source = N::InstructionSource(source_name.clone());
            graph.add_node(source.clone(), Recompute);
            graph
                .add_edge(source.clone(), N::PlanInstructionSelection)
                .expect("artifact DAG");
            graph
                .add_edge(source.clone(), N::TaskIndexInstructionSelection)
                .expect("artifact DAG");
            for task_id in task_ids {
                graph
                    .add_edge(source.clone(), N::TaskInstructionSelection(task_id.clone()))
                    .expect("artifact DAG");
            }
        }
        graph
    }

    pub fn topological_order(&self) -> Result<Vec<ArtifactNode>, DerivationIssue> {
        let mut remaining = self.dependencies.clone();
        let mut order = Vec::with_capacity(remaining.len());
        while !remaining.is_empty() {
            let ready = remaining
                .iter()
                .filter(|(_, dependencies)| dependencies.is_empty())
                .map(|(node, _)| node.clone())
                .collect::<Vec<_>>();
            if ready.is_empty() {
                return Err(DerivationIssue::Cycle);
            }
            for node in ready {
                remaining.remove(&node);
                for dependencies in remaining.values_mut() {
                    dependencies.remove(&node);
                }
                order.push(node);
            }
        }
        Ok(order)
    }

    pub fn descendants(
        &self,
        changed_roots: &BTreeSet<ArtifactNode>,
    ) -> Result<BTreeSet<ArtifactNode>, DerivationIssue> {
        for root in changed_roots {
            if !self.dependencies.contains_key(root) {
                return Err(DerivationIssue::UnknownNode(root.clone()));
            }
        }
        let mut affected = changed_roots.clone();
        for node in self.topological_order()? {
            if self.dependencies[&node]
                .iter()
                .any(|dependency| affected.contains(dependency))
            {
                affected.insert(node);
            }
        }
        for root in changed_roots {
            affected.remove(root);
        }
        Ok(affected)
    }

    pub fn reconcile<F>(
        &self,
        snapshot: &mut BTreeMap<ArtifactNode, Vec<u8>>,
        changed_roots: &BTreeSet<ArtifactNode>,
        mut derive: F,
    ) -> Result<Vec<ArtifactNode>, DerivationIssue>
    where
        F: FnMut(&ArtifactNode, &BTreeMap<ArtifactNode, Vec<u8>>) -> Result<Vec<u8>, ()>,
    {
        let affected = self.descendants(changed_roots)?;
        let mut candidate = snapshot.clone();
        let mut updates = Vec::new();
        for node in self.topological_order()? {
            if !affected.contains(&node) {
                continue;
            }
            let policy = self.policies[&node];
            let existing = candidate.get(&node);
            if existing.is_some()
                && matches!(
                    policy,
                    DerivationPolicy::Snapshot | DerivationPolicy::Allocate
                )
            {
                continue;
            }
            let derived =
                derive(&node, &candidate).map_err(|()| DerivationIssue::Compute(node.clone()))?;
            match (policy, existing) {
                (DerivationPolicy::ValidateOnly, None) => {
                    return Err(DerivationIssue::MissingEvidence(node));
                }
                (
                    DerivationPolicy::ValidateOnly
                    | DerivationPolicy::GenerateOnce
                    | DerivationPolicy::ApprovalBoundary,
                    Some(value),
                ) if *value != derived => {
                    return Err(DerivationIssue::EvidenceMismatch(node));
                }
                (DerivationPolicy::ApprovalBoundary, None) => {
                    return Err(DerivationIssue::MissingEvidence(node));
                }
                (_, Some(value)) if *value == derived => {}
                _ => {
                    candidate.insert(node.clone(), derived);
                    updates.push(node);
                }
            }
        }
        *snapshot = candidate;
        Ok(updates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(node: ArtifactNode) -> BTreeSet<ArtifactNode> {
        BTreeSet::from([node])
    }

    #[test]
    fn concrete_plan_item_and_instruction_changes_reach_fixed_point() {
        let plan_raw = serde_json::to_vec(&json!({
            "hierarchy_selection":{"selection_sha256":"a".repeat(64)}
        }))
        .unwrap();
        let item_raw = serde_json::to_vec(&json!({
            "instruction_selection":{"instructions_sha256":"b".repeat(64)}
        }))
        .unwrap();
        let items = BTreeMap::from([("TASK-001".into(), item_raw.clone())]);
        let mut index = json!({"source_plan":{},
            "instruction_selection":{"instructions_sha256":"c".repeat(64)},
            "tasks":[{"id":"TASK-001","path":"tasks/TASK-001.json"}]});
        let mut execution = json!({"tasks":[{"id":"TASK-001"}]});
        let changed = BTreeSet::from([
            ArtifactNode::PlanBytes,
            ArtifactNode::TaskItemBytes("TASK-001".into()),
            ArtifactNode::InstructionSource("task.general".into()),
        ]);
        let first = reconcile_artifact_bindings(
            &plan_raw,
            &mut index,
            &items,
            Some(&mut execution),
            &changed,
        )
        .unwrap();
        assert_eq!(
            index["source_plan"]["canonical_sha256"],
            fingerprint::canonical(&plan_raw).unwrap()
        );
        assert_eq!(
            index["tasks"][0]["canonical_sha256"],
            fingerprint::canonical(&item_raw).unwrap()
        );
        assert_eq!(execution["task_index_sha256"], fingerprint::raw(&first));
        assert_eq!(execution["tasks"][0]["instructions_sha256"], "b".repeat(64));
        let previous = execution.clone();
        let second = reconcile_artifact_bindings(
            &plan_raw,
            &mut index,
            &items,
            Some(&mut execution),
            &changed,
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(execution, previous);
    }

    #[test]
    fn artifact_graph_propagates_in_topological_order_and_reaches_fixed_point() {
        let graph = DerivationGraph::artifact(&[], &[]);
        let mut state = BTreeMap::from([(ArtifactNode::PlanBytes, b"plan".to_vec())]);
        let derive = |node: &ArtifactNode, state: &BTreeMap<ArtifactNode, Vec<u8>>| {
            let source = match node {
                ArtifactNode::PlanFingerprint => &ArtifactNode::PlanBytes,
                ArtifactNode::TaskIndexBytes => &ArtifactNode::PlanFingerprint,
                ArtifactNode::TaskIndexFingerprint => &ArtifactNode::TaskIndexBytes,
                ArtifactNode::TaskCollectionFingerprint => &ArtifactNode::TaskIndexFingerprint,
                ArtifactNode::ExecutionIndexBytes => &ArtifactNode::TaskCollectionFingerprint,
                _ => return Err(()),
            };
            Ok(crate::derivation::fingerprint::raw(&state[source]).into_bytes())
        };
        let first = graph
            .reconcile(&mut state, &roots(ArtifactNode::PlanBytes), derive)
            .unwrap();
        assert!(first.contains(&ArtifactNode::TaskIndexBytes));
        assert!(first.contains(&ArtifactNode::ExecutionIndexBytes));
        assert!(
            first
                .iter()
                .position(|node| *node == ArtifactNode::TaskIndexBytes)
                < first
                    .iter()
                    .position(|node| *node == ArtifactNode::ExecutionIndexBytes)
        );
        let second = graph
            .reconcile(&mut state, &roots(ArtifactNode::PlanBytes), derive)
            .unwrap();
        assert!(second.is_empty());
        let previous = state[&ArtifactNode::ExecutionIndexBytes].clone();
        state.insert(ArtifactNode::PlanBytes, b"changed plan".to_vec());
        assert!(
            !graph
                .reconcile(&mut state, &roots(ArtifactNode::PlanBytes), derive)
                .unwrap()
                .is_empty()
        );
        assert_ne!(state[&ArtifactNode::ExecutionIndexBytes], previous);
    }

    #[test]
    fn item_and_instruction_roots_reach_execution_without_touching_history() {
        let graph = DerivationGraph::artifact(&["TASK-001".into()], &["task.general".into()]);
        for source in [
            ArtifactNode::TaskItemBytes("TASK-001".into()),
            ArtifactNode::InstructionSource("task.general".into()),
        ] {
            let descendants = graph.descendants(&roots(source)).unwrap();
            assert!(descendants.contains(&ArtifactNode::TaskIndexBytes));
            assert!(descendants.contains(&ArtifactNode::ExecutionIndexBytes));
            assert!(!descendants.contains(&ArtifactNode::TransactionApproval));
        }
    }

    #[test]
    fn graph_rejects_cycles_without_changing_edges() {
        let mut graph = DerivationGraph::artifact(&[], &[]);
        assert_eq!(
            graph.add_edge(ArtifactNode::ExecutionIndexBytes, ArtifactNode::PlanBytes),
            Err(DerivationIssue::Cycle)
        );
        assert!(graph.topological_order().is_ok());
    }

    #[test]
    fn evidence_policies_preserve_snapshot_and_fail_closed() {
        let source = ArtifactNode::JournalBytes;
        let marker = ArtifactNode::CompletionMarker;
        let mut graph = DerivationGraph::new();
        graph.add_node(source.clone(), DerivationPolicy::Recompute);
        graph.add_node(marker.clone(), DerivationPolicy::GenerateOnce);
        graph.add_edge(source.clone(), marker.clone()).unwrap();
        let mut state = BTreeMap::from([(source.clone(), b"new".to_vec())]);
        assert_eq!(
            graph.reconcile(&mut state, &roots(source.clone()), |_, _| Ok(
                b"marker".to_vec()
            )),
            Ok(vec![marker.clone()])
        );
        state.insert(source.clone(), b"changed".to_vec());
        assert_eq!(
            graph.reconcile(&mut state, &roots(source.clone()), |_, _| Ok(
                b"different".to_vec()
            )),
            Err(DerivationIssue::EvidenceMismatch(marker.clone()))
        );
        assert_eq!(state[&marker], b"marker");
        let mut snapshot_graph = DerivationGraph::new();
        snapshot_graph.add_node(source.clone(), DerivationPolicy::Recompute);
        snapshot_graph.add_node(marker.clone(), DerivationPolicy::Snapshot);
        snapshot_graph
            .add_edge(source.clone(), marker.clone())
            .unwrap();
        assert_eq!(
            snapshot_graph.reconcile(&mut state, &roots(source), |_, _| Ok(b"different".to_vec())),
            Ok(Vec::new())
        );
        assert_eq!(state[&marker], b"marker");
    }

    #[test]
    fn validation_and_approval_require_existing_matching_evidence() {
        let source = ArtifactNode::JournalBytes;
        let evidence = ArtifactNode::TransactionApproval;
        for policy in [
            DerivationPolicy::ValidateOnly,
            DerivationPolicy::ApprovalBoundary,
        ] {
            let mut graph = DerivationGraph::new();
            graph.add_node(source.clone(), DerivationPolicy::Recompute);
            graph.add_node(evidence.clone(), policy);
            graph.add_edge(source.clone(), evidence.clone()).unwrap();
            let mut state = BTreeMap::from([(source.clone(), b"source".to_vec())]);
            assert_eq!(
                graph.reconcile(&mut state, &roots(source.clone()), |_, _| Ok(
                    b"proof".to_vec()
                )),
                Err(DerivationIssue::MissingEvidence(evidence.clone()))
            );
            state.insert(evidence.clone(), b"proof".to_vec());
            assert_eq!(
                graph.reconcile(&mut state, &roots(source.clone()), |_, _| Ok(
                    b"proof".to_vec()
                )),
                Ok(Vec::new())
            );
            assert_eq!(
                graph.reconcile(&mut state, &roots(source.clone()), |_, _| Ok(
                    b"drift".to_vec()
                )),
                Err(DerivationIssue::EvidenceMismatch(evidence.clone()))
            );
        }
    }

    #[test]
    fn allocation_generates_once_and_preserves_existing_identity() {
        let source = ArtifactNode::JournalBytes;
        let identity = ArtifactNode::TransactionIdentity;
        let mut graph = DerivationGraph::new();
        graph.add_node(source.clone(), DerivationPolicy::Recompute);
        graph.add_node(identity.clone(), DerivationPolicy::Allocate);
        graph.add_edge(source.clone(), identity.clone()).unwrap();
        let mut state = BTreeMap::from([(source.clone(), b"source".to_vec())]);
        assert_eq!(
            graph.reconcile(&mut state, &roots(source.clone()), |_, _| Ok(
                b"ID-001".to_vec()
            )),
            Ok(vec![identity.clone()])
        );
        assert_eq!(
            graph.reconcile(&mut state, &roots(source), |_, _| Err(())),
            Ok(Vec::new())
        );
        assert_eq!(state[&identity], b"ID-001");
    }
}
