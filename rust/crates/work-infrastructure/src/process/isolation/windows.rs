//! LPAC without network capabilities, private file grants, and a non-breakaway Job.
//! No ACL on the original project or executable is changed, and no package profile is created.
use super::*;
use crate::process::ProcessCapture;
use std::ffi::c_void;
use std::fs::File;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::os::windows::io::FromRawHandle;
use std::ptr::{null, null_mut};
use std::time::Instant;
use work_feature::ports::CommandStatus;

type RawHandle = *mut c_void;
#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit: i32,
}
#[repr(C)]
struct Startup {
    size: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    chars_x: u32,
    chars_y: u32,
    fill: u32,
    flags: u32,
    show: u16,
    reserved_bytes: u16,
    reserved_pointer: *mut u8,
    input: RawHandle,
    output: RawHandle,
    error: RawHandle,
}
#[repr(C)]
struct StartupEx {
    startup: Startup,
    attributes: *mut c_void,
}
#[repr(C)]
struct ProcessInformation {
    process: RawHandle,
    thread: RawHandle,
    process_id: u32,
    thread_id: u32,
}
#[repr(C)]
struct SecurityCapabilities {
    sid: *mut c_void,
    capabilities: *mut c_void,
    count: u32,
    reserved: u32,
}
#[repr(C)]
struct Trustee {
    multiple: *mut c_void,
    operation: u32,
    form: u32,
    kind: u32,
    name: *mut u16,
}
#[repr(C)]
struct ExplicitAccess {
    permissions: u32,
    mode: u32,
    inheritance: u32,
    trustee: Trustee,
}
#[repr(C)]
struct BasicLimits {
    process_time: i64,
    job_time: i64,
    flags: u32,
    minimum: usize,
    maximum: usize,
    active: u32,
    affinity: usize,
    priority: u32,
    scheduling: u32,
}
#[repr(C)]
struct ExtendedLimits {
    basic: BasicLimits,
    io: [u64; 6],
    process_memory: usize,
    job_memory: usize,
    peak_process_memory: usize,
    peak_job_memory: usize,
}
#[repr(C)]
struct Version {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    service_pack: [u16; 128],
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CloseHandle(handle: RawHandle) -> i32;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
    fn CreatePipe(
        read: *mut RawHandle,
        write: *mut RawHandle,
        attributes: *mut SecurityAttributes,
        size: u32,
    ) -> i32;
    fn SetHandleInformation(handle: RawHandle, mask: u32, flags: u32) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        attributes: *mut SecurityAttributes,
        creation: u32,
        flags: u32,
        template: RawHandle,
    ) -> RawHandle;
    fn InitializeProcThreadAttributeList(
        list: *mut c_void,
        count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *mut c_void,
        size: usize,
        previous: *mut c_void,
        returned: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
    fn CreateProcessW(
        application: *const u16,
        command: *mut u16,
        process_attributes: *mut SecurityAttributes,
        thread_attributes: *mut SecurityAttributes,
        inherit: i32,
        flags: u32,
        environment: *mut c_void,
        cwd: *const u16,
        startup: *mut Startup,
        information: *mut ProcessInformation,
    ) -> i32;
    fn CreateJobObjectW(attributes: *mut SecurityAttributes, name: *const u16) -> RawHandle;
    fn SetInformationJobObject(
        job: RawHandle,
        class: i32,
        information: *mut c_void,
        length: u32,
    ) -> i32;
    fn AssignProcessToJobObject(job: RawHandle, process: RawHandle) -> i32;
    fn TerminateJobObject(job: RawHandle, code: u32) -> i32;
    fn TerminateProcess(process: RawHandle, code: u32) -> i32;
    fn ResumeThread(thread: RawHandle) -> u32;
    fn WaitForSingleObject(handle: RawHandle, timeout: u32) -> u32;
    fn GetExitCodeProcess(process: RawHandle, code: *mut u32) -> i32;
    fn GetWindowsDirectoryW(buffer: *mut u16, length: u32) -> u32;
}
#[link(name = "advapi32")]
unsafe extern "system" {
    fn FreeSid(sid: *mut c_void) -> *mut c_void;
    fn GetNamedSecurityInfoW(
        name: *const u16,
        kind: u32,
        information: u32,
        owner: *mut *mut c_void,
        group: *mut *mut c_void,
        dacl: *mut *mut c_void,
        sacl: *mut *mut c_void,
        descriptor: *mut *mut c_void,
    ) -> u32;
    fn SetEntriesInAclW(
        count: u32,
        entries: *mut ExplicitAccess,
        old: *mut c_void,
        new: *mut *mut c_void,
    ) -> u32;
    fn SetNamedSecurityInfoW(
        name: *const u16,
        kind: u32,
        information: u32,
        owner: *mut c_void,
        group: *mut c_void,
        dacl: *mut c_void,
        sacl: *mut c_void,
    ) -> u32;
}
#[link(name = "userenv")]
unsafe extern "system" {
    fn DeriveAppContainerSidFromAppContainerName(name: *const u16, sid: *mut *mut c_void) -> i32;
}
#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(version: *mut Version) -> i32;
}

struct Handle(RawHandle);
impl Handle {
    fn new(raw: RawHandle) -> Result<Self, WorkError> {
        if raw.is_null() || raw as isize == -1 {
            Err(unavailable())
        } else {
            Ok(Self(raw))
        }
    }
    fn into_file(self) -> File {
        let owned = std::mem::ManuallyDrop::new(self);
        // SAFETY: ownership of this valid pipe handle is transferred exactly once.
        unsafe { File::from_raw_handle(owned.0) }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Sid(*mut c_void);
impl Drop for Sid {
    fn drop(&mut self) {
        unsafe {
            FreeSid(self.0);
        }
    }
}
struct Allocation(*mut c_void);
impl Drop for Allocation {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
struct Attributes {
    storage: Vec<u128>,
}
impl Attributes {
    fn new(count: u32) -> Result<Self, WorkError> {
        let mut size = 0;
        // SAFETY: null queries required size without writing an attribute list.
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), count, 0, &mut size);
        }
        if size == 0 || size > 65536 {
            return Err(unavailable());
        }
        let mut storage = vec![0_u128; size.div_ceil(16)];
        // SAFETY: live sixteen-byte-aligned storage has the queried capacity.
        check(unsafe {
            InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), count, 0, &mut size)
        })?;
        Ok(Self { storage })
    }
    fn pointer(&mut self) -> *mut c_void {
        self.storage.as_mut_ptr().cast()
    }
    fn set<T>(&mut self, attribute: usize, value: &mut T) -> Result<(), WorkError> {
        // SAFETY: the caller keeps this initialized attribute payload live until
        // CreateProcess returns. The native list owns no payload allocation.
        check(unsafe {
            UpdateProcThreadAttribute(
                self.pointer(),
                0,
                attribute,
                (value as *mut T).cast(),
                std::mem::size_of::<T>(),
                null_mut(),
                null_mut(),
            )
        })
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.pointer());
        }
    }
}
fn check(success: i32) -> Result<(), WorkError> {
    if success == 0 {
        Err(unavailable())
    } else {
        Ok(())
    }
}
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}
pub(super) fn available() -> bool {
    let mut version = Version {
        size: std::mem::size_of::<Version>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        service_pack: [0; 128],
    };
    // SAFETY: RTL_OSVERSIONINFOW is initialized and correctly sized.
    unsafe { RtlGetVersion(&mut version) == 0 && version.major >= 10 }
}

fn grant(path: &Path, sid: *mut c_void, writable: bool) -> Result<(), WorkError> {
    let name = wide(path);
    let mut descriptor = null_mut();
    let mut old_acl = null_mut();
    // SAFETY: API allocates the descriptor; ACL is borrowed from that allocation.
    if unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            1,
            4,
            null_mut(),
            null_mut(),
            &mut old_acl,
            null_mut(),
            &mut descriptor,
        )
    } != 0
    {
        return Err(unavailable());
    }
    let _descriptor = Allocation(descriptor);
    let mut entry = ExplicitAccess {
        permissions: if writable { 0x001f01ff } else { 0x001200a9 },
        mode: 1,
        inheritance: if writable && path.is_dir() { 3 } else { 0 },
        trustee: Trustee {
            multiple: null_mut(),
            operation: 0,
            form: 0,
            kind: 5,
            name: sid.cast(),
        },
    };
    let mut acl = null_mut();
    // SAFETY: the validated package SID and borrowed old ACL stay live.
    if unsafe { SetEntriesInAclW(1, &mut entry, old_acl, &mut acl) } != 0 {
        return Err(unavailable());
    }
    let _acl = Allocation(acl);
    // Only freshly allocated sandbox paths are passed to this function.
    if unsafe {
        SetNamedSecurityInfoW(name.as_ptr(), 1, 4, null_mut(), null_mut(), acl, null_mut())
    } != 0
    {
        return Err(unavailable());
    }
    Ok(())
}

fn copy_read_only(
    source: &Path,
    destination: &Path,
    exclude: &Path,
    sid: *mut c_void,
    remaining: &mut u64,
    entries: &mut usize,
) -> Result<(), WorkError> {
    let metadata = source.symlink_metadata().map_err(|_| unavailable())?;
    if metadata.file_attributes() & 0x400 != 0 || metadata.file_type().is_symlink() {
        return Err(unavailable());
    }
    *entries = entries.checked_add(1).ok_or_else(unavailable)?;
    if *entries > 10000 {
        return Err(unavailable());
    }
    if metadata.is_dir() {
        std::fs::create_dir(destination).map_err(|_| unavailable())?;
        for entry in std::fs::read_dir(source).map_err(|_| unavailable())? {
            let entry = entry.map_err(|_| unavailable())?;
            let path = entry.path();
            let old_snapshot = path
                .file_name()
                .is_some_and(|name| name == "project-read-only" || name == "tools-read-only")
                && path.parent().and_then(Path::parent) == exclude.parent();
            if path == exclude || old_snapshot {
                continue;
            }
            copy_read_only(
                &entry.path(),
                &destination.join(entry.file_name()),
                exclude,
                sid,
                remaining,
                entries,
            )?;
        }
    } else if metadata.is_file() {
        *remaining = remaining
            .checked_sub(metadata.len())
            .ok_or_else(unavailable)?;
        std::fs::copy(source, destination).map_err(|_| unavailable())?;
    } else {
        return Err(unavailable());
    }
    grant(destination, sid, false)
}

fn quote(argument: &str) -> String {
    if !argument.is_empty() && !argument.chars().any(|c| c.is_whitespace() || c == '"') {
        return argument.into();
    }
    let mut result = String::from("\"");
    let mut slashes = 0;
    for c in argument.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '"' {
            result.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
        } else {
            result.extend(std::iter::repeat_n('\\', slashes));
        }
        result.push(c);
        slashes = 0;
    }
    result.extend(std::iter::repeat_n('\\', slashes * 2));
    result.push('"');
    result
}
fn pipe(attributes: &mut SecurityAttributes) -> Result<(Handle, Handle), WorkError> {
    let mut read = null_mut();
    let mut write = null_mut();
    // SAFETY: out pointers and initialized SECURITY_ATTRIBUTES remain live.
    check(unsafe { CreatePipe(&mut read, &mut write, attributes, 0) })?;
    let read = Handle::new(read)?;
    let write = Handle::new(write)?;
    check(unsafe { SetHandleInformation(read.0, 1, 0) })?;
    Ok((read, write))
}

pub(super) fn run(
    request: &CommandRequest,
    policy: &CommandIsolation,
) -> Result<ProcessCapture, WorkError> {
    let staging = Path::new(&policy.writable_directory);
    let allocation = staging.parent().ok_or_else(unavailable)?;
    let root = Path::new(&policy.read_only_project_root);
    let identity = fingerprint::raw(policy.writable_directory.as_bytes());
    let name = wide(Path::new(&format!("work.command.{}", &identity[..40])));
    let mut raw_sid = null_mut();
    // The moniker is private to this fresh receipt allocation. Deriving its SID
    // does not create a writable package/profile outside the authorized staging.
    if unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut raw_sid) } < 0
        || raw_sid.is_null()
    {
        return Err(unavailable());
    }
    let sid = Sid(raw_sid);
    grant(allocation, sid.0, false)?;
    grant(staging, sid.0, true)?;
    let snapshot = PathBuf::from(&policy.read_only_project_view);
    copy_read_only(
        root,
        &snapshot,
        allocation,
        sid.0,
        &mut (64 * 1024 * 1024),
        &mut 0,
    )?;
    let cwd = snapshot.join(request.cwd.strip_prefix(root).map_err(|_| unavailable())?);
    let original_image = Path::new(&request.argv[0])
        .canonicalize()
        .map_err(|_| unavailable())?;
    let image = if let Ok(relative) = original_image.strip_prefix(root) {
        snapshot.join(relative)
    } else {
        let mut system = [0_u16; 32768];
        let count =
            unsafe { GetWindowsDirectoryW(system.as_mut_ptr(), system.len() as u32) } as usize;
        if count == 0 || count >= system.len() {
            return Err(unavailable());
        }
        let windows =
            PathBuf::from(String::from_utf16(&system[..count]).map_err(|_| unavailable())?)
                .canonicalize()
                .map_err(|_| unavailable())?;
        if original_image.starts_with(&windows) {
            original_image.to_path_buf()
        } else {
            // A private image copy avoids granting a package SID on the user's
            // installed tool. Dependencies not readable in LPAC fail closed.
            let tools = allocation.join("tools-read-only");
            std::fs::create_dir(&tools).map_err(|_| unavailable())?;
            let image = tools.join(original_image.file_name().ok_or_else(unavailable)?);
            std::fs::copy(&original_image, &image).map_err(|_| unavailable())?;
            grant(&image, sid.0, false)?;
            grant(&tools, sid.0, false)?;
            image
        }
    };
    if request.argv.iter().any(|a| a.contains('\0')) {
        return Err(unavailable());
    }
    let command_line = if image
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("cmd.exe"))
    {
        let position = request
            .argv
            .iter()
            .position(|arg| arg.eq_ignore_ascii_case("/c"))
            .ok_or_else(unavailable)?;
        if request.argv.len() != position + 2 {
            return Err(unavailable());
        }
        let prefix = std::iter::once(quote(image.to_str().ok_or_else(unavailable)?))
            .chain(request.argv[1..=position].iter().map(|arg| quote(arg)))
            .collect::<Vec<_>>()
            .join(" ");
        let script = &request.argv[position + 1];
        let script = if request.argv[1..position]
            .iter()
            .any(|arg| arg.eq_ignore_ascii_case("/s"))
        {
            format!("\"{script}\"")
        } else {
            script.clone()
        };
        format!("{prefix} {script}")
    } else {
        std::iter::once(quote(image.to_str().ok_or_else(unavailable)?))
            .chain(request.argv.iter().skip(1).map(|arg| quote(arg)))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut command_line = wide(Path::new(&command_line));
    let image = wide(&image);
    let cwd = wide(&cwd);
    let mut environment = std::collections::BTreeMap::new();
    for name in [
        "HOME",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "TEMP",
        "TMP",
        "WORK_STAGING_DIR",
    ] {
        environment.insert(name, policy.writable_directory.clone());
    }
    let mut windows = [0_u16; 32768];
    let length =
        unsafe { GetWindowsDirectoryW(windows.as_mut_ptr(), windows.len() as u32) } as usize;
    if length == 0 || length >= windows.len() {
        return Err(unavailable());
    }
    let windows = String::from_utf16(&windows[..length]).map_err(|_| unavailable())?;
    environment.insert("SYSTEMROOT", windows.clone());
    environment.insert("WINDIR", windows.clone());
    environment.insert("PATH", format!("{windows}\\System32;{windows}"));
    let mut environment: Vec<u16> = environment
        .into_iter()
        .flat_map(|(name, value)| {
            format!("{name}={value}")
                .encode_utf16()
                .chain([0])
                .collect::<Vec<_>>()
        })
        .chain([0])
        .collect();
    let mut security = SecurityAttributes {
        length: std::mem::size_of::<SecurityAttributes>() as u32,
        descriptor: null_mut(),
        inherit: 1,
    };
    let (out_read, out_write) = pipe(&mut security)?;
    let (err_read, err_write) = pipe(&mut security)?;
    let input = Handle::new(unsafe {
        CreateFileW(
            wide(Path::new("NUL")).as_ptr(),
            0x80000000,
            3,
            &mut security,
            3,
            0,
            null_mut(),
        )
    })?;
    let job = Handle::new(unsafe { CreateJobObjectW(null_mut(), null()) })?;
    // SAFETY: these C layouts contain only integer and nullable pointer fields.
    let mut limits: ExtendedLimits = unsafe { std::mem::zeroed() };
    limits.basic.flags = 0x2000; // KILL_ON_JOB_CLOSE; no BREAKAWAY flags.
    check(unsafe {
        SetInformationJobObject(
            job.0,
            9,
            (&mut limits as *mut ExtendedLimits).cast(),
            std::mem::size_of::<ExtendedLimits>() as u32,
        )
    })?;
    let mut attributes = Attributes::new(3)?;
    let mut capabilities = SecurityCapabilities {
        sid: sid.0,
        capabilities: null_mut(),
        count: 0,
        reserved: 0,
    };
    let mut opt_out = 1_u32; // LPAC opts out of ALL_APPLICATION_PACKAGES grants.
    let mut handles = [input.0, out_write.0, err_write.0];
    attributes.set(0x00020009, &mut capabilities)?;
    attributes.set(0x0002000f, &mut opt_out)?;
    attributes.set(0x00020002, &mut handles)?;
    let mut startup: StartupEx = unsafe { std::mem::zeroed() };
    startup.startup.size = std::mem::size_of::<StartupEx>() as u32;
    startup.startup.flags = 0x100;
    startup.startup.input = input.0;
    startup.startup.output = out_write.0;
    startup.startup.error = err_write.0;
    startup.attributes = attributes.pointer();
    let mut process: ProcessInformation = unsafe { std::mem::zeroed() };
    // SAFETY: every pointer references a live initialized C-layout value, mutable
    // nul-terminated UTF-16 command line, or double-nul-terminated environment.
    // The only inherited handles are the three explicit standard stream handles.
    check(unsafe {
        CreateProcessW(
            image.as_ptr(),
            command_line.as_mut_ptr(),
            null_mut(),
            null_mut(),
            1,
            0x08080404,
            environment.as_mut_ptr().cast(),
            cwd.as_ptr(),
            &mut startup.startup,
            &mut process,
        )
    })?;
    let child = Handle::new(process.process)?;
    let thread = Handle::new(process.thread)?;
    if unsafe { AssignProcessToJobObject(job.0, child.0) } == 0 {
        unsafe {
            TerminateProcess(child.0, 1);
            WaitForSingleObject(child.0, 5000);
        }
        return Err(unavailable());
    }
    if unsafe { ResumeThread(thread.0) } == u32::MAX {
        return Err(unavailable());
    }
    drop(out_write);
    drop(err_write);
    drop(input);
    let stdout = out_read.into_file();
    let stderr = err_read.into_file();
    let out_reader = std::thread::spawn(move || crate::process::read_stream(stdout, true));
    let err_reader = std::thread::spawn(move || crate::process::read_stream(stderr, true));
    let deadline = Instant::now() + request.timeout;
    let (status, exit_code) = loop {
        let wait = unsafe { WaitForSingleObject(child.0, 10) };
        if wait == 0 {
            let mut code = 0;
            if unsafe { GetExitCodeProcess(child.0, &mut code) } == 0 {
                break (CommandStatus::LaunchFailed, None);
            }
            break (CommandStatus::Exited, Some(code as i32));
        }
        if wait != 258 {
            break (CommandStatus::LaunchFailed, None);
        }
        if Instant::now() >= deadline {
            break (CommandStatus::TimedOut, None);
        }
    };
    // Terminate descendants on ordinary leader exit too, before draining streams.
    check(unsafe { TerminateJobObject(job.0, 1) })?;
    unsafe {
        WaitForSingleObject(child.0, 5000);
    }
    drop(job);
    let (stdout, stdout_truncated) = out_reader.join().unwrap_or_default();
    let (stderr, stderr_truncated) = err_reader.join().unwrap_or_default();
    Ok(ProcessCapture {
        status,
        exit_code,
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    fn invoke(
        context: &RequirementWriterContext,
        argv: Vec<String>,
        timeout: u64,
        record: usize,
    ) -> (CommandOutcome, CommandIsolation) {
        let receipt = format!("outputs/work/e/TASK-001/ATTEMPT-001/receipts/CMD-{record:03}");
        let p = policy(context, &receipt).unwrap();
        let runner = IsolatedCommandRunner::new(context.clone());
        let preview = json!({"receipt_dir":receipt,"execution":{"work_isolation":p},
            "working_directory":context.canonical_project_root,"request":{"timeout_seconds":timeout},
            "invocation":{"kind":"direct","executable":argv[0],"argv":argv}});
        runner.bind(&preview).unwrap();
        (
            runner.run(
                &work_feature::execution::command_publication::build_command_request(&preview)
                    .unwrap(),
            ),
            p,
        )
    }
    #[test]
    fn native_lpac_allows_staging_and_denies_original_snapshot_external_writes_and_network() {
        let context = super::super::tests::context();
        std::fs::write(context.canonical_project_root.join("original"), b"before").unwrap();
        let image = std::env::var("COMSPEC").unwrap();
        for (n, script, success) in [
            (1, "echo candidate>\"%WORK_STAGING_DIR%\\after\"", true),
            (2, "echo changed>original", false),
            (3, "echo changed>\"%WORK_STAGING_DIR%\\..\\escape\"", false),
            (4, "cmd.exe /d /c \"echo changed>original\"", false),
        ] {
            let receipt = format!("outputs/work/e/TASK-001/ATTEMPT-001/receipts/CMD-{n:03}");
            let runner = IsolatedCommandRunner::new(context.clone());
            let p = policy(&context, &receipt).unwrap();
            let preview = json!({"receipt_dir":receipt,"execution":{"work_isolation":p},
                "working_directory":context.canonical_project_root,"request":{"timeout_seconds":3},
                "invocation":{"kind":"direct","executable":image,"argv":[image,"/d","/c",script]}});
            runner.bind(&preview).unwrap();
            let result = runner.run(
                &work_feature::execution::command_publication::build_command_request(&preview)
                    .unwrap(),
            );
            assert_eq!(result.status, CommandStatus::Exited, "{result:?}");
            assert_eq!(result.exit_code == Some(0), success, "{result:?}");
            assert_eq!(
                std::fs::read(context.canonical_project_root.join("original")).unwrap(),
                b"before"
            );
            assert!(
                !Path::new(&p.writable_directory)
                    .parent()
                    .unwrap()
                    .join("escape")
                    .exists()
            );
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let system = Path::new(&image).parent().unwrap();
        let (result, _) = invoke(
            &context,
            vec![
                system.join("curl.exe").to_string_lossy().into_owned(),
                "--noproxy".into(),
                "*".into(),
                "--max-time".into(),
                "1".into(),
                format!("http://{}/", listener.local_addr().unwrap()),
            ],
            3,
            20,
        );
        assert_eq!(result.status, CommandStatus::Exited, "{result:?}");
        assert_ne!(result.exit_code, Some(0), "{result:?}");
        assert!(matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
    }
    #[test]
    fn native_job_terminates_descendants_on_leader_exit_and_timeout() {
        let context = super::super::tests::context();
        std::fs::write(context.canonical_project_root.join("original"), b"before").unwrap();
        let executable = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (completed, p) = invoke(
            &context,
            vec![
                executable.clone(),
                "--exact".into(),
                "process::isolation::windows::tests::job_parent_worker".into(),
                "--nocapture".into(),
                "--test-threads=1".into(),
            ],
            3,
            1,
        );
        assert_eq!(completed.exit_code, Some(0), "{completed:?}");
        let stage = Path::new(&p.writable_directory);
        assert!(stage.join("child-started").exists());
        let start = Instant::now();
        let (timed, p) = invoke(
            &context,
            vec![
                executable,
                "--exact".into(),
                "process::isolation::windows::tests::job_timeout_worker".into(),
                "--nocapture".into(),
                "--test-threads=1".into(),
            ],
            1,
            2,
        );
        assert_eq!(timed.status, CommandStatus::TimedOut, "{timed:?}");
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(
            Path::new(&p.writable_directory)
                .join("timeout-started")
                .exists()
        );
        std::thread::sleep(Duration::from_secs(4));
        assert!(!stage.join("child-late").exists());
        assert!(
            !Path::new(&p.writable_directory)
                .join("timeout-late")
                .exists()
        );
        assert_eq!(
            std::fs::read(context.canonical_project_root.join("original")).unwrap(),
            b"before"
        );
    }
    #[test]
    fn job_parent_worker() {
        let Ok(stage) = std::env::var("WORK_STAGING_DIR") else {
            return;
        };
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::isolation::windows::tests::job_child_worker",
                "--nocapture",
                "--test-threads=1",
            ])
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !Path::new(&stage).join("child-started").exists() {
            assert!(Instant::now() < deadline);
            assert!(child.try_wait().unwrap().is_none());
            std::thread::sleep(Duration::from_millis(10));
        }
        // The leader exits without waiting; the owning host Job must stop this child.
        std::process::exit(0);
    }
    #[test]
    fn job_child_worker() {
        let Ok(stage) = std::env::var("WORK_STAGING_DIR") else {
            return;
        };
        assert!(std::fs::write("original", b"escape").is_err());
        std::fs::write(Path::new(&stage).join("child-started"), b"started").unwrap();
        std::thread::sleep(Duration::from_secs(3));
        std::fs::write(Path::new(&stage).join("child-late"), b"late").unwrap();
    }
    #[test]
    fn job_timeout_worker() {
        let Ok(stage) = std::env::var("WORK_STAGING_DIR") else {
            return;
        };
        std::fs::write(Path::new(&stage).join("timeout-started"), b"started").unwrap();
        std::thread::sleep(Duration::from_secs(3));
        std::fs::write(Path::new(&stage).join("timeout-late"), b"late").unwrap();
    }
    #[test]
    fn windows_argument_quoting_keeps_empty_spaces_quotes_and_trailing_backslashes() {
        assert_eq!(quote(""), "\"\"");
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote("a\\"), "a\\");
        assert_eq!(quote("a b\\"), "\"a b\\\\\"");
        assert_eq!(quote("/c"), "/c");
    }
}
