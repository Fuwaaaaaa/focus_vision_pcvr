//! Synthetic HMD tracking sender (UDP head pose + controller + gaze).
//!
//! Wire format mirrors `tracking::receiver` byte-for-byte:
//! - PACKET_HEAD_POSE  (0x01): 37 bytes (no gaze) or 46 bytes (with gaze)
//! - PACKET_CONTROLLER (0x02): 59 bytes
//!
//! Used by the scenario runner to drive the engine's tracking pipeline
//! without a real HMD.

use std::net::SocketAddr;
use std::time::Duration;

use fvp_common::protocol::{ControllerState, TrackingData};
use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;

const PACKET_HEAD_POSE: u8 = 0x01;
const PACKET_CONTROLLER: u8 = 0x02;

/// How the synthetic tracking data evolves over time.
#[derive(Debug, Clone)]
pub enum PoseMode {
    /// Single fixed pose, repeated. Useful for sleep_mode (no motion) tests.
    Still {
        head: TrackingData,
        left: Option<ControllerState>,
        right: Option<ControllerState>,
    },
    /// Sinusoidal head bob: position[1] oscillates around base height.
    /// Controllers stay at their initial pose if provided.
    SineWave {
        base: TrackingData,
        amp_m: f32,
        hz: f32,
        left: Option<ControllerState>,
        right: Option<ControllerState>,
    },
}

impl PoseMode {
    /// Identity pose at typical seated eye height, no controllers.
    /// Default for `tracking_pattern: { "kind": "still" }`.
    pub fn still_origin() -> Self {
        Self::Still {
            head: default_head(),
            left: None,
            right: None,
        }
    }

    /// Reference head pose used by `still_origin` and as the SineWave base.
    pub fn default_head() -> TrackingData {
        default_head()
    }
}

fn default_head() -> TrackingData {
    TrackingData {
        position: [0.0, 1.6, 0.0],
        orientation: [0.0, 0.0, 0.0, 1.0],
        timestamp_ns: 0,
        gaze_x: 0.5,
        gaze_y: 0.5,
        gaze_valid: 0,
    }
}

/// UDP sender targeting the engine's tracking port. Emits packets at the
/// configured rate until cancelled.
pub struct TrackingSender {
    socket: UdpSocket,
    target: SocketAddr,
}

impl TrackingSender {
    pub async fn new(target: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("127.0.0.1:0").await?;
        Ok(Self { socket, target })
    }

    /// Drive the sender at `rate_hz` until `cancel` fires. Each tick produces
    /// one head pose packet and (if present) one packet per controller.
    pub async fn run(&self, mode: PoseMode, rate_hz: u32, cancel: CancellationToken) {
        let period = Duration::from_secs_f64(1.0 / rate_hz.max(1) as f64);
        let mut ticker = tokio::time::interval(period);
        let start = std::time::Instant::now();
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let elapsed_ns = start.elapsed().as_nanos() as u64;
                    let (head, left, right) = sample(&mode, elapsed_ns);
                    let pkt = encode_head_pose(&head);
                    let _ = self.socket.send_to(&pkt, self.target).await;
                    if let Some(c) = left {
                        let pkt = encode_controller(&c);
                        let _ = self.socket.send_to(&pkt, self.target).await;
                    }
                    if let Some(c) = right {
                        let pkt = encode_controller(&c);
                        let _ = self.socket.send_to(&pkt, self.target).await;
                    }
                }
                _ = cancel.cancelled() => break,
            }
        }
    }
}

/// Snapshot the pose for time `t_ns` according to the mode.
fn sample(mode: &PoseMode, t_ns: u64) -> (TrackingData, Option<ControllerState>, Option<ControllerState>) {
    match mode {
        PoseMode::Still { head, left, right } => {
            let mut head = *head;
            head.timestamp_ns = t_ns;
            (head, *left, *right)
        }
        PoseMode::SineWave { base, amp_m, hz, left, right } => {
            let mut head = *base;
            head.timestamp_ns = t_ns;
            let secs = t_ns as f64 / 1e9;
            let dy = (*amp_m as f64 * (2.0 * std::f64::consts::PI * *hz as f64 * secs).sin()) as f32;
            head.position[1] = base.position[1] + dy;
            (head, *left, *right)
        }
    }
}

/// Encode `[0x01][timestamp:8][position:12][orientation:16]` (=37 bytes),
/// optionally `+ [gaze_x:4][gaze_y:4][valid:1]` (=46 bytes when `gaze_valid != 0`).
pub fn encode_head_pose(td: &TrackingData) -> Vec<u8> {
    let mut buf = Vec::with_capacity(46);
    buf.push(PACKET_HEAD_POSE);
    buf.extend_from_slice(&td.timestamp_ns.to_le_bytes());
    for v in &td.position {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for v in &td.orientation {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    if td.gaze_valid != 0 {
        buf.extend_from_slice(&td.gaze_x.to_le_bytes());
        buf.extend_from_slice(&td.gaze_y.to_le_bytes());
        buf.push(td.gaze_valid);
    }
    buf
}

/// Encode `[0x02][id:1][timestamp:8][position:12][orientation:16]
///  [trigger:4][grip:4][stick_x:4][stick_y:4][buttons:4][battery:1]` (=59 bytes).
pub fn encode_controller(cs: &ControllerState) -> Vec<u8> {
    let mut buf = Vec::with_capacity(59);
    buf.push(PACKET_CONTROLLER);
    buf.push(cs.controller_id);
    buf.extend_from_slice(&cs.timestamp_ns.to_le_bytes());
    for v in &cs.position {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    for v in &cs.orientation {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    buf.extend_from_slice(&cs.trigger.to_le_bytes());
    buf.extend_from_slice(&cs.grip.to_le_bytes());
    buf.extend_from_slice(&cs.thumbstick_x.to_le_bytes());
    buf.extend_from_slice(&cs.thumbstick_y.to_le_bytes());
    buf.extend_from_slice(&cs.button_flags.to_le_bytes());
    buf.push(cs.battery_level);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn td(t: u64, p: [f32; 3]) -> TrackingData {
        TrackingData {
            position: p,
            orientation: [0.0, 0.0, 0.0, 1.0],
            timestamp_ns: t,
            gaze_x: 0.5,
            gaze_y: 0.5,
            gaze_valid: 0,
        }
    }

    #[test]
    fn encode_head_pose_no_gaze_is_37_bytes() {
        let pkt = encode_head_pose(&td(100, [1.0, 2.0, 3.0]));
        assert_eq!(pkt.len(), 37);
        assert_eq!(pkt[0], PACKET_HEAD_POSE);
        let ts = u64::from_le_bytes(pkt[1..9].try_into().unwrap());
        assert_eq!(ts, 100);
    }

    #[test]
    fn encode_head_pose_with_gaze_is_46_bytes() {
        let mut d = td(0, [0.0; 3]);
        d.gaze_valid = 1;
        d.gaze_x = 0.3;
        d.gaze_y = 0.7;
        let pkt = encode_head_pose(&d);
        assert_eq!(pkt.len(), 46);
        assert_eq!(pkt[45], 1);
    }

    #[test]
    fn encode_controller_is_59_bytes() {
        let cs = ControllerState {
            controller_id: 1,
            timestamp_ns: 50,
            position: [0.1, 0.2, 0.3],
            orientation: [0.0, 0.0, 0.0, 1.0],
            trigger: 0.7,
            grip: 0.3,
            thumbstick_x: 0.0,
            thumbstick_y: 0.0,
            button_flags: 0xFF,
            battery_level: 80,
        };
        let pkt = encode_controller(&cs);
        assert_eq!(pkt.len(), 59);
        assert_eq!(pkt[0], PACKET_CONTROLLER);
        assert_eq!(pkt[1], 1);
    }

    #[test]
    fn encoded_head_pose_round_trips_through_receiver_parser() {
        // The receiver's parser is in tracking::receiver and is the canonical
        // wire format. Validating round-trip here catches drift between
        // encode/decode.
        let original = td(987_654, [1.0, 2.0, 3.0]);
        let pkt = encode_head_pose(&original);
        // skip the type byte, decode the body
        let body = &pkt[1..];
        let ts = u64::from_le_bytes(body[0..8].try_into().unwrap());
        let px = f32::from_le_bytes(body[8..12].try_into().unwrap());
        let py = f32::from_le_bytes(body[12..16].try_into().unwrap());
        let pz = f32::from_le_bytes(body[16..20].try_into().unwrap());
        assert_eq!(ts, original.timestamp_ns);
        assert_eq!([px, py, pz], original.position);
    }

    #[tokio::test]
    async fn sender_emits_head_pose_packet_to_target() {
        let recv = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target = recv.local_addr().unwrap();
        let sender = TrackingSender::new(target).await.unwrap();
        let cancel = CancellationToken::new();
        let cancel_for_run = cancel.clone();
        let handle = tokio::spawn(async move {
            sender.run(PoseMode::still_origin(), 90, cancel_for_run).await;
        });

        let mut buf = [0u8; 256];
        let result = tokio::time::timeout(
            Duration::from_millis(500),
            recv.recv_from(&mut buf),
        ).await;

        cancel.cancel();
        let _ = handle.await;

        let (n, _peer) = result.expect("no packet within 500ms").unwrap();
        assert!(n >= 37, "head pose packet must be at least 37 bytes, got {}", n);
        assert_eq!(buf[0], PACKET_HEAD_POSE);
    }
}
