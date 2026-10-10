//! Atomic creation of a control record, refusing to replace an existing path.
use std::path::Path;
use work_feature::error::WorkError;
use work_feature::execution::file_transaction::error;

#[cfg(windows)]
fn install(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(source: *const u16, target: *const u16, flags: u32) -> i32;
    }
    let source: Vec<_> = source.as_os_str().encode_wide().chain([0]).collect();
    let target: Vec<_> = target.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: null-terminated paths remain live. WRITE_THROUGH (8) deliberately
    // excludes REPLACE_EXISTING and COPY_ALLOWED, so no foreign path is replaced.
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), 8) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn install(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::ffi::{CString, c_char};
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::ErrorKind::InvalidInput)?;
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| std::io::ErrorKind::InvalidInput)?;
    #[cfg(target_os = "macos")]
    unsafe extern "C" {
        fn renamex_np(source: *const c_char, target: *const c_char, flags: u32) -> i32;
    }
    #[cfg(target_os = "linux")]
    unsafe extern "C" {
        fn renameat2(
            oldfd: i32,
            source: *const c_char,
            newfd: i32,
            target: *const c_char,
            flags: u32,
        ) -> i32;
    }
    // SAFETY: live C strings; Darwin RENAME_EXCL (4) and Linux
    // RENAME_NOREPLACE (1), with AT_FDCWD (-100), atomically refuse replacement.
    #[cfg(target_os = "macos")]
    let result = unsafe { renamex_np(source.as_ptr(), target.as_ptr(), 4) };
    #[cfg(target_os = "linux")]
    let result = unsafe { renameat2(-100, source.as_ptr(), -100, target.as_ptr(), 1) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn install(_source: &Path, _target: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

pub fn new(source: &Path, target: &Path) -> Result<(), WorkError> {
    let result = install(source, target);
    #[cfg(unix)]
    let result = result.and_then(|()| {
        std::fs::File::open(target.parent().ok_or(std::io::ErrorKind::InvalidInput)?)?.sync_all()
    });
    result.map_err(|failure| {
        let mut issue = error("file_transaction_control_commit_failed");
        issue.details = serde_json::json!({"native_error_code":failure.raw_os_error(),"recovery_required":true});
        issue
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_record_is_installed_but_existing_foreign_record_is_never_replaced() {
        let root = std::env::temp_dir().join(format!(
            "work-control-commit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("authorization.tmp");
        let target = root.join("authorization.json");
        std::fs::write(&source, b"complete authorization").unwrap();
        new(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"complete authorization");
        std::fs::write(&source, b"different authorization").unwrap();
        assert!(new(&source, &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"complete authorization");
        assert_eq!(std::fs::read(&source).unwrap(), b"different authorization");
    }
}
