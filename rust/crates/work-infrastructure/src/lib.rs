//! Filesystem, Git, process, and platform adapters.

pub mod artifact_paths;
pub mod clock_workspace;
pub mod codec;
pub mod delegation_storage;
pub mod discussion;
pub mod execution;
pub mod files;
pub mod fixture_support;
pub mod git;
pub mod handoff_storage;
pub mod hierarchy_catalog;
pub mod instruction;
pub mod process;
pub mod recovery;
pub mod routing_sources;
pub mod skill_bundle;
pub mod skill_catalog;
pub mod source_snapshot_storage;
pub mod specification;
pub mod task;
pub mod transaction_storage;
pub mod workflow_storage;
pub mod writer_lock;
