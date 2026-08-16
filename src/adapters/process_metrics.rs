//! Process-local observability used by explicit performance experiments.

#[cfg(windows)]
pub(crate) fn working_set_bytes() -> Option<u64> {
    use windows_sys::Win32::{
        System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        System::Threading::GetCurrentProcess,
    };
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?,
        ..PROCESS_MEMORY_COUNTERS::default()
    };
    // SAFETY: the pseudo-handle is valid in the current process and `counters`
    // is a correctly sized writable structure for the duration of the call.
    let ok = unsafe {
        GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut counters).cast(), counters.cb)
    };
    (ok != 0).then_some(counters.WorkingSetSize as u64)
}

#[cfg(not(windows))]
pub(crate) fn working_set_bytes() -> Option<u64> {
    None
}
