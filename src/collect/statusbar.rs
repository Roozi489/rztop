//! Collects uptime and ping at the global app refresh interval.

use super::Collector;

/// Statusbar snapshot data, cached at 1 Hz.
#[derive(Debug, Clone, Default)]
pub struct StatusbarInfo {
    /// System uptime in seconds since boot.
    pub uptime_seconds: u64,
    pub ping: String,
    pub ping_history: std::collections::VecDeque<Option<i64>>,
}

/// Statusbar collector. Stateless apart from the most recent snapshot.
///
/// `GetTickCount64` is infallible — there is no degraded mode to
/// surface — so the collector and snapshot carry no `CollectStatus`.
pub struct StatusbarCollector {
    pub info: StatusbarInfo,
    probe: fn(&str) -> String,
    ping_target: String,
}

impl StatusbarCollector {
    pub fn new() -> Self {
        Self {
            info: StatusbarInfo {
                ping: "ping ...".into(),
                ..Default::default()
            },
            probe: ping,
            ping_target: std::env::var("RTOP_PING_TARGET").unwrap_or_else(|_| "1.1.1.1".into()),
        }
    }
}

impl Collector for StatusbarCollector {
    type Snapshot = crate::runner::StatusbarSnapshot;

    fn collect(&mut self) {
        self.info.uptime_seconds = crate::tools::system_uptime_secs();
        self.info.ping = (self.probe)(&self.ping_target);
        let sample = self
            .info
            .ping
            .strip_prefix("ping ")
            .and_then(|s| s.strip_suffix("ms"))
            .and_then(|s| s.parse().ok());
        self.info.ping_history.push_back(sample);
        while self.info.ping_history.len() > 3600 {
            self.info.ping_history.pop_front();
        }
    }

    fn snapshot(&self) -> Self::Snapshot {
        crate::runner::StatusbarSnapshot {
            info: self.info.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_ping_target_is_reported() {
        assert_eq!(ping("invalid"), "ping bad IP");
    }

    #[test]
    #[ignore = "sends an ICMP probe to the configured internet target"]
    fn live_ping() {
        let result = ping("1.1.1.1");
        println!("1.1.1.1: {result}");
        assert!(result.ends_with("ms"), "{result}");
    }

    #[test]
    fn collect_publishes_nonzero_uptime() {
        // Any process that has begun running has a non-zero uptime
        // by the time the test executes.
        let mut c = StatusbarCollector::new();
        c.probe = |_| "ping 4ms".into();
        c.collect();
        assert!(c.info.uptime_seconds > 0);
    }

    #[test]
    fn collect_is_monotonic_across_two_cycles() {
        let mut c = StatusbarCollector::new();
        c.probe = |_| "ping 4ms".into();
        c.collect();
        let first = c.info.uptime_seconds;
        // Uptime cannot decrease across collection cycles.
        std::thread::sleep(std::time::Duration::from_millis(50));
        c.collect();
        assert!(c.info.uptime_seconds >= first);
    }

    #[test]
    fn each_refresh_adds_one_ping_sample_and_bounds_history() {
        let mut c = StatusbarCollector::new();
        c.probe = |_| "ping 4ms".into();
        c.collect();
        c.collect();
        assert_eq!(c.info.ping_history.len(), 2);
        assert_eq!(c.info.ping_history.back(), Some(&Some(4)));
        c.probe = |_| "ping timeout".into();
        c.collect();
        assert_eq!(c.info.ping_history.back(), Some(&None));
        for _ in 0..3600 {
            c.collect();
        }
        assert_eq!(c.info.ping_history.len(), 3600);
    }
}

#[link(name = "iphlpapi")]
unsafe extern "system" {
    fn IcmpCreateFile() -> isize;
    fn IcmpCloseHandle(handle: isize) -> i32;
    fn IcmpSendEcho(
        handle: isize,
        destination: u32,
        data: *const u8,
        size: u16,
        options: *const std::ffi::c_void,
        reply: *mut std::ffi::c_void,
        reply_size: u32,
        timeout: u32,
    ) -> u32;
}

fn ping(target: &str) -> String {
    let Ok(ip) = target.parse::<std::net::Ipv4Addr>() else {
        return "ping bad IP".into();
    };
    // SAFETY: owned ICMP handle, valid request and aligned reply buffer. The first
    // three u32 fields of ICMP_ECHO_REPLY are Address, Status and RoundTripTime.
    unsafe {
        let handle = IcmpCreateFile();
        if handle == -1 {
            return "ping N/A".into();
        }
        let payload = b"rtop";
        let mut reply = [0u64; 128];
        let count = IcmpSendEcho(
            handle,
            u32::from_ne_bytes(ip.octets()),
            payload.as_ptr(),
            4,
            std::ptr::null(),
            reply.as_mut_ptr().cast(),
            1024,
            1000,
        );
        IcmpCloseHandle(handle);
        let fields = reply.as_ptr().cast::<u32>();
        if count > 0 && *fields.add(1) == 0 {
            format!("ping {}ms", *fields.add(2))
        } else {
            "ping timeout".into()
        }
    }
}
