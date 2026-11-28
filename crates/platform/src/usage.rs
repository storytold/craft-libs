//! The process's memory use, for a status readout ("Scratch: 1.2G/8.0G") and the Efficiency item.

/// One sample of memory use, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemorySample {
    /// The process's resident memory.
    pub used: u64,
    /// Physical memory the process may still use: the system's available memory plus `used`.
    pub available: u64,
}

/// Samples memory use now; `None` where the platform gives no reading (the web) or the reading
/// fails. Linux reads `/proc`; macOS and Windows ask `sysinfo`. Cheap enough to call once a
/// second, not every frame.
// new: PhotoCraft computes document sizes only (panels/status_bar.rs:69) and samples no process
// memory. Linux reads `VmRSS` of `/proc/self/status` and `MemAvailable` of `/proc/meminfo`, both
// in kibibytes, which needs no page size. `sysinfo` reads the available memory exactly.
pub fn sample() -> Option<MemorySample> {
    #[cfg(target_os = "linux")]
    {
        let used = kibibytes(&std::fs::read_to_string("/proc/self/status").ok()?, "VmRSS:")?;
        let free = kibibytes(&std::fs::read_to_string("/proc/meminfo").ok()?, "MemAvailable:")?;
        Some(MemorySample { used, available: free.saturating_add(used) })
    }
    #[cfg(any(target_os = "macos", windows))]
    {
        use sysinfo::{MemoryRefreshKind, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
        let pid = sysinfo::get_current_pid().ok()?;
        let mut system = System::new_with_specifics(RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()));
        system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing().with_memory());
        let used = system.process(pid)?.memory();
        Some(MemorySample { used, available: system.available_memory().saturating_add(used) })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        None
    }
}

/// The bytes of the `key` line of a `/proc` file ("VmRSS:   1234 kB").
#[cfg(target_os = "linux")]
fn kibibytes(text: &str, key: &str) -> Option<u64> {
    let rest = text.lines().find_map(|line| line.strip_prefix(key))?;
    let mut parts = rest.split_whitespace();
    let n: u64 = parts.next()?.parse().ok()?;
    match parts.next() {
        Some("kB") => n.checked_mul(1024),
        _ => None,
    }
}
