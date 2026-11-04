// Modified for rztop on 2026-11-04: kept only the snapshot types and LatestSlot; the collector threads now live in the desktop frontend.
use crate::collect::CollectStatus;
use crate::domain::{
    cpu::CpuInfo, disk::DiskData, gpu::GpuInfo, memory::MemInfo, network::NetInfo,
    process::ProcInfo,
};
use arc_swap::ArcSwapOption;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(crate) struct CpuSnapshot {
    pub(crate) info: CpuInfo,
    #[allow(dead_code, reason = "collector health is not shown in the GUI yet")]
    pub(crate) status: CollectStatus,
}

#[derive(Debug, Clone)]
pub(crate) struct DiskSnapshot {
    pub(crate) info: DiskData,
    #[allow(dead_code, reason = "collector health is not shown in the GUI yet")]
    pub(crate) status: CollectStatus,
}

/// GPU snapshot containing every detected device's data, published
/// once per cycle. Mirrors
/// [`NetSnapshot`] — one snapshot, one Vec of devices, one status.
#[derive(Debug, Clone)]
pub(crate) struct GpuSnapshot {
    pub(crate) devices: Vec<GpuInfo>,
    #[allow(dead_code, reason = "collector health is not shown in the GUI yet")]
    pub(crate) status: CollectStatus,
}

#[derive(Debug, Clone)]
pub(crate) struct MemSnapshot {
    pub(crate) info: MemInfo,
    #[allow(dead_code, reason = "collector health is not shown in the GUI yet")]
    pub(crate) status: CollectStatus,
}

#[derive(Debug, Clone)]
pub(crate) struct NetSnapshot {
    pub(crate) nets: Vec<NetInfo>,
    #[allow(dead_code, reason = "collector health is not shown in the GUI yet")]
    pub(crate) status: CollectStatus,
}

#[derive(Debug, Clone)]
pub(crate) struct ProcSnapshot {
    pub(crate) procs: Vec<ProcInfo>,
    #[allow(dead_code, reason = "collector health is not shown in the GUI yet")]
    pub(crate) status: CollectStatus,
}

#[derive(Debug, Clone)]
pub(crate) struct StatusbarSnapshot {
    pub(crate) info: crate::collect::statusbar::StatusbarInfo,
}

/// Thread-safe slot that always holds the latest value.
///
/// Publishers overwrite; consumers read the latest. Multiple publishes
/// between reads naturally coalesce — only the most recent value is
/// kept. Both operations are lock-free atomic swaps via
/// [`arc_swap::ArcSwapOption`].
pub(crate) struct LatestSlot<T> {
    inner: Arc<ArcSwapOption<T>>,
}

impl<T> Clone for LatestSlot<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> LatestSlot<T> {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(ArcSwapOption::empty()),
        }
    }

    /// Store new data, replacing any previous value. Lock-free.
    pub(crate) fn publish(&self, data: T) {
        self.inner.store(Some(Arc::new(data)));
    }

    /// Read the latest value, if any. Lock-free.
    pub(crate) fn latest(&self) -> Option<Arc<T>> {
        self.inner.load_full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_slot_publishes_and_reads() {
        let slot = LatestSlot::new();
        assert!(slot.latest().is_none());

        slot.publish(42_u32);
        let val = slot.latest().expect("should have value");
        assert_eq!(*val, 42);
    }

    #[test]
    fn latest_slot_overwrites_previous() {
        let slot = LatestSlot::new();
        slot.publish(1_u32);
        slot.publish(2_u32);
        assert_eq!(*slot.latest().unwrap(), 2);
    }

    #[test]
    fn latest_slot_clone_shares_state() {
        let slot = LatestSlot::new();
        let clone = slot.clone();
        slot.publish(99_u32);
        assert_eq!(*clone.latest().unwrap(), 99);
    }

    #[test]
    fn cpu_snapshot_clones_data() {
        let snap = CpuSnapshot {
            info: CpuInfo::default(),
            status: CollectStatus::Ok,
        };
        assert_eq!(snap.info.core_count, 0);
        assert_eq!(snap.status, CollectStatus::Ok);
    }
}
