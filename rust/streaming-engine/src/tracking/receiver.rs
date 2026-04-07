use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::UdpSocket;

use fvp_common::protocol::{TrackingData, ControllerState};

/// Tracking packet header type IDs (matches HMD sender)
const PACKET_HEAD_POSE: u8 = 0x01;
const PACKET_CONTROLLER: u8 = 0x02;

/// Receives tracking data (6DoF poses) and controller state from HMD via UDP.
pub struct TrackingReceiver {
    latest_head: Arc<Mutex<Option<TrackingData>>>,
    latest_controllers: Arc<Mutex<[Option<ControllerState>; 2]>>,
}

impl TrackingReceiver {
    pub fn new(
        latest_head: Arc<Mutex<Option<TrackingData>>>,
        latest_controllers: Arc<Mutex<[Option<ControllerState>; 2]>>,
    ) -> Self {
        Self {
            latest_head,
            latest_controllers,
        }
    }

    /// Run the UDP tracking receiver loop. Call from a tokio task.
    pub async fn run(&self, bind_addr: SocketAddr) -> std::io::Result<()> {
        let socket = UdpSocket::bind(bind_addr).await?;
        log::info!("Tracking receiver listening on {}", bind_addr);

        let mut buf = [0u8; 256]; // Tracking packets are small (<100 bytes)

        loop {
            let (len, _peer) = socket.recv_from(&mut buf).await?;
            if len < 1 {
                continue;
            }

            let packet_type = buf[0];
            let payload = &buf[1..len];

            match packet_type {
                PACKET_HEAD_POSE => {
                    if let Some(data) = parse_head_pose(payload) {
                        // Forward gaze data to NVENC for foveated encoding
                        if data.gaze_valid != 0 {
                            crate::engine::notify_gaze_update(data.gaze_x, data.gaze_y, true);
                        }
                        if let Ok(mut guard) = self.latest_head.lock() {
                            *guard = Some(data);
                        }
                    }
                }
                PACKET_CONTROLLER => {
                    if let Some(state) = parse_controller(payload) {
                        if let Ok(mut guard) = self.latest_controllers.lock() {
                            let idx = state.controller_id as usize;
                            if idx < 2 {
                                guard[idx] = Some(state);
                            }
                        }
                    }
                }
                _ => {
                    log::trace!("Unknown tracking packet type: 0x{:02x}", packet_type);
                }
            }
        }
    }
}

/// Parse head pose from payload (36 bytes: timestamp_ns(8) + position(12) + orientation(16))
fn parse_head_pose(data: &[u8]) -> Option<TrackingData> {
    if data.len() < 36 {
        return None;
    }

    let timestamp_ns = u64::from_le_bytes(data[0..8].try_into().ok()?);
    let px = f32::from_le_bytes(data[8..12].try_into().ok()?);
    let py = f32::from_le_bytes(data[12..16].try_into().ok()?);
    let pz = f32::from_le_bytes(data[16..20].try_into().ok()?);
    let ox = f32::from_le_bytes(data[20..24].try_into().ok()?);
    let oy = f32::from_le_bytes(data[24..28].try_into().ok()?);
    let oz = f32::from_le_bytes(data[28..32].try_into().ok()?);
    let ow = f32::from_le_bytes(data[32..36].try_into().ok()?);

    // Gaze data (optional — appended after base 36 bytes)
    let (gaze_x, gaze_y, gaze_valid) = if data.len() >= 45 {
        let gx = f32::from_le_bytes(data[36..40].try_into().ok()?);
        let gy = f32::from_le_bytes(data[40..44].try_into().ok()?);
        let gv = data[44];
        (gx, gy, gv)
    } else {
        (0.5, 0.5, 0) // Default center, not valid
    };

    Some(TrackingData {
        position: [px, py, pz],
        orientation: [ox, oy, oz, ow],
        timestamp_ns,
        gaze_x,
        gaze_y,
        gaze_valid,
    })
}

/// Parse controller state from payload
fn parse_controller(data: &[u8]) -> Option<ControllerState> {
    if data.len() < 53 {
        return None;
    }

    let controller_id = data[0];
    let timestamp_ns = u64::from_le_bytes(data[1..9].try_into().ok()?);
    let px = f32::from_le_bytes(data[9..13].try_into().ok()?);
    let py = f32::from_le_bytes(data[13..17].try_into().ok()?);
    let pz = f32::from_le_bytes(data[17..21].try_into().ok()?);
    let ox = f32::from_le_bytes(data[21..25].try_into().ok()?);
    let oy = f32::from_le_bytes(data[25..29].try_into().ok()?);
    let oz = f32::from_le_bytes(data[29..33].try_into().ok()?);
    let ow = f32::from_le_bytes(data[33..37].try_into().ok()?);
    let trigger = f32::from_le_bytes(data[37..41].try_into().ok()?);
    let grip = f32::from_le_bytes(data[41..45].try_into().ok()?);
    let thumbstick_x = f32::from_le_bytes(data[45..49].try_into().ok()?);
    let thumbstick_y = f32::from_le_bytes(data[49..53].try_into().ok()?);

    let button_flags = if data.len() >= 57 {
        u32::from_le_bytes(data[53..57].try_into().ok()?)
    } else {
        0
    };

    let battery_level = if data.len() >= 58 { data[57] } else { 100 };

    Some(ControllerState {
        controller_id,
        timestamp_ns,
        position: [px, py, pz],
        orientation: [ox, oy, oz, ow],
        trigger,
        grip,
        thumbstick_x,
        thumbstick_y,
        button_flags,
        battery_level,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_head_pose_packet(ts: u64, x: f32, y: f32, z: f32) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(PACKET_HEAD_POSE);
        buf.extend_from_slice(&ts.to_le_bytes());
        buf.extend_from_slice(&x.to_le_bytes());
        buf.extend_from_slice(&y.to_le_bytes());
        buf.extend_from_slice(&z.to_le_bytes());
        buf.extend_from_slice(&0.0f32.to_le_bytes()); // qx
        buf.extend_from_slice(&0.0f32.to_le_bytes()); // qy
        buf.extend_from_slice(&0.0f32.to_le_bytes()); // qz
        buf.extend_from_slice(&1.0f32.to_le_bytes()); // qw
        buf
    }

    #[test]
    fn test_parse_head_pose() {
        let pkt = make_head_pose_packet(12345, 1.0, 2.0, 3.0);
        let data = parse_head_pose(&pkt[1..]).unwrap();
        assert_eq!(data.timestamp_ns, 12345);
        assert_eq!(data.position, [1.0, 2.0, 3.0]);
        assert_eq!(data.orientation[3], 1.0); // w
    }

    #[test]
    fn test_parse_head_pose_too_short() {
        assert!(parse_head_pose(&[0u8; 10]).is_none());
        assert!(parse_head_pose(&[0u8; 35]).is_none()); // 1 byte short
    }

    #[test]
    fn test_parse_head_pose_with_gaze() {
        let mut pkt = make_head_pose_packet(100, 0.0, 0.0, 0.0);
        // Append gaze extension: gaze_x(4) + gaze_y(4) + gaze_valid(1) = 9 bytes
        pkt.extend_from_slice(&0.3f32.to_le_bytes()); // gaze_x
        pkt.extend_from_slice(&0.7f32.to_le_bytes()); // gaze_y
        pkt.push(1); // gaze_valid

        let data = parse_head_pose(&pkt[1..]).unwrap();
        assert!((data.gaze_x - 0.3).abs() < 1e-6);
        assert!((data.gaze_y - 0.7).abs() < 1e-6);
        assert_eq!(data.gaze_valid, 1);
    }

    #[test]
    fn test_parse_head_pose_without_gaze_defaults_center() {
        let pkt = make_head_pose_packet(100, 0.0, 0.0, 0.0);
        let data = parse_head_pose(&pkt[1..]).unwrap();
        assert_eq!(data.gaze_x, 0.5);
        assert_eq!(data.gaze_y, 0.5);
        assert_eq!(data.gaze_valid, 0);
    }

    fn make_controller_packet(id: u8, trigger: f32) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(PACKET_CONTROLLER);
        buf.push(id);                                        // controller_id
        buf.extend_from_slice(&1000u64.to_le_bytes());       // timestamp_ns
        buf.extend_from_slice(&0.1f32.to_le_bytes());        // px
        buf.extend_from_slice(&0.2f32.to_le_bytes());        // py
        buf.extend_from_slice(&0.3f32.to_le_bytes());        // pz
        buf.extend_from_slice(&0.0f32.to_le_bytes());        // qx
        buf.extend_from_slice(&0.0f32.to_le_bytes());        // qy
        buf.extend_from_slice(&0.0f32.to_le_bytes());        // qz
        buf.extend_from_slice(&1.0f32.to_le_bytes());        // qw
        buf.extend_from_slice(&trigger.to_le_bytes());       // trigger
        buf.extend_from_slice(&0.5f32.to_le_bytes());        // grip
        buf.extend_from_slice(&0.0f32.to_le_bytes());        // thumbstick_x
        buf.extend_from_slice(&0.0f32.to_le_bytes());        // thumbstick_y
        buf.extend_from_slice(&0x03u32.to_le_bytes());       // button_flags
        buf.push(80);                                         // battery_level
        buf
    }

    #[test]
    fn test_parse_controller() {
        let pkt = make_controller_packet(1, 0.75);
        let state = parse_controller(&pkt[1..]).unwrap();
        assert_eq!(state.controller_id, 1);
        assert_eq!(state.timestamp_ns, 1000);
        assert_eq!(state.position, [0.1, 0.2, 0.3]);
        assert!((state.trigger - 0.75).abs() < 1e-6);
        assert!((state.grip - 0.5).abs() < 1e-6);
        assert_eq!(state.button_flags, 0x03);
        assert_eq!(state.battery_level, 80);
    }

    #[test]
    fn test_parse_controller_too_short() {
        assert!(parse_controller(&[0u8; 20]).is_none());
        assert!(parse_controller(&[0u8; 52]).is_none()); // 1 byte short
    }

    #[test]
    fn test_parse_controller_minimal_53_bytes() {
        // Exactly 53 bytes = no button_flags or battery
        let pkt = make_controller_packet(0, 1.0);
        let minimal = &pkt[1..54]; // 53 bytes (id + 52 body)
        let state = parse_controller(minimal).unwrap();
        assert_eq!(state.controller_id, 0);
        assert_eq!(state.button_flags, 0); // default
        assert_eq!(state.battery_level, 100); // default
    }

    #[tokio::test]
    async fn test_tracking_receiver_udp() {
        let head = Arc::new(Mutex::new(None));
        let controllers = Arc::new(Mutex::new([None, None]));

        // Bind a temporary socket to get a free port, then drop it
        let tmp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let recv_addr = tmp.local_addr().unwrap();
        drop(tmp);

        // Spawn receiver on that port
        let head2 = head.clone();
        let controllers2 = controllers.clone();
        let recv_handle = tokio::spawn(async move {
            let r = TrackingReceiver::new(head2, controllers2);
            r.run(recv_addr).await.ok();
        });

        // Give receiver time to bind
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Send a head pose packet
        let pkt = make_head_pose_packet(999, 0.5, 1.5, 2.5);
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        sender.send_to(&pkt, recv_addr).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let data = head.lock().unwrap().clone();
        assert!(data.is_some());
        let d = data.unwrap();
        assert_eq!(d.position, [0.5, 1.5, 2.5]);

        recv_handle.abort();
    }
}
