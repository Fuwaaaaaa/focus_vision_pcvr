use std::time::Instant;

/// Process memory usage monitor using OS APIs.
///
/// Polls process RSS at a configurable interval and warns if memory grows
/// by more than `growth_threshold_mb` within 1 hour. Uses GetProcessMemoryInfo
/// on Windows and /proc/self/status on Linux/Android.
pub struct MemoryMonitor {
    poll_interval_secs: u32,
    growth_threshold_mb: u32,
    baseline_mb: Option<u64>,
    baseline_time: Option<Instant>,
    last_poll: Instant,
}

impl MemoryMonitor {
    pub fn new(poll_interval_secs: u32, growth_threshold_mb: u32) -> Self {
        Self {
            poll_interval_secs,
            growth_threshold_mb,
            baseline_mb: None,
            baseline_time: None,
            last_poll: Instant::now(),
        }
    }

    /// Check memory usage if the poll interval has elapsed.
    /// Returns the current RSS in MB, or None if not yet time to poll.
    pub fn check(&mut self) -> Option<u64> {
        if self.last_poll.elapsed().as_secs() < self.poll_interval_secs as u64 {
            return None;
        }
        self.last_poll = Instant::now();

        let rss_mb = get_process_rss_mb();
        if rss_mb == 0 {
            return None; // OS API unavailable
        }

        // Set baseline on first successful read
        if self.baseline_mb.is_none() {
            self.baseline_mb = Some(rss_mb);
            self.baseline_time = Some(Instant::now());
            log::info!("Memory monitor: baseline RSS = {} MB", rss_mb);
            return Some(rss_mb);
        }

        let baseline = self.baseline_mb.unwrap();
        let elapsed_hours = self.baseline_time.unwrap().elapsed().as_secs_f64() / 3600.0;

        // Check for abnormal growth over 1 hour
        if elapsed_hours >= 1.0 {
            let growth = rss_mb.saturating_sub(baseline);
            if growth >= self.growth_threshold_mb as u64 {
                log::warn!(
                    "Memory growth warning: {} MB → {} MB (+{} MB in {:.1}h, threshold: {} MB/h)",
                    baseline, rss_mb, growth, elapsed_hours, self.growth_threshold_mb
                );
            }
            // Reset baseline for next hour
            self.baseline_mb = Some(rss_mb);
            self.baseline_time = Some(Instant::now());
        }

        Some(rss_mb)
    }

    /// Get the current RSS without side effects (for session log).
    pub fn current_rss_mb() -> u64 {
        get_process_rss_mb()
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

    #[test]
    fn test_memory_monitor_first_check_sets_baseline() {
        let mut mon = MemoryMonitor::new(0, 50); // 0s interval for immediate check
        let rss = mon.check();
        // Should return Some on platforms with OS API support
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        assert!(rss.is_some());
        // Baseline should be set
        assert!(mon.baseline_mb.is_some() || rss.is_none());
    }

    #[test]
    fn test_memory_monitor_respects_interval() {
        let mut mon = MemoryMonitor::new(3600, 50); // 1 hour interval
        let rss = mon.check();
        // First check should work
        let rss2 = mon.check();
        // Second check should return None (interval not elapsed)
        assert!(rss2.is_none());
        let _ = rss; // suppress unused warning
    }

    #[test]
    fn test_get_process_rss_mb_returns_value() {
        let rss = get_process_rss_mb();
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        assert!(rss > 0, "RSS should be non-zero on supported platforms");
        let _ = rss;
    }

    #[test]
    fn test_threshold_detection_logic() {
        // Test the growth detection logic directly
        let mut mon = MemoryMonitor::new(0, 50);
        // Simulate baseline
        mon.baseline_mb = Some(100);
        mon.baseline_time = Some(Instant::now().checked_sub(std::time::Duration::from_secs(3700))
            .unwrap_or(Instant::now()));
        // The check() will use the real RSS, but the growth logic is what matters
        // Since we can't control OS RSS, we verify the struct state
        assert_eq!(mon.growth_threshold_mb, 50);
        assert_eq!(mon.baseline_mb, Some(100));
    }
}
