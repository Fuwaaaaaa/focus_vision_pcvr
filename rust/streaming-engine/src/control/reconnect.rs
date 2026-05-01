//! Reconnect state machine — extracted from `engine.rs::run_streaming` so the
//! counter rules (separate accept-failure and reconnect-attempt counters,
//! exponential backoff cap, when to break vs continue) are unit-testable
//! without bringing up the full async streaming loop.
//!
//! Ownership split (Phase 4.2 device work — TODO):
//!   - `session_cancel` continues to scope the streaming task.
//!   - A second `connection_cancel` will scope only the TCP listener so
//!     audio + recording can persist across the 5 s hold window without
//!     being torn down on every Wi-Fi blip. That rewiring is intentionally
//!     out of scope for the software-only phase because validating
//!     "audio kept playing" requires a real HMD; the state machine here
//!     is the deterministic foundation that work will build on.

use std::time::Duration;

/// Hard cap on consecutive accept failures. Past this, the engine bails out —
/// distinct from `MAX_RECONNECT_ATTEMPTS` which only governs log noise.
pub(crate) const MAX_ACCEPT_FAILURES: u32 = 5;

/// Soft cap on Wi-Fi reconnection attempts. The engine keeps accepting past
/// this; the counter exists purely to surface a warning that the link is flaky.
pub(crate) const MAX_RECONNECT_ATTEMPTS: u32 = 10;

const BACKOFF_BASE: Duration = Duration::from_secs(1);
const BACKOFF_MAX_SHIFT: u32 = 4; // cap at 2^4 = 16x base = 16 s.

/// Per-engine state machine for the accept→session→hold→accept loop.
///
/// Two counters live here intentionally:
/// - `accept_failures` tracks `tcp_server.listen_and_accept()` errors and
///   gates the engine-stop decision (hard cap = `MAX_ACCEPT_FAILURES`).
/// - `reconnect_attempts` tracks Wi-Fi-style mid-stream drops so we can warn
///   on a flaky link without ever stopping the engine (soft cap = `MAX_RECONNECT_ATTEMPTS`).
///
/// Mixing them caused a regression once where a long Wi-Fi outage stopped
/// the engine; keeping them separate is now load-bearing.
#[derive(Debug, Default)]
pub(crate) struct ReconnectState {
    accept_failures: u32,
    reconnect_attempts: u32,
}

impl ReconnectState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn accept_failures(&self) -> u32 { self.accept_failures }
    pub(crate) fn reconnect_attempts(&self) -> u32 { self.reconnect_attempts }

    /// `tcp_server.listen_and_accept()` returned `Err` outside of a hold
    /// window. Increments the hard-cap counter.
    pub(crate) fn record_accept_failure(&mut self) {
        self.accept_failures = self.accept_failures.saturating_add(1);
    }

    /// `tcp_server.listen_and_accept()` succeeded — reset the hard-cap counter
    /// so prior transient failures don't push us past the threshold later.
    pub(crate) fn record_accept_success(&mut self) {
        self.accept_failures = 0;
    }

    /// Client sent DISCONNECT cleanly — both counters reset because neither
    /// the link nor the listener is in a degraded state.
    pub(crate) fn record_clean_disconnect(&mut self) {
        self.accept_failures = 0;
        self.reconnect_attempts = 0;
    }

    /// TCP read returned EOF / Wi-Fi dropped mid-stream. Increments the
    /// soft-cap counter only; the engine keeps trying to accept.
    pub(crate) fn record_connection_lost(&mut self) {
        self.reconnect_attempts = self.reconnect_attempts.saturating_add(1);
    }

    /// Protocol-level error (oversized message, bad framing). Same accounting
    /// as `record_connection_lost` — soft-cap only.
    pub(crate) fn record_protocol_error(&mut self) {
        self.reconnect_attempts = self.reconnect_attempts.saturating_add(1);
    }

    /// `tcp_server.listen_and_accept()` returned `Err` inside the 5 s hold
    /// window after a ConnectionLost. Counts as an accept failure (hard cap).
    pub(crate) fn record_hold_accept_failure(&mut self) {
        self.accept_failures = self.accept_failures.saturating_add(1);
    }

    /// Engine should bail out — only triggered by accept failures, never by
    /// reconnect attempts.
    pub(crate) fn should_stop_engine(&self) -> bool {
        self.accept_failures > MAX_ACCEPT_FAILURES
    }

    /// Soft warning gate for a flaky Wi-Fi link. Reads cleanly in the loop:
    /// `if state.is_reconnect_warning_due() { log::warn!(...) }`.
    pub(crate) fn is_reconnect_warning_due(&self) -> bool {
        self.reconnect_attempts > MAX_RECONNECT_ATTEMPTS
    }

    /// Exponential backoff delay before the next accept attempt. Returns
    /// `None` when no failures are pending so the caller can skip the sleep.
    /// Doubles each failure (1 → 2 → 4 → 8 → 16 s) and caps at 16 s.
    pub(crate) fn next_backoff(&self) -> Option<Duration> {
        if self.accept_failures == 0 {
            return None;
        }
        let shift = (self.accept_failures - 1).min(BACKOFF_MAX_SHIFT);
        Some(BACKOFF_BASE * 2u32.pow(shift))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_counters_zero() {
        let s = ReconnectState::new();
        assert_eq!(s.accept_failures(), 0);
        assert_eq!(s.reconnect_attempts(), 0);
        assert!(s.next_backoff().is_none(),
            "no failures yet → no backoff sleep needed");
    }

    #[test]
    fn test_accept_success_resets_failures() {
        let mut s = ReconnectState::new();
        s.record_accept_failure();
        s.record_accept_failure();
        assert_eq!(s.accept_failures(), 2);
        s.record_accept_success();
        assert_eq!(s.accept_failures(), 0);
    }

    #[test]
    fn test_clean_disconnect_resets_both_counters() {
        let mut s = ReconnectState::new();
        s.record_accept_failure();
        s.record_connection_lost();
        s.record_connection_lost();
        s.record_clean_disconnect();
        assert_eq!(s.accept_failures(), 0);
        assert_eq!(s.reconnect_attempts(), 0);
    }

    #[test]
    fn test_connection_lost_increments_reconnect_only() {
        let mut s = ReconnectState::new();
        s.record_connection_lost();
        s.record_connection_lost();
        assert_eq!(s.reconnect_attempts(), 2);
        assert_eq!(s.accept_failures(), 0,
            "Wi-Fi drops must not count as accept failures");
    }

    #[test]
    fn test_protocol_error_increments_reconnect_only() {
        let mut s = ReconnectState::new();
        s.record_protocol_error();
        assert_eq!(s.reconnect_attempts(), 1);
        assert_eq!(s.accept_failures(), 0);
    }

    #[test]
    fn test_hold_accept_failure_increments_accepts() {
        let mut s = ReconnectState::new();
        s.record_hold_accept_failure();
        assert_eq!(s.accept_failures(), 1);
        assert_eq!(s.reconnect_attempts(), 0);
    }

    #[test]
    fn test_break_after_max_accept_failures() {
        let mut s = ReconnectState::new();
        for _ in 0..MAX_ACCEPT_FAILURES {
            s.record_accept_failure();
            assert!(!s.should_stop_engine(),
                "must keep going at or below the limit ({})",
                s.accept_failures());
        }
        // One more push us past the cap.
        s.record_accept_failure();
        assert!(s.should_stop_engine(),
            "exceeding {} accept failures must stop the engine",
            MAX_ACCEPT_FAILURES);
    }

    #[test]
    fn test_no_break_at_max_reconnect_attempts() {
        // Reconnect attempts are warning-only; the engine must keep accepting
        // even past MAX_RECONNECT_ATTEMPTS so a long Wi-Fi outage doesn't
        // permanently kill the session.
        let mut s = ReconnectState::new();
        for _ in 0..(MAX_RECONNECT_ATTEMPTS + 5) {
            s.record_connection_lost();
        }
        assert!(!s.should_stop_engine(),
            "reconnect attempts must not stop the engine");
        assert!(s.is_reconnect_warning_due(),
            "warning should fire once we've crossed the soft cap");
    }

    #[test]
    fn test_backoff_exponential_capped_at_16s() {
        let mut s = ReconnectState::new();
        s.record_accept_failure(); // 1 failure → 1 s
        assert_eq!(s.next_backoff(), Some(Duration::from_secs(1)));
        s.record_accept_failure(); // 2 → 2 s
        assert_eq!(s.next_backoff(), Some(Duration::from_secs(2)));
        s.record_accept_failure(); // 3 → 4 s
        assert_eq!(s.next_backoff(), Some(Duration::from_secs(4)));
        s.record_accept_failure(); // 4 → 8 s
        assert_eq!(s.next_backoff(), Some(Duration::from_secs(8)));
        s.record_accept_failure(); // 5 → 16 s (cap)
        assert_eq!(s.next_backoff(), Some(Duration::from_secs(16)));
        // Even past the cap, backoff stays at 16 s.
        s.record_accept_failure();
        assert_eq!(s.next_backoff(), Some(Duration::from_secs(16)));
    }

    #[test]
    fn test_backoff_none_after_success_reset() {
        let mut s = ReconnectState::new();
        s.record_accept_failure();
        s.record_accept_failure();
        assert!(s.next_backoff().is_some());
        s.record_accept_success();
        assert!(s.next_backoff().is_none(),
            "successful accept clears the backoff requirement");
    }
}
