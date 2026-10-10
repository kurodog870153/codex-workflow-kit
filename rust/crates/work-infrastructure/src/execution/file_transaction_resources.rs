//! Bounded byte budgets before project files are read or published.
use std::fs::File;
use std::io::Read;
use std::path::Path;
use work_feature::error::WorkError;
use work_feature::execution::file_transaction::error;

// DTOs encode bytes as JSON arrays and clone complete recovery evidence.
// Bound the conservative peak instead of counting files or source lines.
pub const MAX_PROJECT_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_CONTROL_BYTES: u64 = 64 * 1024 * 1024;
const RESERVE_BYTES: u64 = 1024 * 1024;

pub fn requirements(raw_bytes: u64) -> Result<(u64, u64), WorkError> {
    if raw_bytes > MAX_PROJECT_BYTES {
        let mut failure = error("file_transaction_resource_limit");
        failure.details = serde_json::json!({"weighted_bytes":raw_bytes,"limit_bytes":MAX_PROJECT_BYTES,"recovery_required":true});
        return Err(failure);
    }
    let memory = raw_bytes
        .checked_mul(128)
        .and_then(|v| v.checked_add(RESERVE_BYTES))
        .ok_or_else(|| error("file_transaction_resource_limit"))?;
    let storage = raw_bytes
        .checked_mul(24)
        .and_then(|v| v.checked_add(RESERVE_BYTES))
        .ok_or_else(|| error("file_transaction_resource_limit"))?;
    Ok((memory, storage))
}

pub fn verify(raw_bytes: u64, available_storage: u64) -> Result<(), WorkError> {
    let (memory, storage) = requirements(raw_bytes)?;
    if storage > available_storage {
        let mut failure = error("file_transaction_storage_insufficient");
        failure.details = serde_json::json!({"required_bytes":storage,"available_bytes":available_storage,"recovery_required":true});
        return Err(failure);
    }
    check_memory(memory, super::file_transaction_memory::available()?)?;
    // Check the current allocator without claiming other processes cannot
    // consume resources later. Durable evidence still protects later failure.
    let mut probe = Vec::<u8>::new();
    probe
        .try_reserve_exact(
            usize::try_from(memory).map_err(|_| error("file_transaction_memory_insufficient"))?,
        )
        .map_err(|_| error("file_transaction_memory_insufficient"))?;
    Ok(())
}

fn check_memory(required: u64, available: u64) -> Result<(), WorkError> {
    if required > available {
        let mut failure = error("file_transaction_memory_insufficient");
        failure.details = serde_json::json!({"required_bytes":required,"available_bytes":available,"recovery_required":true});
        return Err(failure);
    }
    Ok(())
}

pub fn read(path: &Path, limit: u64) -> Result<Vec<u8>, WorkError> {
    read_with_memory(path, limit, super::file_transaction_memory::available)
}

fn read_with_memory(
    path: &Path,
    limit: u64,
    available: impl FnOnce() -> Result<u64, WorkError>,
) -> Result<Vec<u8>, WorkError> {
    let file = File::open(path).map_err(|_| error("file_transaction_file_read"))?;
    let length = file
        .metadata()
        .map_err(|_| error("file_transaction_file_read"))?
        .len();
    if length > limit {
        return Err(error("file_transaction_resource_limit"));
    }
    // Control JSON may expand into a Value tree and coexist with frozen evidence.
    // Check that expansion before reading, not only the raw byte allocation.
    let required = length
        .checked_mul(32)
        .and_then(|value| value.checked_add(RESERVE_BYTES))
        .ok_or_else(|| error("file_transaction_memory_insufficient"))?;
    check_memory(required, available()?)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(
            usize::try_from(length).map_err(|_| error("file_transaction_resource_limit"))?,
        )
        .map_err(|_| error("file_transaction_memory_insufficient"))?;
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("file_transaction_file_read"))?;
    if bytes.len() as u64 > limit || bytes.len() as u64 != length {
        return Err(error("file_transaction_target_drift"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_read_distinguishes_static_limit_from_host_memory_without_reading_or_changing_evidence()
     {
        let path = std::env::temp_dir().join(format!(
            "work-control-resource-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, [1; 42]).unwrap();
        assert_eq!(
            read_with_memory(&path, 41, || panic!("static limit must be first"))
                .unwrap_err()
                .reason_code,
            "file_transaction_resource_limit"
        );
        let failure = read_with_memory(&path, 42, || Ok(0)).unwrap_err();
        assert_eq!(failure.reason_code, "file_transaction_memory_insufficient");
        assert_eq!(failure.details["required_bytes"], 42 * 32 + RESERVE_BYTES);
        assert_eq!(failure.details["available_bytes"], 0);
        assert_eq!(
            read_with_memory(&path, 42, || Ok(u64::MAX)).unwrap(),
            [1; 42]
        );
        assert_eq!(std::fs::read(path).unwrap(), [1; 42]);
    }
    #[test]
    fn byte_and_storage_limits_reject_before_allocation() {
        assert!(requirements(MAX_PROJECT_BYTES).is_ok());
        assert_eq!(
            requirements(MAX_PROJECT_BYTES + 1).unwrap_err().reason_code,
            "file_transaction_resource_limit"
        );
        assert_eq!(
            requirements(u64::MAX).unwrap_err().reason_code,
            "file_transaction_resource_limit"
        );
        let (_, storage) = requirements(42).unwrap();
        assert_eq!(
            verify(42, storage - 1).unwrap_err().reason_code,
            "file_transaction_storage_insufficient"
        );
        assert!(verify(42, storage).is_ok());
        let (memory, _) = requirements(42).unwrap();
        assert_eq!(
            check_memory(memory, memory - 1).unwrap_err().reason_code,
            "file_transaction_memory_insufficient"
        );
        assert!(check_memory(memory, memory).is_ok());
    }
}
