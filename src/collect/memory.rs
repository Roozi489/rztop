// Modified for rztop on 2026-11-08: compressed memory replaces the system cache.
// Modified for rztop on 2026-11-03: permission status, I/O ranking and pagefile clarity.
use crate::domain::memory::MemInfo;
use std::collections::VecDeque;

use super::{Collector, counters::percent_u64};

const MAX_HISTORY: usize = 300;

/// Memory data collector using Windows APIs.
pub struct MemCollector {
    pub info: MemInfo,
    pub status: super::CollectStatus,
}

impl MemCollector {
    pub fn new() -> Self {
        Self {
            info: MemInfo::default(),
            status: super::CollectStatus::Ok,
        }
    }

    fn collect_impl(&mut self) {
        self.status = super::CollectStatus::Ok;
        self.collect_memory();
    }

    fn collect_memory(&mut self) {
        use windows::Win32::System::SystemInformation::*;

        let mut mem_status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };

        // SAFETY: mem_status is a properly initialized MEMORYSTATUSEX with
        // dwLength set to the struct size. GlobalMemoryStatusEx writes to
        // this valid, properly-aligned struct and the return value is checked.
        unsafe {
            if GlobalMemoryStatusEx(&mut mem_status).is_ok() {
                let total = mem_status.ullTotalPhys;
                let available = mem_status.ullAvailPhys;
                let used = total.saturating_sub(available);

                self.info.stats.used = used;
                self.info.stats.available = available;

                if total > 0 {
                    push_pct(&mut self.info.percent.used, used, total);
                }
            } else {
                tracing::warn!(
                    subsystem = %crate::log::Subsystem::Memory,
                    "GlobalMemoryStatusEx failed",
                );
                self.status
                    .downgrade(super::CollectStatus::Failed("memory query failed"));
            }
        }

        // GetPerformanceInfo reports commit in pages.
        use windows::Win32::System::ProcessStatus::*;

        let mut perf = PERFORMANCE_INFORMATION {
            cb: std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
            ..Default::default()
        };
        // SAFETY: perf is a properly initialized PERFORMANCE_INFORMATION with
        // cb set to the struct size. GetPerformanceInfo writes to this valid,
        // properly-aligned struct and the return value is checked.
        unsafe {
            if GetPerformanceInfo(&mut perf, perf.cb).is_ok() {
                self.info.stats.commit_used =
                    (perf.CommitTotal as u64).saturating_mul(perf.PageSize as u64);
                self.info.stats.commit_limit =
                    (perf.CommitLimit as u64).saturating_mul(perf.PageSize as u64);
                push_pct(
                    &mut self.info.percent.commit,
                    self.info.stats.commit_used,
                    self.info.stats.commit_limit,
                );
                let mut pages = PageFilePages::default();
                // EnumPageFilesW invokes the callback synchronously with this live context.
                if EnumPageFilesW(
                    Some(sum_page_file),
                    (&mut pages as *mut PageFilePages).cast(),
                )
                .inspect_err(super::permissions::note_error)
                .is_ok()
                {
                    let (total, used, free) = pages.bytes(perf.PageSize as u64);
                    self.info.stats.swap_total = total;
                    self.info.stats.swap_used = used;
                    self.info.stats.swap_free = free;
                    push_pct(&mut self.info.percent.swap_used, used, total);
                    push_pct(&mut self.info.percent.swap_free, free, total);
                } else {
                    self.status
                        .downgrade(super::CollectStatus::Degraded("page-file query failed"));
                }
            } else {
                self.status.downgrade(super::CollectStatus::Degraded(
                    "memory performance query failed",
                ));
            }
        }
    }
}

impl Collector for MemCollector {
    type Snapshot = crate::runner::MemSnapshot;

    fn collect(&mut self) {
        self.collect_impl();
    }

    fn snapshot(&self) -> Self::Snapshot {
        crate::runner::MemSnapshot {
            info: self.info.clone(),
            status: self.status.clone(),
        }
    }
}

fn push_pct(deque: &mut VecDeque<i64>, value: u64, total: u64) {
    let pct = percent_u64(value, total).min(100);
    deque.push_back(pct);
    while deque.len() > MAX_HISTORY {
        deque.pop_front();
    }
}

#[cfg(test)]
pub fn calculate_used(total: u64, available: u64) -> u64 {
    total.saturating_sub(available)
}

#[derive(Default)]
struct PageFilePages {
    total: u64,
    used: u64,
}

impl PageFilePages {
    fn bytes(&self, page_size: u64) -> (u64, u64, u64) {
        let total = self.total.saturating_mul(page_size);
        let used = self.used.saturating_mul(page_size).min(total);
        (total, used, total.saturating_sub(used))
    }
}

unsafe extern "system" fn sum_page_file(
    context: *mut std::ffi::c_void,
    info: *mut windows::Win32::System::ProcessStatus::ENUM_PAGE_FILE_INFORMATION,
    _: windows::core::PCWSTR,
) -> windows::core::BOOL {
    // SAFETY: EnumPageFilesW supplies a valid record; caller provides a live,
    // exclusively borrowed PageFilePages for the synchronous enumeration.
    let (pages, info) = unsafe { (&mut *context.cast::<PageFilePages>(), &*info) };
    pages.total = pages.total.saturating_add(info.TotalSize as u64);
    pages.used = pages.used.saturating_add(info.TotalInUse as u64);
    true.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculate_used_from_total_available() {
        assert_eq!(calculate_used(16_000_000_000, 8_000_000_000), 8_000_000_000);
        assert_eq!(calculate_used(100, 150), 0); // Saturating
    }

    #[test]
    fn calculate_swap_from_pagefile() {
        assert_eq!(
            PageFilePages { total: 8, used: 2 }.bytes(4096),
            (32768, 8192, 24576)
        );
        assert_eq!(PageFilePages::default().bytes(4096), (0, 0, 0));
        assert_eq!(
            PageFilePages { total: 1, used: 2 }.bytes(4096),
            (4096, 4096, 0)
        );
        assert_eq!(
            PageFilePages {
                total: u64::MAX,
                used: u64::MAX
            }
            .bytes(4096),
            (u64::MAX, u64::MAX, 0)
        );
    }

    #[test]
    fn collect_returns_valid_mem_info() {
        let mut collector = MemCollector::new();
        collector.collect();
        assert!(collector.info.stats.used > 0);
    }
}
