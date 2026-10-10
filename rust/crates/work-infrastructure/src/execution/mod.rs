//! Project-relative execution adapters.

mod control_commit;
mod file_transaction_memory;
mod file_transaction_resources;
pub mod file_transaction_storage;
mod project_file_metadata;
#[cfg(any(target_os = "macos", test))]
mod project_file_metadata_macos;
pub mod storage;
