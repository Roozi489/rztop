use super::win::{PdhCounter, PdhQuery};
use crate::domain::process::ProcInfo;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct ProcessGpu {
    counters: Option<(PdhQuery, PdhCounter, PdhCounter)>,
}

fn pid(name: &str) -> Option<u32> {
    name.strip_prefix("pid_")?.split('_').next()?.parse().ok()
}

impl ProcessGpu {
    pub fn update(&mut self, procs: &mut [ProcInfo]) {
        if self.counters.is_none() {
            self.counters = (|| {
                let q = PdhQuery::open().ok()?;
                let usage = q.add_english(r"\GPU Engine(*)\Utilization Percentage")?;
                let mem = q.add_english(r"\GPU Process Memory(*)\Dedicated Usage")?;
                Some((q, usage, mem))
            })();
        }
        let Some((q, usage, mem)) = &self.counters else {
            return;
        };
        if q.collect().is_err() {
            self.counters = None;
            return;
        }
        let usage = usage.array();
        let mem = mem.array();
        let mut percents = HashMap::<u32, f64>::new();
        let mut bytes = HashMap::<u32, u64>::new();
        if let Some(values) = &usage {
            for (name, value) in values {
                if let Some(pid) = pid(name) {
                    let current = percents.entry(pid).or_default();
                    *current = current.max(value.clamp(0.0, 100.0));
                }
            }
        }
        if let Some(values) = &mem {
            for (name, value) in values {
                if let Some(pid) = pid(name) {
                    let current = bytes.entry(pid).or_default();
                    *current = current.saturating_add(value.max(0.0) as u64);
                }
            }
        }
        for p in procs {
            p.gpu_p = usage
                .as_ref()
                .map(|_| percents.get(&p.pid).copied().unwrap_or(0.0));
            p.gpu_mem = mem
                .as_ref()
                .map(|_| bytes.get(&p.pid).copied().unwrap_or(0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires live Windows GPU counters"]
    fn live_gpu_counters() {
        let mut collector = ProcessGpu::default();
        let mut procs = vec![ProcInfo {
            pid: std::process::id(),
            ..Default::default()
        }];
        collector.update(&mut procs);
        std::thread::sleep(std::time::Duration::from_secs(1));
        collector.update(&mut procs);
        let (_, usage, mem) = collector.counters.as_ref().expect("GPU query");
        let usage = usage.array().expect("GPU engine readings");
        let mem = mem.array().expect("GPU memory readings");
        assert!(!usage.is_empty());
        assert!(!mem.is_empty());
        println!(
            "GPU engine instances: {}; GPU memory instances: {}; peak engine: {:.1}%",
            usage.len(),
            mem.len(),
            usage.iter().map(|(_, v)| *v).fold(0.0, f64::max)
        );
    }

    #[test]
    fn instance_pid() {
        assert_eq!(pid("pid_123_luid_0x0000_phys_0_eng_1"), Some(123));
        assert_eq!(pid("_Total"), None);
        assert_eq!(pid("pid_bad_luid"), None);
    }
}
