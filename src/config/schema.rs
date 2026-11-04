// Modified for rztop on 2026-11-04: kept only the integer keys and their bounds; the options-menu key schema went with the terminal frontend.
//! Integer config keys and their bounds.
//!
//! `int_keys!` generates [`IntKey`] from one table connecting each
//! variant to its TOML name and `Config` field. [`Config::validate`]
//! clamps every key into [`IntKey::bounds`] after loading a file.

use super::Config;

macro_rules! int_keys {
    ( $( $var:ident => $name:literal => $section:ident . $field:ident ),* $(,)? ) => {
        /// Integer-typed config keys.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum IntKey { $( $var ),* }

        impl IntKey {
            /// Every variant in declaration order.
            pub const ALL: &'static [IntKey] = &[ $( Self::$var ),* ];

            /// TOML field name (snake_case).
            pub fn name(self) -> &'static str {
                match self { $( Self::$var => $name, )* }
            }

            /// Read the current int value.
            pub fn get(self, config: &Config) -> i64 {
                match self { $( Self::$var => config.$section.$field, )* }
            }

            /// Write the int value. No clamping — caller should
            /// invoke [`Config::validate`] to apply bounds.
            pub fn set(self, config: &mut Config, value: i64) {
                match self { $( Self::$var => config.$section.$field = value, )* }
            }
        }
    };
}

int_keys! {
    UpdateMs => "update_ms" => refresh.update_ms,
    CpuUpdateMs => "cpu_update_ms" => refresh.cpu_update_ms,
    MemUpdateMs => "mem_update_ms" => refresh.mem_update_ms,
    DiskUpdateMs => "disk_update_ms" => refresh.disk_update_ms,
    NetUpdateMs => "net_update_ms" => refresh.net_update_ms,
    GpuUpdateMs => "gpu_update_ms" => refresh.gpu_update_ms,
    SwapMaxMib => "swap_max_mib" => mem.swap_max_mib,
    GpuPowerMaxWatts => "gpu_power_max_watts" => gpu.gpu_power_max_watts,
    GpuClockMaxMhz => "gpu_clock_max_mhz" => gpu.gpu_clock_max_mhz,
    CpuClockMaxMhz => "cpu_clock_max_mhz" => cpu.cpu_clock_max_mhz,
    ProcUpdateMs => "proc_update_ms" => refresh.proc_update_ms,
    NetDownload => "net_download" => net.net_download,
    NetUpload => "net_upload" => net.net_upload,
}

impl IntKey {
    /// Inclusive `[min, max]` bounds for this key.
    pub fn bounds(self) -> std::ops::RangeInclusive<i64> {
        match self {
            Self::UpdateMs => 100..=86_400_000,
            Self::CpuUpdateMs
            | Self::MemUpdateMs
            | Self::DiskUpdateMs
            | Self::NetUpdateMs
            | Self::GpuUpdateMs
            | Self::ProcUpdateMs => 0..=86_400_000,
            Self::NetDownload | Self::NetUpload => 0..=10_000_000,
            Self::GpuPowerMaxWatts => 0..=10_000,
            Self::GpuClockMaxMhz | Self::CpuClockMaxMhz => 0..=10_000,
            Self::SwapMaxMib => 0..=1_000_000_000,
        }
    }

    /// Clamp `value` into [`Self::bounds`], returning the in-range
    /// value. Used by [`Config::validate`] after loading a TOML file
    /// so out-of-range values land at the nearest valid edge instead
    /// of failing the whole load.
    pub fn clamp_to_bounds(self, value: i64) -> i64 {
        let bounds = self.bounds();
        value.clamp(*bounds.start(), *bounds.end())
    }
}
