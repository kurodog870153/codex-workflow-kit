//! Native Windows security snapshot and rejection of unsaved named streams.
#[cfg(any(windows, test))]
fn canonical_descriptor(descriptor: &[u8]) -> Result<Vec<u8>, work_feature::error::WorkError> {
    use work_feature::execution::file_transaction::error;
    let invalid = || error("file_transaction_security_invalid");
    if descriptor.len() < 20
        || descriptor.len() > 4096
        || descriptor[0] != 1
        || u16::from_le_bytes([descriptor[2], descriptor[3]]) & 0x8004 != 0x8004
        || descriptor[12..16] != [0; 4]
    {
        return Err(invalid());
    }
    let mut result = descriptor[..20].to_vec();
    let mut ranges = Vec::new();
    for field in [4, 8, 16] {
        let offset = u32::from_le_bytes(descriptor[field..field + 4].try_into().unwrap()) as usize;
        let header = descriptor
            .get(offset..offset.checked_add(8).ok_or_else(invalid)?)
            .filter(|_| offset >= 20 && offset % 4 == 0)
            .ok_or_else(invalid)?;
        let length = if field == 16 {
            usize::from(u16::from_le_bytes([header[2], header[3]]))
        } else {
            if header[0] != 1 || header[1] > 15 {
                return Err(invalid());
            }
            8 + 4 * usize::from(header[1])
        };
        if length < 8 || length % 4 != 0 {
            return Err(invalid());
        }
        let end = offset.checked_add(length).ok_or_else(invalid)?;
        let part = descriptor.get(offset..end).ok_or_else(invalid)?;
        if ranges.iter().any(|&(start, last, previous)| {
            offset < last
                && start < end
                && !(field == 8 && previous == 4 && offset == start && end == last)
        }) {
            return Err(invalid());
        }
        ranges.push((offset, end, field));
        let at = result.len() as u32;
        result[field..field + 4].copy_from_slice(&at.to_le_bytes());
        result.extend_from_slice(part);
    }
    // Preserve all control flags, SID bytes, ACE order, masks and inheritance
    // flags. Only self-relative packing and unused inter-component padding vary.
    Ok(result)
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use work_feature::error::WorkError;
    use work_feature::execution::file_transaction::error;
    #[repr(C)]
    struct Stream {
        size: i64,
        name: [u16; 296],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn FindFirstStreamW(
            name: *const u16,
            level: u32,
            data: *mut c_void,
            flags: u32,
        ) -> *mut c_void;
        fn FindNextStreamW(handle: *mut c_void, data: *mut c_void) -> i32;
        fn FindClose(handle: *mut c_void) -> i32;
        fn GetLastError() -> u32;
    }
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn GetFileSecurityW(
            name: *const u16,
            information: u32,
            descriptor: *mut c_void,
            length: u32,
            needed: *mut u32,
        ) -> i32;
        fn SetFileSecurityW(name: *const u16, information: u32, descriptor: *const c_void) -> i32;
    }
    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain([0]).collect()
    }
    pub fn read(path: &Path) -> Result<Vec<u8>, WorkError> {
        let name = wide(path);
        let mut stream = Stream {
            size: 0,
            name: [0; 296],
        };
        // SAFETY: buffers match the documented Win32 layouts and remain live through each call.
        unsafe {
            let handle = FindFirstStreamW(name.as_ptr(), 0, (&mut stream as *mut Stream).cast(), 0);
            if handle as isize == -1 {
                return Err(error("file_transaction_streams_unverifiable"));
            }
            let result = (|| {
                loop {
                    let len = stream
                        .name
                        .iter()
                        .position(|c| *c == 0)
                        .unwrap_or(stream.name.len());
                    if String::from_utf16_lossy(&stream.name[..len]) != "::$DATA" {
                        return Err(error("file_transaction_named_stream_unsupported"));
                    }
                    if FindNextStreamW(handle, (&mut stream as *mut Stream).cast()) == 0 {
                        if GetLastError() != 38 {
                            return Err(error("file_transaction_streams_unverifiable"));
                        }
                        break;
                    }
                }
                let mut needed = 0;
                GetFileSecurityW(name.as_ptr(), 7, std::ptr::null_mut(), 0, &mut needed);
                if needed == 0 {
                    return Err(error("file_transaction_security_unverifiable"));
                }
                if needed > 4096 {
                    return Err(error("file_transaction_metadata_resource_limit"));
                }
                let mut aligned = vec![0u32; (needed as usize).div_ceil(4)];
                if GetFileSecurityW(
                    name.as_ptr(),
                    7,
                    aligned.as_mut_ptr().cast(),
                    needed,
                    &mut needed,
                ) == 0
                {
                    return Err(error("file_transaction_security_unverifiable"));
                }
                // SAFETY: the byte view is within the initialized aligned allocation.
                let descriptor =
                    std::slice::from_raw_parts(aligned.as_ptr().cast::<u8>(), needed as usize)
                        .to_vec();
                super::canonical_descriptor(&descriptor)
            })();
            if FindClose(handle) == 0 {
                return Err(error("file_transaction_stream_handle_close"));
            }
            result
        }
    }
    pub fn write(path: &Path, descriptor: &[u8]) -> Result<(), WorkError> {
        let canonical = super::canonical_descriptor(descriptor)?;
        let descriptor = canonical.as_slice();
        if descriptor.len() < 20 || descriptor[0] != 1 {
            return Err(error("file_transaction_security_invalid"));
        }
        let control = u16::from_le_bytes([descriptor[2], descriptor[3]]);
        if control & 0x8004 != 0x8004 {
            return Err(error("file_transaction_security_invalid"));
        }
        let offsets: Vec<_> = (4..20)
            .step_by(4)
            .map(|n| {
                u32::from_le_bytes(descriptor[n..n + 4].try_into().expect("bounded header"))
                    as usize
            })
            .collect();
        for offset in &offsets[..2] {
            if *offset < 20
                || offset % 4 != 0
                || offset
                    .checked_add(8)
                    .is_none_or(|end| end > descriptor.len())
                || descriptor[*offset] != 1
                || descriptor[*offset + 1] > 15
                || offset
                    .checked_add(8 + 4 * descriptor[*offset + 1] as usize)
                    .is_none_or(|end| end > descriptor.len())
            {
                return Err(error("file_transaction_security_invalid"));
            }
        }
        let acl = offsets[3];
        if offsets[2] != 0
            || acl < 20
            || acl % 4 != 0
            || acl.checked_add(8).is_none_or(|end| end > descriptor.len())
        {
            return Err(error("file_transaction_security_invalid"));
        }
        let size = u16::from_le_bytes([descriptor[acl + 2], descriptor[acl + 3]]) as usize;
        if size < 8
            || acl
                .checked_add(size)
                .is_none_or(|end| end > descriptor.len())
        {
            return Err(error("file_transaction_security_invalid"));
        }
        let count = u16::from_le_bytes([descriptor[acl + 4], descriptor[acl + 5]]) as usize;
        let mut at = acl + 8;
        for _ in 0..count {
            if at + 4 > acl + size {
                return Err(error("file_transaction_security_invalid"));
            }
            let length = u16::from_le_bytes([descriptor[at + 2], descriptor[at + 3]]) as usize;
            if length < 4 || at.checked_add(length).is_none_or(|end| end > acl + size) {
                return Err(error("file_transaction_security_invalid"));
            }
            at += length;
        }
        let protection = if control & 0x1000 != 0 {
            0x80000000
        } else {
            0x20000000
        };
        let name = wide(path);
        let mut aligned = vec![0u32; descriptor.len().div_ceil(4)];
        // SAFETY: the byte view exactly spans the initialized aligned descriptor allocation.
        unsafe {
            std::slice::from_raw_parts_mut(aligned.as_mut_ptr().cast::<u8>(), aligned.len() * 4)
                [..descriptor.len()]
                .copy_from_slice(descriptor);
        }
        // SetNamedSecurityInfo automatically merges inherited ACEs from the staging
        // directory, which is not necessarily the original target's parent. Use the
        // low-level file API here specifically to restore the saved descriptor and
        // its inheritance control bits without recomputing the saved DACL.
        // SAFETY: all SID/ACL offsets and lengths in this aligned descriptor were
        // checked above; the complete self-relative descriptor stays live.
        let result =
            unsafe { SetFileSecurityW(name.as_ptr(), 7 | protection, aligned.as_ptr().cast()) };
        if result == 0 {
            let mut failure = error("file_transaction_security_not_restorable");
            // SAFETY: read the calling thread's last error immediately after failure.
            failure.details["native_error_code"] = serde_json::json!(unsafe { GetLastError() });
            return Err(failure);
        }
        if read(path)? != descriptor {
            return Err(error("file_transaction_security_readback"));
        }
        Ok(())
    }
}
#[cfg(windows)]
pub(super) use windows::{read, write};

#[cfg(test)]
mod tests {
    use super::*;
    fn descriptor(order: &[usize]) -> Vec<u8> {
        let owner = vec![1, 1, 0, 0, 0, 0, 0, 5, 1, 0, 0, 0];
        let group = vec![1, 1, 0, 0, 0, 0, 0, 5, 2, 0, 0, 0];
        let mut acl = vec![2, 0, 28, 0, 1, 0, 0, 0, 0, 0, 20, 0, 0x89, 0, 0x12, 0];
        acl.extend_from_slice(&owner);
        let parts = [owner, group, acl];
        let mut bytes = vec![0; 20];
        bytes[0] = 1;
        bytes[2..4].copy_from_slice(&0x8404_u16.to_le_bytes());
        for &index in order {
            let field = [4, 8, 16][index];
            let offset = bytes.len() as u32;
            bytes[field..field + 4].copy_from_slice(&offset.to_le_bytes());
            bytes.extend_from_slice(&parts[index]);
        }
        bytes
    }
    #[test]
    fn security_packing_can_vary_but_identities_permissions_and_control_cannot() {
        let saved = canonical_descriptor(&descriptor(&[0, 1, 2])).unwrap();
        assert_eq!(
            canonical_descriptor(&descriptor(&[2, 1, 0])).unwrap(),
            saved
        );
        for position in [2, 3, 28, 40, 56, 57] {
            let mut changed = saved.clone();
            changed[position] ^= if position == 2 { 1 } else { 0x10 };
            assert_ne!(canonical_descriptor(&changed).unwrap(), saved);
        }
        for position in [4, 8, 16] {
            let mut corrupt = saved.clone();
            corrupt[position..position + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(canonical_descriptor(&corrupt).is_err());
        }
    }
    #[test]
    fn shared_owner_group_sid_is_valid_but_acl_aliasing_is_not() {
        let mut shared = descriptor(&[0, 1, 2]);
        let owner = shared[4..8].to_vec();
        shared[8..12].copy_from_slice(&owner);
        let normalized = canonical_descriptor(&shared).unwrap();
        assert_eq!(&normalized[20..32], &normalized[32..44]);
        let mut separate = descriptor(&[0, 1, 2]);
        let sid = separate[20..32].to_vec();
        separate[32..44].copy_from_slice(&sid);
        assert_eq!(canonical_descriptor(&separate).unwrap(), normalized);
        shared[16..20].copy_from_slice(&owner);
        assert!(canonical_descriptor(&shared).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn native_security_roundtrip_retains_inheritance_and_ace_order() {
        let dir = std::env::temp_dir().join(format!(
            "work-security-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("other-parent")).unwrap();
        let original = dir.join("original");
        let temporary = dir.join("other-parent/temporary");
        std::fs::write(&original, b"original").unwrap();
        std::fs::write(&temporary, b"staged").unwrap();
        let evidence = read(&original).unwrap();
        write(&temporary, &evidence).unwrap();
        assert_eq!(read(&temporary).unwrap(), evidence);
        assert_eq!(read(&original).unwrap(), evidence);
    }
}
