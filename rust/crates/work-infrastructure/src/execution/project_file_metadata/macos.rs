//! macOS ownership snapshots. Unsupported security metadata is never silently lost.
use work_feature::error::WorkError;
use work_feature::execution::file_transaction::error;
use work_model::execution::file_transaction::FileMetadata;

pub(in crate::execution) fn ownership(snapshot: &FileMetadata) -> Result<(u32, u32), WorkError> {
    match (snapshot.unix_mode, snapshot.unix_uid, snapshot.unix_gid) {
        (Some(mode), Some(uid), Some(gid))
            if mode & 0o7000 == 0 && uid != u32::MAX && gid != u32::MAX =>
        {
            Ok((uid, gid))
        }
        _ => Err(error("file_transaction_unix_metadata_incomplete")),
    }
}

#[cfg(any(target_os = "macos", test))]
fn verify_uuid(
    stored: &[u8; 16],
    lookup: impl FnOnce() -> Result<[u8; 16], WorkError>,
) -> Result<(), WorkError> {
    if *stored == [0; 16] {
        return Ok(());
    }
    if *stored != lookup()? {
        return Err(error("file_transaction_owner_uuid_unsupported"));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use std::collections::BTreeMap;
    use std::ffi::{CString, c_char, c_void};
    use std::fs::{File, OpenOptions};
    use std::os::fd::AsRawFd;
    use std::os::macos::fs::MetadataExt as MacMetadataExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::Path;

    #[repr(C)]
    struct AttributeList {
        count: u16,
        reserved: u16,
        groups: [u32; 5],
    }

    unsafe extern "C" {
        fn flistxattr(fd: i32, names: *mut u8, size: usize, options: i32) -> isize;
        fn fgetxattr(
            fd: i32,
            name: *const c_char,
            value: *mut u8,
            size: usize,
            position: u32,
            options: i32,
        ) -> isize;
        fn fsetxattr(
            fd: i32,
            name: *const c_char,
            value: *const u8,
            size: usize,
            position: u32,
            options: i32,
        ) -> i32;
        fn acl_get_fd_np(fd: i32, kind: i32) -> *mut c_void;
        fn acl_free(acl: *mut c_void) -> i32;
        fn fchown(fd: i32, uid: u32, gid: u32) -> i32;
        fn mbr_uid_to_uuid(uid: u32, uuid: *mut u8) -> i32;
        fn mbr_gid_to_uuid(gid: u32, uuid: *mut u8) -> i32;
        fn fgetattrlist(
            fd: i32,
            attributes: *mut AttributeList,
            buffer: *mut c_void,
            size: usize,
            options: usize,
        ) -> i32;
    }

    pub fn read(file: &File) -> Result<(u32, u32), WorkError> {
        let metadata = file
            .metadata()
            .map_err(|_| error("file_transaction_metadata_read"))?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.st_flags() != 0
            || metadata.mode() & 0o7000 != 0
        {
            return Err(error("file_transaction_metadata_unsupported"));
        }
        // Numeric ownership can restore its corresponding DirectoryService UUID,
        // but cannot restore a separately stored, unrelated UUID identity.
        // ATTR_CMN_UUID | ATTR_CMN_GRPUUID returns length + two 16-byte UUIDs.
        let mut attributes = AttributeList {
            count: 5,
            reserved: 0,
            groups: [0x01800000, 0, 0, 0, 0],
        };
        let mut identities = [0_u32; 9];
        // SAFETY: repr(C) matches attrlist; the aligned buffer has the exact
        // documented fixed result size and remains live throughout the call.
        if unsafe {
            fgetattrlist(
                file.as_raw_fd(),
                &mut attributes,
                identities.as_mut_ptr().cast(),
                std::mem::size_of_val(&identities),
                0,
            )
        } != 0
            || identities[0] != 36
        {
            return Err(error("file_transaction_owner_unverifiable"));
        }
        let uuid = |words: &[u32]| -> [u8; 16] {
            let mut result = [0; 16];
            for (chunk, word) in result.chunks_exact_mut(4).zip(words) {
                chunk.copy_from_slice(&word.to_ne_bytes());
            }
            result
        };
        verify_uuid(&uuid(&identities[1..5]), || {
            let mut mapped = [0; 16];
            // SAFETY: uuid_t is a live sixteen-byte output buffer.
            if unsafe { mbr_uid_to_uuid(metadata.uid(), mapped.as_mut_ptr()) } != 0 {
                return Err(error("file_transaction_owner_unverifiable"));
            }
            Ok(mapped)
        })?;
        verify_uuid(&uuid(&identities[5..9]), || {
            let mut mapped = [0; 16];
            // SAFETY: uuid_t is a live sixteen-byte output buffer.
            if unsafe { mbr_gid_to_uuid(metadata.gid(), mapped.as_mut_ptr()) } != 0 {
                return Err(error("file_transaction_owner_unverifiable"));
            }
            Ok(mapped)
        })?;
        // SAFETY: ACL_TYPE_EXTENDED is 0x100; libc returns an owned ACL or errno.
        let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), 0x100) };
        if !acl.is_null() {
            // Even an empty explicit ACL can contain flags that mode cannot restore.
            // SAFETY: release exactly the owned allocation returned by libc.
            if unsafe { acl_free(acl) } != 0 {
                return Err(error("file_transaction_acl_unverifiable"));
            }
            return Err(error("file_transaction_acl_unsupported"));
        }
        // Darwin filesec_get_property reports ENOENT only when no ACL was present.
        // Permission/unsupported/query failures are not evidence of absence.
        if std::io::Error::last_os_error().raw_os_error() != Some(2) {
            return Err(error("file_transaction_acl_unverifiable"));
        }
        Ok((metadata.uid(), metadata.gid()))
    }

    const MAX_XATTR_BYTES: usize = 1024 * 1024;

    fn names(file: &File) -> Result<Vec<u8>, WorkError> {
        // SAFETY: the live descriptor is borrowed; SHOWCOMPRESSION includes hidden attributes.
        let size = unsafe { flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0, 0x20) };
        if size < 0 {
            return Err(error("file_transaction_xattrs_unverifiable"));
        }
        let size = size as usize;
        if size > MAX_XATTR_BYTES {
            return Err(error("file_transaction_metadata_resource_limit"));
        }
        if size == 0 {
            return Ok(Vec::new());
        }
        let mut names = vec![0; size];
        // SAFETY: the output buffer has exactly the queried capacity.
        if unsafe { flistxattr(file.as_raw_fd(), names.as_mut_ptr(), size, 0x20) } != size as isize
            || names.last() != Some(&0)
        {
            return Err(error("file_transaction_xattrs_unverifiable"));
        }
        Ok(names)
    }

    pub fn read_xattrs(file: &File) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
        let names_before = names(file)?;
        let mut result = BTreeMap::new();
        let mut total = names_before.len();
        for name in names_before
            .split(|byte| *byte == 0)
            .filter(|name| !name.is_empty())
        {
            let key = std::str::from_utf8(name)
                .map_err(|_| error("file_transaction_xattrs_unverifiable"))?;
            // These attributes carry security grants or compressed-file semantics beyond plain values.
            if matches!(key, "com.apple.macl" | "com.apple.decmpfs") {
                return Err(error("file_transaction_xattrs_unsupported"));
            }
            let name =
                CString::new(name).map_err(|_| error("file_transaction_xattrs_unverifiable"))?;
            // SAFETY: name is terminated; null queries size without accessing a value buffer.
            let length = unsafe {
                fgetxattr(
                    file.as_raw_fd(),
                    name.as_ptr(),
                    std::ptr::null_mut(),
                    0,
                    0,
                    0,
                )
            };
            if length < 0 {
                return Err(error("file_transaction_xattrs_unverifiable"));
            }
            total = total
                .checked_add(length as usize)
                .ok_or_else(|| error("file_transaction_metadata_resource_limit"))?;
            if total > MAX_XATTR_BYTES {
                return Err(error("file_transaction_metadata_resource_limit"));
            }
            let mut value = vec![0; length as usize];
            // SAFETY: descriptor and name remain live; output capacity equals the checked length.
            if unsafe {
                fgetxattr(
                    file.as_raw_fd(),
                    name.as_ptr(),
                    value.as_mut_ptr(),
                    value.len(),
                    0,
                    0,
                )
            } != length
            {
                return Err(error("file_transaction_xattrs_unverifiable"));
            }
            if result.insert(key.to_owned(), value).is_some() {
                return Err(error("file_transaction_xattrs_unverifiable"));
            }
        }
        if names(file)? != names_before {
            return Err(error("file_transaction_target_drift"));
        }
        Ok(result)
    }

    pub fn write(path: &Path, snapshot: &FileMetadata) -> Result<(), WorkError> {
        let (uid, gid) = ownership(snapshot)?;
        let expected = snapshot
            .macos_xattrs
            .as_ref()
            .ok_or_else(|| error("file_transaction_xattrs_incomplete"))?;
        let file = OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|_| error("file_transaction_metadata_read"))?;
        let current = read(&file)?;
        let current_xattrs = read_xattrs(&file)?;
        // Never remove an unapproved attribute, even on a temporary. Refuse incompatible inheritance.
        if current_xattrs.keys().any(|key| !expected.contains_key(key)) {
            return Err(error("file_transaction_xattrs_not_restorable"));
        }
        let mut total = 0_usize;
        for (key, value) in expected {
            total = total
                .checked_add(key.len())
                .and_then(|size| size.checked_add(value.len()))
                .ok_or_else(|| error("file_transaction_metadata_resource_limit"))?;
            if total > MAX_XATTR_BYTES {
                return Err(error("file_transaction_metadata_resource_limit"));
            }
            if key.is_empty() || matches!(key.as_str(), "com.apple.macl" | "com.apple.decmpfs") {
                return Err(error("file_transaction_xattrs_unsupported"));
            }
            let name = CString::new(key.as_bytes())
                .map_err(|_| error("file_transaction_xattrs_unverifiable"))?;
            // SAFETY: borrowed live descriptor, terminated name, and value buffer; no position/extra options.
            if current_xattrs.get(key) != Some(value)
                && unsafe {
                    fsetxattr(
                        file.as_raw_fd(),
                        name.as_ptr(),
                        value.as_ptr(),
                        value.len(),
                        0,
                        0,
                    )
                } != 0
            {
                return Err(error("file_transaction_xattrs_not_restorable"));
            }
        }
        if current != (uid, gid) {
            // SAFETY: fd remains open; sentinel IDs were rejected by ownership().
            if unsafe { fchown(file.as_raw_fd(), uid, gid) } != 0 {
                return Err(error("file_transaction_owner_not_restorable"));
            }
        }
        // chown may clear permission bits, so mode must be applied after ownership.
        file.set_permissions(std::fs::Permissions::from_mode(snapshot.unix_mode.unwrap()))
            .map_err(|_| error("file_transaction_metadata_write"))?;
        if read(&file)? != (uid, gid)
            || read_xattrs(&file)? != *expected
            || file
                .metadata()
                .map_err(|_| error("file_transaction_metadata_read"))?
                .mode()
                != snapshot.unix_mode.unwrap()
        {
            return Err(error("file_transaction_metadata_readback"));
        }
        file.sync_all()
            .map_err(|_| error("file_transaction_metadata_write"))
    }
}
#[cfg(target_os = "macos")]
pub(in crate::execution) use native::{read, read_xattrs, write};

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn xattr_snapshot_requires_complete_evidence_and_never_removes_unapproved_attributes() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let path = std::env::temp_dir().join(format!(
            "work-xattr-snapshot-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"preserved bytes").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let metadata = file.metadata().unwrap();
        let mut snapshot = FileMetadata {
            readonly: false,
            unix_mode: Some(metadata.permissions().mode()),
            unix_uid: Some(metadata.uid()),
            unix_gid: Some(metadata.gid()),
            windows_security: None,
            macos_xattrs: None,
        };
        assert_eq!(
            write(&path, &snapshot).unwrap_err().reason_code,
            "file_transaction_xattrs_incomplete"
        );
        snapshot.macos_xattrs = Some(read_xattrs(&file).unwrap());
        assert!(
            std::process::Command::new("/usr/bin/xattr")
                .args(["-w", "com.work.unapproved", "retain"])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        let unchanged = read_xattrs(&file).unwrap();
        assert_eq!(
            write(&path, &snapshot).unwrap_err().reason_code,
            "file_transaction_xattrs_not_restorable"
        );
        assert_eq!(read_xattrs(&file).unwrap(), unchanged);
        snapshot.macos_xattrs = Some(unchanged.clone());
        snapshot
            .macos_xattrs
            .as_mut()
            .unwrap()
            .insert("x".repeat(1024), vec![1]);
        assert_eq!(
            write(&path, &snapshot).unwrap_err().reason_code,
            "file_transaction_xattrs_not_restorable"
        );
        assert_eq!(read_xattrs(&file).unwrap(), unchanged);
        assert_eq!(std::fs::read(&path).unwrap(), b"preserved bytes");
        snapshot.macos_xattrs = Some(unchanged.clone());
        snapshot
            .macos_xattrs
            .as_mut()
            .unwrap()
            .insert("com.apple.macl".into(), vec![0]);
        assert_eq!(
            write(&path, &snapshot).unwrap_err().reason_code,
            "file_transaction_xattrs_unsupported"
        );
        assert_eq!(read_xattrs(&file).unwrap(), unchanged);
        let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let socket_file = std::fs::File::from(std::os::fd::OwnedFd::from(socket));
        assert_eq!(
            read_xattrs(&socket_file).unwrap_err().reason_code,
            "file_transaction_xattrs_unverifiable"
        );
        snapshot.macos_xattrs = Some(unchanged.clone());
        snapshot
            .macos_xattrs
            .as_mut()
            .unwrap()
            .insert("com.work.oversized".into(), vec![0; 1024 * 1024 + 1]);
        assert_eq!(
            write(&path, &snapshot).unwrap_err().reason_code,
            "file_transaction_metadata_resource_limit"
        );
        assert_eq!(read_xattrs(&file).unwrap(), unchanged);
    }
    #[test]
    fn uuid_ownership_requires_exact_numeric_identity_mapping() {
        verify_uuid(&[0; 16], || panic!("zero UUID needs no lookup")).unwrap();
        verify_uuid(&[7; 16], || Ok([7; 16])).unwrap();
        assert_eq!(
            verify_uuid(&[7; 16], || Ok([8; 16]))
                .unwrap_err()
                .reason_code,
            "file_transaction_owner_uuid_unsupported"
        );
        assert_eq!(
            verify_uuid(&[7; 16], || Err(error(
                "file_transaction_owner_unverifiable"
            )))
            .unwrap_err()
            .reason_code,
            "file_transaction_owner_unverifiable"
        );
    }
    #[test]
    fn legacy_and_partial_ownership_evidence_is_never_inferred() {
        let mut snapshot: FileMetadata = serde_json::from_str(include_str!(
            "../../../fixtures/file-transaction/legacy-file-metadata.json"
        ))
        .unwrap();
        assert!(ownership(&snapshot).is_err());
        assert_eq!(
            serde_json::to_string(&snapshot).unwrap(),
            include_str!("../../../fixtures/file-transaction/legacy-file-metadata.json")
        );
        snapshot.unix_uid = Some(501);
        assert!(ownership(&snapshot).is_err());
        snapshot.unix_gid = Some(20);
        assert_eq!(ownership(&snapshot).unwrap(), (501, 20));
        snapshot.unix_uid = Some(u32::MAX);
        assert!(ownership(&snapshot).is_err());
        snapshot.unix_uid = Some(501);
        snapshot.unix_mode = Some(0o104644);
        assert!(ownership(&snapshot).is_err());
    }
}
