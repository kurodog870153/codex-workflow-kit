//! macOS ownership snapshots. Unsupported security metadata is never silently lost.
use work_feature::error::WorkError;
use work_feature::execution::file_transaction::error;
use work_model::execution::file_transaction::FileMetadata;

pub(super) fn ownership(snapshot: &FileMetadata) -> Result<(u32, u32), WorkError> {
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
    use std::ffi::c_void;
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
        // SAFETY: the live descriptor is borrowed; zero size requests only a length.
        // XATTR_SHOWCOMPRESSION also exposes otherwise hidden compression attributes.
        let size = unsafe { flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0, 0x20) };
        if size < 0 {
            return Err(error("file_transaction_xattrs_unverifiable"));
        }
        if size != 0 {
            return Err(error("file_transaction_xattrs_unsupported"));
        }
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

    pub fn write(path: &Path, snapshot: &FileMetadata) -> Result<(), WorkError> {
        let (uid, gid) = ownership(snapshot)?;
        let file = OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|_| error("file_transaction_metadata_read"))?;
        let current = read(&file)?;
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
pub(super) use native::{read, write};

#[cfg(test)]
mod tests {
    use super::*;
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
        let mut snapshot: FileMetadata =
            serde_json::from_str(r#"{"readonly":false,"unix_mode":33188}"#).unwrap();
        assert!(ownership(&snapshot).is_err());
        assert_eq!(
            serde_json::to_string(&snapshot).unwrap(),
            r#"{"readonly":false,"unix_mode":33188}"#
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
