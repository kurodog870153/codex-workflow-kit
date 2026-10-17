//! Public project-file contract records.
use crate::contract::ContractRecord;
use std::collections::BTreeMap;

pub fn records() -> BTreeMap<String, ContractRecord> {
    let registry = crate::contract::data::registry();
    [
        "work-file-recovery-preview",
        "work-file-transaction-preview",
        "work-file-transaction-request",
        "work-file-transaction-result",
    ]
    .into_iter()
    .map(|id| (id.to_owned(), registry.items[id].clone()))
    .collect()
}
