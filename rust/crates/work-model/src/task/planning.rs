//! Shared requirement sources and instruction choices.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningSource {
    pub snapshot: crate::source::snapshot::SourceSnapshot,
    pub artifacts: super::source::TaskArtifactPaths,
    pub hierarchy_selection: crate::hierarchy::HierarchySelection,
    pub skill_selection: crate::skill::SkillSelection,
    pub acceptance_criteria: Vec<super::source::TaskAcceptance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanningInstructionSelection {
    pub selected_paths: Vec<String>,
    pub references: Vec<String>,
}
