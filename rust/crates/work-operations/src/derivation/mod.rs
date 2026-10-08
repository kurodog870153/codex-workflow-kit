//! Shared rules for values derived from Work artifacts and evidence.

pub mod fingerprint;
pub mod graph;
pub mod identity;
pub mod legacy_layout;
pub mod publication;
pub mod snapshot;
pub mod transaction;

pub use graph::{ArtifactNode, DerivationGraph, DerivationIssue, DerivationPolicy};
