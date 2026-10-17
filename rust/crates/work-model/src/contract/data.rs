//! Embedded public contract descriptions and scaffold ordering.
use super::{ContractCatalog, ContractCatalogEntry, ContractRegistrySnapshot};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

use super::ContractRecord;

include!(concat!(env!("OUT_DIR"), "/contract_segments.rs"));

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogData {
    schema: String,
    catalog: ContractCatalog,
}

fn embedded_catalog() -> &'static CatalogData {
    static DATA: OnceLock<CatalogData> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../../resources/contract/catalog.json"))
            .expect("embedded contract catalog matches its model")
    })
}

fn base_registry() -> ContractRegistrySnapshot {
    let data = embedded_catalog();
    ContractRegistrySnapshot {
        schema: data.schema.clone(),
        catalog: data.catalog.clone(),
        items: BTreeMap::new(),
    }
}

fn merge_segment(registry: &mut ContractRegistrySnapshot, id: &str, segment: &str) {
    let record: ContractRecord =
        serde_json::from_str(segment).expect("embedded contract segment matches its model");
    assert_eq!(id, record.description.id, "contract segment ID mismatch");
    if !registry
        .catalog
        .contracts
        .iter()
        .any(|entry| entry.id == id)
    {
        registry.catalog.contracts.push(ContractCatalogEntry {
            id: id.to_owned(),
            kind: record.description.kind,
            caller_constructible: record.description.caller_constructible,
        });
    }
    assert!(
        registry.items.insert(id.to_owned(), record).is_none(),
        "duplicate contract ID"
    );
}

pub fn registry() -> ContractRegistrySnapshot {
    let mut registry = base_registry();
    for (id, segment) in SEGMENTS {
        merge_segment(&mut registry, id, segment);
    }
    registry.catalog.contracts.sort_by(|a, b| a.id.cmp(&b.id));
    registry
}

pub fn registry_value() -> Value {
    serde_json::to_value(registry()).expect("serializable contract registry")
}

pub fn scaffold_orders() -> Value {
    static ORDERS: OnceLock<Value> = OnceLock::new();
    ORDERS
        .get_or_init(|| {
            serde_json::from_str(include_str!(
                "../../resources/contract/scaffold-orders.json"
            ))
            .expect("embedded scaffold orders match their model")
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_merges_one_record_and_rejects_duplicates() {
        let mut registry = base_registry();
        let mut record = super::registry().items["work-cli-result"].clone();
        record.description.id = "work-cli-result-copy".into();
        let segment = serde_json::to_string(&record).unwrap();
        merge_segment(&mut registry, "work-cli-result-copy", &segment);
        assert_eq!(registry.items["work-cli-result-copy"], record);
        assert!(
            std::panic::catch_unwind(move || merge_segment(
                &mut registry,
                "work-cli-result-copy",
                &segment
            ))
            .is_err()
        );
    }

    #[test]
    fn segmented_registry_preserves_the_current_catalog() {
        let registry = registry();
        assert_eq!(registry.items.len(), 100);
        assert_eq!(registry.catalog.contracts.len(), 100);
        for (id, record) in &registry.items {
            assert_eq!(id, &record.description.id);
            assert!(
                registry
                    .catalog
                    .contracts
                    .iter()
                    .any(|entry| &entry.id == id)
            );
        }
    }
}
