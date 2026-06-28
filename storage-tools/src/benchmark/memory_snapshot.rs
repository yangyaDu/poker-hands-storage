use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshot {
    pub rss_bytes: Option<u64>,
    pub heap_total_bytes: Option<u64>,
    pub heap_used_bytes: Option<u64>,
    pub external_bytes: Option<u64>,
    pub array_buffers_bytes: Option<u64>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkMemoryReport {
    pub before: MemorySnapshot,
    pub after: MemorySnapshot,
    pub delta_rss_bytes: Option<i64>,
    pub delta_heap_used_bytes: Option<i64>,
    pub notes: Vec<String>,
}

impl BenchmarkMemoryReport {
    pub fn new(before: MemorySnapshot, after: MemorySnapshot) -> Self {
        let mut notes = Vec::new();
        if let Some(note) = before.note.as_ref().or(after.note.as_ref()) {
            notes.push(note.clone());
        }

        Self {
            delta_rss_bytes: delta_bytes(before.rss_bytes, after.rss_bytes),
            delta_heap_used_bytes: delta_bytes(before.heap_used_bytes, after.heap_used_bytes),
            before,
            after,
            notes,
        }
    }
}

pub fn get_memory_snapshot() -> MemorySnapshot {
    platform_memory_snapshot()
}

fn delta_bytes(before: Option<u64>, after: Option<u64>) -> Option<i64> {
    let before = before?;
    let after = after?;
    Some(after as i64 - before as i64)
}

#[cfg(windows)]
fn platform_memory_snapshot() -> MemorySnapshot {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};

    #[repr(C)]
    #[allow(non_snake_case)]
    struct ProcessMemoryCounters {
        cb: u32,
        PageFaultCount: u32,
        PeakWorkingSetSize: usize,
        WorkingSetSize: usize,
        QuotaPeakPagedPoolUsage: usize,
        QuotaPagedPoolUsage: usize,
        QuotaPeakNonPagedPoolUsage: usize,
        QuotaNonPagedPoolUsage: usize,
        PagefileUsage: usize,
        PeakPagefileUsage: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
    }

    #[link(name = "psapi")]
    extern "system" {
        fn GetProcessMemoryInfo(
            process: *mut c_void,
            counters: *mut ProcessMemoryCounters,
            size: u32,
        ) -> i32;
    }

    let mut counters: ProcessMemoryCounters = unsafe { zeroed() };
    counters.cb = size_of::<ProcessMemoryCounters>() as u32;
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            size_of::<ProcessMemoryCounters>() as u32,
        )
    };

    if ok == 0 {
        return unsupported_snapshot("Windows GetProcessMemoryInfo failed");
    }

    MemorySnapshot {
        rss_bytes: Some(counters.WorkingSetSize as u64),
        heap_total_bytes: Some(counters.PeakPagefileUsage as u64),
        heap_used_bytes: Some(counters.PagefileUsage as u64),
        external_bytes: None,
        array_buffers_bytes: None,
        note: Some(
            "Windows memory uses process working set for RSS and pagefile usage as heap approximation."
                .to_owned(),
        ),
    }
}

#[cfg(target_os = "linux")]
fn platform_memory_snapshot() -> MemorySnapshot {
    let status = match std::fs::read_to_string("/proc/self/status") {
        Ok(status) => status,
        Err(error) => {
            return unsupported_snapshot(format!("Could not read /proc/self/status: {error}"));
        }
    };

    let rss_bytes = status.lines().find_map(|line| {
        let rest = line.strip_prefix("VmRSS:")?.trim();
        let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
        Some(kb * 1024)
    });

    MemorySnapshot {
        rss_bytes,
        heap_total_bytes: None,
        heap_used_bytes: None,
        external_bytes: None,
        array_buffers_bytes: None,
        note: Some(
            "Linux memory uses /proc/self/status VmRSS; heap approximation is unavailable."
                .to_owned(),
        ),
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_memory_snapshot() -> MemorySnapshot {
    unsupported_snapshot("Memory snapshot is unsupported on this platform")
}

fn unsupported_snapshot(message: impl Into<String>) -> MemorySnapshot {
    MemorySnapshot {
        rss_bytes: None,
        heap_total_bytes: None,
        heap_used_bytes: None,
        external_bytes: None,
        array_buffers_bytes: None,
        note: Some(message.into()),
    }
}
