//! Current host memory capacity, independent of project paths and task declarations.
use work_feature::error::WorkError;
use work_feature::execution::file_transaction::error;

#[cfg(windows)]
pub fn available() -> Result<u64, WorkError> {
    #[repr(C)]
    struct Status {
        length: u32,
        load: u32,
        values: [u64; 7],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(status: *mut Status) -> i32;
    }
    let mut status = Status {
        length: std::mem::size_of::<Status>() as u32,
        load: 0,
        values: [0; 7],
    };
    // SAFETY: MEMORYSTATUSEX is two DWORDs followed by seven DWORDLONGs;
    // the buffer is initialized, correctly sized and lives through the call.
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return Err(error("file_transaction_memory_unverifiable"));
    }
    // Available physical memory, commit capacity and virtual address space.
    Ok(status.values[1].min(status.values[3]).min(status.values[5]))
}

#[cfg(target_os = "linux")]
pub fn available() -> Result<u64, WorkError> {
    let info = std::fs::read_to_string("/proc/meminfo")
        .map_err(|_| error("file_transaction_memory_unverifiable"))?;
    let line = info
        .lines()
        .find_map(|line| line.strip_prefix("MemAvailable:"))
        .ok_or_else(|| error("file_transaction_memory_unverifiable"))?;
    let mut fields = line.split_whitespace();
    let bytes = fields
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .and_then(|v| v.checked_mul(1024))
        .filter(|_| fields.next() == Some("kB"))
        .ok_or_else(|| error("file_transaction_memory_unverifiable"))?;
    Ok(bytes)
}

#[cfg(target_os = "macos")]
pub fn available() -> Result<u64, WorkError> {
    unsafe extern "C" {
        fn mach_host_self() -> u32;
        static mach_task_self_: u32;
        // XNU vm_size_t is uintptr_t, unlike the 32-bit natural_t statistics.
        fn host_page_size(host: u32, size: *mut usize) -> i32;
        fn host_statistics(host: u32, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
        fn mach_port_deallocate(task: u32, name: u32) -> i32;
    }
    let mut statistics = [0_u32; 15];
    let mut count = statistics.len() as u32;
    let mut page_size = 0_usize;
    // SAFETY: HOST_VM_INFO (2) writes vm_statistics_data_t, fifteen natural_t
    // fields as declared by XNU. Every out buffer is live and sized by count.
    // Release the host send right on both success and failure.
    let (pages_result, statistics_result) = unsafe {
        let host = mach_host_self();
        let pages = host_page_size(host, &mut page_size);
        let stats = host_statistics(host, 2, statistics.as_mut_ptr().cast(), &mut count);
        let _ = mach_port_deallocate(mach_task_self_, host);
        (pages, stats)
    };
    if pages_result != 0 || statistics_result != 0 || page_size == 0 || count < 12 {
        return Err(error("file_transaction_memory_unverifiable"));
    }
    // Conservative immediately free pages; do not promise inactive pages can
    // be reclaimed. XNU free_count already includes speculative pages.
    u64::from(statistics[0])
        .checked_mul(page_size as u64)
        .ok_or_else(|| error("file_transaction_memory_unverifiable"))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn available() -> Result<u64, WorkError> {
    Err(error("file_transaction_memory_unverifiable"))
}
