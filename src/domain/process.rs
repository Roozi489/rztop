// Modified for rztop on 2026-11-03: permission status, I/O ranking and pagefile clarity.
// Modified for rztop on 2026-11-02: monitoring, layout and interaction enhancements.
use std::fmt;

/// Persisted descending sort for the general I/O ranking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IoSort {
    #[default]
    Total,
    Read,
    Write,
}

impl IoSort {
    pub fn value(self, rates: IoRates) -> u64 {
        match self {
            Self::Total => rates.total(),
            Self::Read => rates.read,
            Self::Write => rates.write,
        }
    }
}

/// Information about a single process.
#[derive(Debug, Clone)]
pub struct ProcInfo {
    /// Process ID.
    pub pid: u32,
    /// Process name (e.g. "chrome.exe").
    pub name: String,
    /// Full command line.
    pub cmd: String,
    /// Thread count.
    pub threads: usize,
    /// Username of the process owner.
    pub user: String,
    /// Private committed memory in bytes (Windows Commit size).
    pub mem: u64,
    /// Resident physical memory (working set), in bytes.
    pub ram: u64,
    pub gpu_p: Option<f64>,
    pub gpu_mem: Option<u64>,
    /// Current CPU usage percentage (may exceed 100% if per-core).
    pub cpu_p: f64,
    /// Current process state.
    pub state: ProcState,
    /// Process priority class.
    pub priority: PriorityClass,
    /// Parent process ID.
    pub ppid: u32,
    /// Total CPU time consumed (100ns intervals).
    pub cpu_time: u64,
    /// Total IO bytes read.
    pub io_read: u64,
    /// Total IO bytes written.
    pub io_write: u64,
    /// General I/O bytes/second, requiring two valid samples of the same process.
    pub io_rates: Option<IoRates>,
}

impl Default for ProcInfo {
    fn default() -> Self {
        Self {
            pid: 0,
            name: String::new(),
            cmd: String::new(),
            threads: 0,
            user: String::new(),
            mem: 0,
            ram: 0,
            gpu_p: None,
            gpu_mem: None,
            cpu_p: 0.0,
            state: ProcState::Unknown,
            priority: PriorityClass::Normal,
            ppid: 0,
            cpu_time: 0,
            io_read: 0,
            io_write: 0,
            io_rates: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IoRates {
    pub read: u64,
    pub write: u64,
}

impl IoRates {
    pub fn total(self) -> u64 {
        self.read.saturating_add(self.write)
    }
}

/// Display-only metadata for one visible process row.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcDisplayEntry {
    /// Index into the collector's raw `procs` vector.
    pub proc_index: usize,
    /// Tree view prefix string (e.g. "├─ ", "│  └─ ").
    pub prefix: String,
    /// Depth in process tree (0 = root).
    pub depth: usize,
    /// Aggregated CPU% override (used when `proc_aggregate` is on in tree mode).
    pub cpu_override: Option<f64>,
    /// Aggregated memory override (used when `proc_aggregate` is on in tree mode).
    pub mem_override: Option<u64>,
}

impl ProcDisplayEntry {
    pub fn flat(proc_index: usize) -> Self {
        Self {
            proc_index,
            prefix: String::new(),
            depth: 0,
            cpu_override: None,
            mem_override: None,
        }
    }

    pub fn tree(proc_index: usize, prefix: String, depth: usize) -> Self {
        Self {
            proc_index,
            prefix,
            depth,
            cpu_override: None,
            mem_override: None,
        }
    }
}

/// Process state on Windows.
///
/// Unlike Linux (R/S/D/Z/T/t/X/K/W/P), Windows has fewer observable states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcState {
    Running,
    /// Every thread is parked in a suspended wait. Modern Windows
    /// suspends UWP and Store apps when they lose focus, and a
    /// debugger freeze looks the same, so this distinguishes "idle by
    /// choice" from "stopped by the system" for a process sitting at
    /// 0% CPU.
    Suspended,
    Unknown,
}

impl fmt::Display for ProcState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Running => write!(f, "Running"),
            Self::Suspended => write!(f, "Suspended"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Windows process priority class.
///
/// Replaces Unix nice values (-20..19) with 6 discrete classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PriorityClass {
    Idle = 0,
    BelowNormal = 1,
    Normal = 2,
    AboveNormal = 3,
    High = 4,
    Realtime = 5,
}

impl fmt::Display for PriorityClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => write!(f, "Idle"),
            Self::BelowNormal => write!(f, "Below Normal"),
            Self::Normal => write!(f, "Normal"),
            Self::AboveNormal => write!(f, "Above Normal"),
            Self::High => write!(f, "High"),
            Self::Realtime => write!(f, "Realtime"),
        }
    }
}

/// Display process CPU using either one-core or total-machine capacity.
pub fn display_cpu_percent(cpu_per_core: f64, core_count: usize, per_core: bool) -> f64 {
    if !cpu_per_core.is_finite() {
        return 0.0;
    }
    let core_count = core_count.max(1);
    let value = if per_core {
        cpu_per_core
    } else {
        cpu_per_core / core_count as f64
    };
    let max_value = if per_core {
        100.0 * core_count as f64
    } else {
        100.0
    };
    value.clamp(0.0, max_value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_state_display_names() {
        assert_eq!(ProcState::Running.to_string(), "Running");
        assert_eq!(ProcState::Unknown.to_string(), "Unknown");
    }

    #[test]
    fn priority_class_ordering() {
        assert!(PriorityClass::Idle < PriorityClass::BelowNormal);
        assert!(PriorityClass::BelowNormal < PriorityClass::Normal);
        assert!(PriorityClass::Normal < PriorityClass::AboveNormal);
        assert!(PriorityClass::AboveNormal < PriorityClass::High);
        assert!(PriorityClass::High < PriorityClass::Realtime);
    }

    #[test]
    fn priority_class_display() {
        assert_eq!(PriorityClass::Normal.to_string(), "Normal");
        assert_eq!(PriorityClass::Realtime.to_string(), "Realtime");
        assert_eq!(PriorityClass::Idle.to_string(), "Idle");
    }
}
