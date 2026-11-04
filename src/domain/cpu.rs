// Modified for rztop on 2026-11-02: monitoring, layout and interaction enhancements.
use std::collections::VecDeque;

/// Aggregate CPU usage histories by category.
#[derive(Debug, Clone, Default)]
pub struct CpuPercent {
    pub total: VecDeque<i64>,
    pub user: VecDeque<i64>,
    pub system: VecDeque<i64>,
    pub idle: VecDeque<i64>,
}

/// CPU usage data for all cores and aggregate statistics.
#[derive(Debug, Clone)]
pub struct CpuInfo {
    pub cpu_percent: CpuPercent,
    /// Per-core usage history (index = core number).
    pub core_percent: Vec<VecDeque<i64>>,
    /// Emulated load average (1, 5, 15 minute rolling EMA).
    pub load_avg: [f64; 3],
    /// CPU model name (e.g. "Intel Core i9-13900K").
    pub cpu_name: String,
    /// Current CPU frequency as display string (e.g. "5.80 GHz").
    pub cpu_hz: String,
    /// Current estimated MHz (registry base frequency if PDH is unavailable).
    pub clock_mhz: u32,
    /// Largest valid reading this session, not a hardware boost limit.
    pub peak_clock_mhz: u32,
    pub core_count: usize,
    /// Temperature history: index 0 = package, 1+ = per-core.
    pub temp: Vec<VecDeque<i64>>,
    /// CPU package power in watts (from LHM), if available.
    pub cpu_watts: Option<f64>,
    /// Maximum observed CPU package power in watts (from LHM), if available.
    pub cpu_max_watts: Option<f64>,
}

impl Default for CpuInfo {
    fn default() -> Self {
        Self {
            cpu_percent: CpuPercent::default(),
            core_percent: Vec::new(),
            load_avg: [0.0; 3],
            cpu_name: String::new(),
            cpu_hz: String::new(),
            clock_mhz: 0,
            peak_clock_mhz: 0,
            core_count: 0,
            temp: Vec::new(),
            cpu_watts: None,
            cpu_max_watts: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cpu_info_has_empty_percent_fields() {
        let info = CpuInfo::default();
        assert!(info.cpu_percent.total.is_empty());
        assert!(info.cpu_percent.user.is_empty());
        assert!(info.cpu_percent.system.is_empty());
        assert!(info.cpu_percent.idle.is_empty());
    }

    #[test]
    fn default_cpu_info_has_zero_cores() {
        let info = CpuInfo::default();
        assert_eq!(info.core_count, 0);
        assert!(info.core_percent.is_empty());
    }
}
