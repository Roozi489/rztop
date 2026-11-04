use std::collections::VecDeque;

/// GPU usage percentage histories.
#[derive(Debug, Clone, Default)]
pub struct GpuPercent {
    pub utilization: VecDeque<i64>,
    pub vram: VecDeque<i64>,
    pub power: VecDeque<i64>,
}

/// GPU monitoring data for a single GPU device.
#[derive(Debug, Clone)]
pub struct GpuInfo {
    /// Vendor-prefixed stable identifier for this device. Format
    /// is `"<VENDOR>:<UUID-or-fallback>"`:
    ///
    /// * NVIDIA: `"NVIDIA:GPU-12345678-1234-..."` — sourced from
    ///   `nvmlDeviceGetUUID`. Fallback `"NVIDIA:adapter:N"` (with
    ///   vendor-relative index) when NVML is unavailable or the
    ///   UUID call fails.
    /// * AMD: `"AMD:<UDID>"` — sourced from `AdapterInfoX4::strUDID`.
    ///   Fallback `"AMD:adapter:N"` if the UDID is empty.
    /// * Intel: `"INTEL:adapter:N"` — IGCL does not expose a
    ///   per-card UUID at this struct level, so the
    ///   vendor-relative discovery index is the only stable id
    ///   available today.
    ///
    /// Meant for matching the persisted `view.gpu_iface` against the
    /// live device list.
    /// Bound at discovery time; immutable for the device's
    /// lifetime.
    #[expect(dead_code, reason = "the GUI does not persist the selected GPU yet")]
    pub stable_id: String,
    /// GPU device name (e.g. "NVIDIA RTX 4090").
    pub name: String,
    /// Usage histories.
    pub gpu_percent: GpuPercent,
    /// GPU core clock speed in MHz.
    pub gpu_clock_speed: u32,
    /// Maximum GPU core clock speed in MHz.
    pub gpu_max_clock_speed: u32,
    /// Current power draw in milliwatts.
    pub pwr_usage: i64,
    /// Maximum power limit in milliwatts.
    pub pwr_max_usage: i64,
    /// Temperature history in °C.
    pub temp: VecDeque<i64>,
    /// Total VRAM in bytes.
    pub mem_total: u64,
    /// Used VRAM in bytes.
    pub mem_used: u64,
    /// VRAM utilization percentage history.
    pub mem_utilization_percent: VecDeque<i64>,
}

impl Default for GpuInfo {
    fn default() -> Self {
        Self {
            stable_id: String::new(),
            name: String::new(),
            gpu_percent: GpuPercent::default(),
            gpu_clock_speed: 0,
            gpu_max_clock_speed: 0,
            pwr_usage: 0,
            pwr_max_usage: 255_000,
            temp: VecDeque::from([0]),
            mem_total: 0,
            mem_used: 0,
            mem_utilization_percent: VecDeque::from([0]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_info_default_has_empty_percent() {
        let gpu = GpuInfo::default();
        assert!(gpu.gpu_percent.utilization.is_empty());
        assert!(gpu.gpu_percent.vram.is_empty());
        assert!(gpu.gpu_percent.power.is_empty());
    }
}
