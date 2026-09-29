use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::config::MemoryMonitorConfig;

/// How long a growth measurement spans.
const WINDOW: Duration = Duration::from_secs(3600);

/// Process memory usage monitor using OS APIs.
///
/// Takes the process RSS (see [`run`]) and warns if memory grows by more
/// than `growth_threshold_mb` within 1 hour. Uses GetProcessMemoryInfo on
/// Windows and /proc/self/status on Linux/Android. The engine runs inside
/// vrserver.exe, so the figure is SteamVR's process as a whole.
pub struct MemoryMonitor {
    growth_threshold_mb: u32,
    /// The reading the current hour is measured from.
    baseline: Option<(u64, Instant)>,
}

impl MemoryMonitor {
    pub fn new(growth_threshold_mb: u32) -> Self {
        Self { growth_threshold_mb, baseline: None }
    }

    /// Take a reading made at `now`. Once an hour has passed since the
    /// baseline, logs the growth, starts the next hour from this reading,
    /// and returns the growth if it reached the threshold.
    pub fn observe(&mut self, rss_mb: u64, now: Instant) -> Option<u64> {
        let Some((baseline_mb, since)) = self.baseline else {
            log::info!("Memory monitor: {} MB at start", rss_mb);
            self.baseline = Some((rss_mb, now));
            return None;
        };
        let elapsed = now.saturating_duration_since(since);
        if elapsed < WINDOW {
            return None;
        }
        self.baseline = Some((rss_mb, now));
        let growth = rss_mb.saturating_sub(baseline_mb);
        let hours = elapsed.as_secs_f64() / 3600.0;
        if growth >= u64::from(self.growth_threshold_mb) {
            log::warn!(
                "Memory growth warning: {} MB → {} MB (+{} MB in {:.1} h, threshold {} MB/h)",
                baseline_mb, rss_mb, growth, hours, self.growth_threshold_mb
            );
            Some(growth)
        } else {
            log::info!("Memory monitor: {} MB ({} MB an hour ago)", rss_mb, baseline_mb);
            None
        }
    }

    /// Get the current RSS without side effects (for session log).
    pub fn current_rss_mb() -> u64 {
        get_process_rss_mb()
    }
}

/// Read the process's memory every `poll_interval_seconds` until `cancel`.
/// REGRESSION: `[memory_monitor]` was read and nothing ran it.
pub async fn run(config: MemoryMonitorConfig, cancel: CancellationToken) {
    let mut monitor = MemoryMonitor::new(config.growth_threshold_mb);
    // validate() keeps the interval at 10 s or more; 0 would panic here.
    let period = Duration::from_secs(u64::from(config.poll_interval_seconds.max(1)));
    let mut tick = tokio::time::interval(period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = tick.tick() => {}
            _ = cancel.cancelled() => return,
        }
        let rss_mb = get_process_rss_mb();
        if rss_mb > 0 {
            monitor.observe(rss_mb, Instant::now());
        }
    }
}

/// Get process RSS in MB using OS-specific APIs.
#[cfg(target_os = "windows")]
fn get_process_rss_mb() -> u64 {
    use std::mem;

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

    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(
            process: isize,
            ppsmemCounters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
    }

    unsafe {
        let mut pmc: ProcessMemoryCounters = mem::zeroed();
        pmc.cb = mem::size_of::<ProcessMemoryCounters>() as u32;
        if K32GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb) != 0 {
            (pmc.WorkingSetSize / (1024 * 1024)) as u64
        } else {
            log::warn!("GetProcessMemoryInfo failed");
            0
        }
    }
}

#[cfg(target_os = "linux")]
fn get_process_rss_mb() -> u64 {
    match std::fs::read_to_string("/proc/self/status") {
        Ok(status) => {
            for line in status.lines() {
                if line.starts_with("VmRSS:") {
                    // Format: "VmRSS:    12345 kB"
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(kb) = parts[1].parse::<u64>() {
                            return kb / 1024;
                        }
                    }
                }
            }
            log::warn!("VmRSS not found in /proc/self/status");
            0
        }
        Err(e) => {
            log::warn!("Failed to read /proc/self/status: {}", e);
            0
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn get_process_rss_mb() -> u64 {
    log::debug!("Memory monitor not supported on this platform");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn test_first_reading_is_the_baseline() {
        let mut mon = MemoryMonitor::new(50);
        let t0 = Instant::now();
        assert_eq!(mon.observe(400, t0), None);
        assert_eq!(mon.baseline, Some((400, t0)));
    }

    #[test]
    fn test_growth_is_judged_once_an_hour() {
        let mut mon = MemoryMonitor::new(50);
        let t0 = Instant::now();
        mon.observe(400, t0);
        // Within the hour nothing is judged, however high it goes.
        assert_eq!(mon.observe(900, t0 + 59 * MINUTE), None);
        // An hour on: +60 MB reaches the 50 MB threshold.
        assert_eq!(mon.observe(460, t0 + 60 * MINUTE), Some(60));
        // The next hour starts from 460: +40 MB is under it.
        assert_eq!(mon.observe(500, t0 + 120 * MINUTE), None);
        assert_eq!(mon.baseline, Some((500, t0 + 120 * MINUTE)));
        // Shrinking is not growth.
        assert_eq!(mon.observe(300, t0 + 180 * MINUTE), None);
    }

    #[test]
    fn test_get_process_rss_mb_returns_value() {
        let rss = get_process_rss_mb();
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        assert!(rss > 0, "RSS should be non-zero on supported platforms");
        let _ = rss;
    }

    #[tokio::test]
    async fn test_run_stops_when_cancelled() {
        let cancel = CancellationToken::new();
        let task = tokio::spawn(run(MemoryMonitorConfig::default(), cancel.clone()));
        tokio::time::sleep(Duration::from_millis(50)).await; // the first reading
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("the monitor must stop on cancel")
            .unwrap();
    }
}
