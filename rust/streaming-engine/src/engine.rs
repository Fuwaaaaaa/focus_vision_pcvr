use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::control::tcp_server::TcpControlServer;
use crate::metrics::latency::{FrameTimestamps, LatencyTracker};
use crate::pipeline;
use crate::tracking::receiver::TrackingReceiver;
use crate::transport::rtp::RtpPacketizer;
use crate::transport::udp::UdpSender;
use fvp_common::protocol::{ControllerState, TrackingData};

/// H.265 encoded frame data submitted from the C++ OpenVR driver.
/// The C++ driver handles D3D11 texture capture, NV12 conversion, and
/// NVENC encoding. Rust receives only the encoded NAL units.
pub struct EncodedFrame {
    pub frame_index: u32,
    pub nal_data: Vec<u8>,
    pub is_idr: bool,
    pub timestamps: FrameTimestamps,
}

/// The main streaming engine running on a tokio runtime.
pub struct StreamingEngine {
    runtime: Runtime,
    frame_tx: mpsc::Sender<EncodedFrame>,
    latest_tracking: Arc<StdMutex<Option<TrackingData>>>,
    latest_controllers: Arc<StdMutex<[Option<ControllerState>; 2]>>,
    latency_tracker: Arc<StdMutex<LatencyTracker>>,
    cancel_token: CancellationToken,
    config: AppConfig,
}

impl StreamingEngine {
    pub fn new(config: AppConfig) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Build tokio runtime with limited threads (eng review decision #1)
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("fvp-stream")
            .build()?;

        let (frame_tx, frame_rx) = mpsc::channel::<EncodedFrame>(4);
        let latest_tracking = Arc::new(StdMutex::new(None));
        let latest_controllers: Arc<StdMutex<[Option<ControllerState>; 2]>> =
            Arc::new(StdMutex::new([None, None]));
        let latency_tracker = Arc::new(StdMutex::new(LatencyTracker::new(90)));

        let cancel_token = CancellationToken::new();
        let tracking_clone = latest_tracking.clone();
        let controllers_clone = latest_controllers.clone();
        let tracker_clone = latency_tracker.clone();
        let config_clone = config.clone();

        // Spawn the main streaming task
        let cancel = cancel_token.clone();
        runtime.spawn(async move {
            tokio::select! {
                result = run_streaming(config_clone, frame_rx, tracking_clone, tracker_clone) => {
                    if let Err(e) = result {
                        log::error!("Streaming engine error: {}", e);
                    }
                }
                _ = cancel.cancelled() => {
                    log::info!("Streaming task cancelled");
                }
            }
        });

        // Spawn tracking receiver (UDP, separate port)
        let tracking_head = latest_tracking.clone();
        let tracking_ctrl = latest_controllers.clone();
        let tracking_port = config.network.udp_port + 2; // 9947
        let cancel = cancel_token.clone();
        runtime.spawn(async move {
            let receiver = TrackingReceiver::new(tracking_head, tracking_ctrl);
            let addr: SocketAddr = format!("0.0.0.0:{}", tracking_port).parse().unwrap();
            tokio::select! {
                result = receiver.run(addr) => {
                    if let Err(e) = result {
                        log::error!("Tracking receiver error: {}", e);
                    }
                }
                _ = cancel.cancelled() => {
                    log::info!("Tracking receiver cancelled");
                }
            }
        });

        Ok(Self {
            runtime,
            frame_tx,
            latest_tracking,
            latest_controllers,
            latency_tracker,
            cancel_token,
            config,
        })
    }

    /// Submit a frame for encoding and sending. Called from C++ thread.
    pub fn submit_frame(&self, frame: EncodedFrame) -> bool {
        match self.frame_tx.try_send(frame) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                log::warn!("Frame dropped: send channel full");
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                log::error!("Frame channel closed");
                false
            }
        }
    }

    /// Get latest tracking data. Called from C++ thread.
    pub fn get_tracking(&self) -> Option<TrackingData> {
        self.latest_tracking.lock().map_err(|e| log::error!("Tracking lock poisoned: {}", e)).ok()?.clone()
    }

    /// Get latest controller state. Called from C++ thread.
    /// `id`: 0 = left, 1 = right.
    pub fn get_controller(&self, id: u8) -> Option<ControllerState> {
        let guard = self.latest_controllers.lock().map_err(|e| log::error!("Controller lock poisoned: {}", e)).ok()?;
        let idx = id as usize;
        if idx < 2 { guard[idx] } else { None }
    }

    /// Cancel all async tasks for graceful shutdown.
    pub fn shutdown(&self) {
        self.cancel_token.cancel();
    }

    /// Log latency stats periodically.
    pub fn log_stats(&self) {
        if let Ok(tracker) = self.latency_tracker.lock() {
            if let Some(avg) = tracker.avg_pc_latency_us() {
                log::info!(
                    "Latency (PC side): avg={}us, encode={}us, frames={}",
                    avg,
                    tracker.avg_encode_latency_us().unwrap_or(0),
                    tracker.frame_count()
                );
            }
        }
    }
}

async fn run_streaming(
    config: AppConfig,
    mut frame_rx: mpsc::Receiver<EncodedFrame>,
    tracking: Arc<StdMutex<Option<TrackingData>>>,
    latency_tracker: Arc<StdMutex<LatencyTracker>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Step 1: Wait for HMD to connect via TCP
    let tcp_server = TcpControlServer::new(config.clone());
    let (_tcp_stream, peer_addr) = tcp_server.listen_and_accept().await?;
    log::info!("HMD connected from {}, starting video stream", peer_addr);

    // Step 2: Create UDP sender for video — send to the HMD's IP
    let udp_target: SocketAddr = SocketAddr::new(peer_addr.ip(), config.network.udp_port + 1);
    let udp_sender = UdpSender::new(udp_target).await?;

    // Step 3: Process frames
    let mut packetizer = RtpPacketizer::new(0x46565000); // "FVP\0"
    let mut fec_encoder = crate::transport::fec::FecEncoder::new(config.network.fec_redundancy);
    let mut frame_count: u64 = 0;

    while let Some(mut frame) = frame_rx.recv().await {
        // NAL data arrives pre-encoded from C++ NVENC encoder.
        // Mark encode timestamps for latency tracking (encode happened in C++,
        // but we record receipt time here).
        frame.timestamps.mark_encode_start();
        frame.timestamps.mark_encode_end();

        let timestamp_90khz = (frame_count * (fvp_common::RTP_CLOCK_RATE as u64 / 90)) as u32;

        let packets = pipeline::encode_frame_to_packets_with_fec(
            &frame.nal_data,
            frame.frame_index,
            timestamp_90khz,
            frame.is_idr,
            &mut fec_encoder,
            &mut packetizer,
        );

        // Send via UDP
        if let Err(e) = udp_sender.send_all(&packets).await {
            log::warn!("UDP send error: {}", e);
        }

        frame.timestamps.mark_send();

        // Record latency
        if let Ok(mut tracker) = latency_tracker.lock() {
            tracker.record(frame.timestamps);
        }

        frame_count += 1;

        // Log stats every 5 seconds
        if frame_count % 450 == 0 {
            if let Ok(tracker) = latency_tracker.lock() {
                if let Some(avg) = tracker.avg_pc_latency_us() {
                    log::info!("PC latency: avg={}us encode={}us",
                        avg, tracker.avg_encode_latency_us().unwrap_or(0));
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::metrics::latency::FrameTimestamps;

    #[test]
    fn test_engine_creation() {
        let config = AppConfig::default();
        let engine = StreamingEngine::new(config);
        assert!(engine.is_ok());
        let engine = engine.unwrap();
        engine.shutdown();
    }

    #[test]
    fn test_get_tracking_none_initially() {
        let config = AppConfig::default();
        let engine = StreamingEngine::new(config).unwrap();
        assert!(engine.get_tracking().is_none());
        engine.shutdown();
    }

    #[test]
    fn test_get_controller_none_initially() {
        let config = AppConfig::default();
        let engine = StreamingEngine::new(config).unwrap();
        assert!(engine.get_controller(0).is_none());
        assert!(engine.get_controller(1).is_none());
        engine.shutdown();
    }

    #[test]
    fn test_get_controller_invalid_id() {
        let config = AppConfig::default();
        let engine = StreamingEngine::new(config).unwrap();
        assert!(engine.get_controller(2).is_none());
        assert!(engine.get_controller(255).is_none());
        engine.shutdown();
    }

    #[test]
    fn test_submit_frame_success() {
        let config = AppConfig::default();
        let engine = StreamingEngine::new(config).unwrap();
        let frame = EncodedFrame {
            frame_index: 0,
            nal_data: vec![0u8; 100],
            is_idr: true,
            timestamps: FrameTimestamps::new(0),
        };
        assert!(engine.submit_frame(frame));
        engine.shutdown();
    }

    #[test]
    fn test_submit_frame_channel_full() {
        let config = AppConfig::default();
        let engine = StreamingEngine::new(config).unwrap();
        // Channel capacity is 4, so 5th frame should fail
        for i in 0..4 {
            let frame = EncodedFrame {
                frame_index: i,
                nal_data: vec![0u8; 100],
                is_idr: i == 0,
                timestamps: FrameTimestamps::new(i),
            };
            assert!(engine.submit_frame(frame), "frame {} should succeed", i);
        }
        // 5th frame — channel full (receiver not consuming since TCP not connected)
        let frame = EncodedFrame {
            frame_index: 4,
            nal_data: vec![0u8; 100],
            is_idr: false,
            timestamps: FrameTimestamps::new(4),
        };
        assert!(!engine.submit_frame(frame), "frame 4 should fail (channel full)");
        engine.shutdown();
    }
}
