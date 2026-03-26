use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::config::AppConfig;
use crate::control::tcp_server::TcpControlServer;
use crate::metrics::latency::{FrameTimestamps, LatencyTracker};
use crate::pipeline;
use crate::transport::rtp::RtpPacketizer;
use crate::transport::udp::UdpSender;
use fvp_common::protocol::TrackingData;

/// Frame data submitted from the C++ OpenVR driver.
pub struct SubmittedFrame {
    pub frame_index: u32,
    pub data: Vec<u8>, // Raw pixel data or pre-encoded NAL units
    pub width: u32,
    pub height: u32,
    pub timestamps: FrameTimestamps,
}

/// The main streaming engine running on a tokio runtime.
pub struct StreamingEngine {
    runtime: Runtime,
    frame_tx: mpsc::Sender<SubmittedFrame>,
    latest_tracking: Arc<StdMutex<Option<TrackingData>>>,
    latency_tracker: Arc<StdMutex<LatencyTracker>>,
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

        let (frame_tx, frame_rx) = mpsc::channel::<SubmittedFrame>(4); // Small buffer to avoid latency
        let latest_tracking = Arc::new(StdMutex::new(None));
        let latency_tracker = Arc::new(StdMutex::new(LatencyTracker::new(90))); // 1 second at 90fps

        let tracking_clone = latest_tracking.clone();
        let tracker_clone = latency_tracker.clone();
        let config_clone = config.clone();

        // Spawn the main streaming task
        runtime.spawn(async move {
            if let Err(e) = run_streaming(config_clone, frame_rx, tracking_clone, tracker_clone).await {
                log::error!("Streaming engine error: {}", e);
            }
        });

        Ok(Self {
            runtime,
            frame_tx,
            latest_tracking,
            latency_tracker,
            config,
        })
    }

    /// Submit a frame for encoding and sending. Called from C++ thread.
    pub fn submit_frame(&self, frame: SubmittedFrame) -> bool {
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
        self.latest_tracking.lock().ok()?.clone()
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
    mut frame_rx: mpsc::Receiver<SubmittedFrame>,
    tracking: Arc<StdMutex<Option<TrackingData>>>,
    latency_tracker: Arc<StdMutex<LatencyTracker>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Step 1: Wait for HMD to connect via TCP
    let tcp_server = TcpControlServer::new(config.clone());
    let _tcp_stream = tcp_server.listen_and_accept().await?;
    log::info!("HMD connected, starting video stream");

    // Step 2: Create UDP sender for video
    let udp_target: SocketAddr = format!("0.0.0.0:{}", config.network.udp_port + 1)
        .parse()
        .unwrap(); // Will be replaced with actual HMD IP
    let udp_sender = UdpSender::new(udp_target).await?;

    // Step 3: Process frames
    let mut packetizer = RtpPacketizer::new(0x46565000); // "FVP\0"
    let mut frame_count: u64 = 0;

    while let Some(mut frame) = frame_rx.recv().await {
        frame.timestamps.mark_encode_start();

        // In production: encode with NVENC via FFmpeg
        // For now: treat frame.data as pre-encoded NAL units
        let encoded_data = &frame.data;

        frame.timestamps.mark_encode_end();

        // Packetize with FEC
        let is_keyframe = frame_count % 90 == 0; // IDR every ~1 second
        let timestamp_90khz = (frame_count * (fvp_common::RTP_CLOCK_RATE as u64 / 90)) as u32;

        let packets = pipeline::encode_frame_to_packets(
            encoded_data,
            frame.frame_index,
            timestamp_90khz,
            is_keyframe,
            config.network.fec_redundancy,
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
