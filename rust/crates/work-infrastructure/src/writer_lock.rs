//! Cross-process writer mutexes.

use std::fs::{File, OpenOptions};
use std::path::Path;

use fs4::FileExt;
use serde_json::json;
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::WriterGuard;
pub use work_feature::ports::WriterLock;

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalWriterLock;

pub struct FileGuard(File);

impl WriterGuard for FileGuard {}

impl Drop for FileGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn busy() -> WorkError {
    WorkError::new(
        ExitCode::LockConflict,
        "work_state_writer_busy",
        "Another Work command is updating this requirement.",
        json!({}),
    )
}

impl WriterLock for LocalWriterLock {
    type Guard = FileGuard;

    fn acquire(&self, path: &Path) -> Result<Self::Guard, WorkError> {
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path)
            .map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "writer_lock_open_failed",
                    "The Work writer lock could not be opened.",
                    json!({"path": path.to_string_lossy()}),
                )
            })?;
        FileExt::try_lock(&file).map_err(|_| busy())?;
        Ok(FileGuard(file))
    }

    fn require_idle(&self, path: &Path) -> Result<(), WorkError> {
        if !path.exists() {
            return Ok(());
        }
        let file = File::open(path).map_err(|_| busy())?;
        FileExt::try_lock(&file).map_err(|_| busy())?;
        FileExt::unlock(&file).map_err(|_| busy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn idle_probe_does_not_create_file_and_guard_releases() {
        let root = std::env::temp_dir().join(format!(
            "work-rust-lock-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join(".work-state-writer.lock");
        let lock = LocalWriterLock;
        lock.require_idle(&path).unwrap();
        assert!(!path.exists());
        let guard = lock.acquire(&path).unwrap();
        assert_eq!(
            lock.require_idle(&path).unwrap_err().reason_code,
            "work_state_writer_busy"
        );
        drop(guard);
        lock.require_idle(&path).unwrap();
    }
}
