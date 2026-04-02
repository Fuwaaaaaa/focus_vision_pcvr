use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use fvp_common::{HEARTBEAT_INTERVAL_MS, HEARTBEAT_MAX_MISSES};

/// Heartbeat monitor — detects connection loss from missed heartbeats.
pub struct HeartbeatMonitor {
    last_received: Arc<AtomicU64>, // epoch millis of last received heartbeat
    connected: Arc<AtomicBool>,
    sequence: u32,
}

impl HeartbeatMonitor {
    pub fn new() -> Self {
        let now = epoch_millis();
        Self {
            last_received: Arc::new(AtomicU64::new(now)),
            connected: Arc::new(AtomicBool::new(true)),
            sequence: 0,
        }
    }

    /// Record that a heartbeat was received.
    pub fn on_heartbeat_received(&self) {
        self.last_received.store(epoch_millis(), Ordering::Relaxed);
        self.connected.store(true, Ordering::Relaxed);
    }

    /// Check if the connection is still alive.
    /// Returns false if heartbeats have been missed beyond the threshold.
    pub fn check(&self) -> bool {
        let last = self.last_received.load(Ordering::Relaxed);
        let now = epoch_millis();
        let elapsed_ms = now.saturating_sub(last);
        let timeout_ms = HEARTBEAT_INTERVAL_MS * HEARTBEAT_MAX_MISSES as u64;

        if elapsed_ms > timeout_ms {
            self.connected.store(false, Ordering::Relaxed);
            false
        } else {
            true
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Generate a heartbeat packet to send.
    pub fn make_heartbeat_packet(&mut self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(13);
        buf.push(fvp_common::protocol::msg_type::HEARTBEAT);
        buf.extend_from_slice(&self.sequence.to_le_bytes());
        buf.extend_from_slice(&epoch_millis().to_le_bytes());
        self.sequence += 1;
        buf
    }

    pub fn last_received_clone(&self) -> Arc<AtomicU64> {
        self.last_received.clone()
    }

    pub fn connected_clone(&self) -> Arc<AtomicBool> {
        self.connected.clone()
    }
}

fn epoch_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state_connected() {
        let mon = HeartbeatMonitor::new();
        assert!(mon.is_connected());
        assert!(mon.check());
    }

    #[test]
    fn test_heartbeat_received_stays_connected() {
        let mon = HeartbeatMonitor::new();
        mon.on_heartbeat_received();
        assert!(mon.check());
    }

    #[test]
    fn test_make_heartbeat_packet() {
        let mut mon = HeartbeatMonitor::new();
        let pkt1 = mon.make_heartbeat_packet();
        let pkt2 = mon.make_heartbeat_packet();
        assert_eq!(pkt1[0], fvp_common::protocol::msg_type::HEARTBEAT);
        assert_eq!(pkt1.len(), 13);
        // Sequence should increment
        let seq1 = u32::from_le_bytes([pkt1[1], pkt1[2], pkt1[3], pkt1[4]]);
        let seq2 = u32::from_le_bytes([pkt2[1], pkt2[2], pkt2[3], pkt2[4]]);
        assert_eq!(seq2, seq1 + 1);
    }
}
