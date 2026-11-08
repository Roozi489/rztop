// Modified for rztop on 2026-10-23: adapted for the rztop desktop app.
use std::collections::VecDeque;

/// Memory statistics in bytes.
#[derive(Debug, Clone, Default)]
pub struct MemStats {
    pub used: u64,
    pub available: u64,
    /// RAM holding compressed pages: the working set of Windows' Memory
    /// Compression process. `None` when that process is not running.
    pub compressed: Option<u64>,
    pub commit_used: u64,
    pub commit_limit: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub swap_free: u64,
}

/// Percentage histories for each memory category (0-100).
#[derive(Debug, Clone, Default)]
pub struct MemPercent {
    pub used: VecDeque<i64>,
    pub commit: VecDeque<i64>,
    pub swap_used: VecDeque<i64>,
    pub swap_free: VecDeque<i64>,
}

/// System memory and swap usage information.
#[derive(Debug, Clone, Default)]
pub struct MemInfo {
    pub stats: MemStats,
    pub percent: MemPercent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_info_default_has_zero_stats() {
        let mem = MemInfo::default();
        assert_eq!(mem.stats.used, 0);
        assert_eq!(mem.stats.available, 0);
        assert_eq!(mem.stats.compressed, None);
        assert_eq!(mem.stats.commit_used, 0);
        assert_eq!(mem.stats.commit_limit, 0);
        assert_eq!(mem.stats.swap_total, 0);
        assert_eq!(mem.stats.swap_used, 0);
        assert_eq!(mem.stats.swap_free, 0);
    }

    #[test]
    fn mem_info_default_has_empty_percent() {
        let mem = MemInfo::default();
        assert!(mem.percent.used.is_empty());
        assert!(mem.percent.commit.is_empty());
        assert!(mem.percent.swap_used.is_empty());
        assert!(mem.percent.swap_free.is_empty());
    }
}
