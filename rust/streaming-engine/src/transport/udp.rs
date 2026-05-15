use std::net::SocketAddr;
use tokio::net::UdpSocket;

/// Default UDP send buffer size (2MB — large enough for burst video frames).
const DEFAULT_SEND_BUF: u32 = 2 * 1024 * 1024;
/// Default UDP receive buffer size (2MB).
const DEFAULT_RECV_BUF: u32 = 2 * 1024 * 1024;
/// DSCP value for Expedited Forwarding (EF) — best-effort QoS marking.
/// Many routers ignore this, but it's free to set.
const DSCP_EF: u32 = 46 << 2; // 0xB8

/// Apply socket optimizations. Failures are logged but not fatal.
fn apply_socket_opts(socket: &UdpSocket, send_buf: Option<u32>, recv_buf: Option<u32>) {
    use std::os::windows::io::AsRawSocket;
    let raw = socket.as_raw_socket();

    // SO_SNDBUF
    if let Some(size) = send_buf {
        let ret = unsafe {
            libc_setsockopt(raw as usize, SOL_SOCKET, SO_SNDBUF, &size as *const u32 as *const _, 4)
        };
        if ret != 0 {
            log::warn!("setsockopt SO_SNDBUF failed (non-fatal)");
        }
    }

    // SO_RCVBUF
    if let Some(size) = recv_buf {
        let ret = unsafe {
            libc_setsockopt(raw as usize, SOL_SOCKET, SO_RCVBUF, &size as *const u32 as *const _, 4)
        };
        if ret != 0 {
            log::warn!("setsockopt SO_RCVBUF failed (non-fatal)");
        }
    }

    // DSCP / TOS marking
    let tos = DSCP_EF;
    let ret = unsafe {
        libc_setsockopt(raw as usize, IPPROTO_IP, IP_TOS, &tos as *const u32 as *const _, 4)
    };
    if ret != 0 {
        log::debug!("setsockopt IP_TOS (DSCP) failed (non-fatal, many routers ignore)");
    }
}

// Windows socket constants
const SOL_SOCKET: i32 = 0xFFFF;
const SO_SNDBUF: i32 = 0x1001;
const SO_RCVBUF: i32 = 0x1002;
const IPPROTO_IP: i32 = 0;
const IP_TOS: i32 = 3;

extern "system" {
    fn setsockopt(s: usize, level: i32, optname: i32, optval: *const u8, optlen: i32) -> i32;
}

unsafe fn libc_setsockopt(s: usize, level: i32, optname: i32, optval: *const u8, optlen: i32) -> i32 {
    unsafe { setsockopt(s, level, optname, optval, optlen) }
}

/// Simulator-only knob: probability (0..=100) that each outbound packet is
/// silently dropped before the syscall. Used by scenario tests to inject
/// packet loss on the engine→HMD video path so the adaptive bitrate
/// controller (and other loss-sensitive code) get exercised in CI without
/// a real flaky network. Production builds compile this out completely.
#[cfg(feature = "simulator")]
pub static SIMULATOR_LOSS_PCT: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(0);

/// Test-only: set the simulator drop probability. Values are clamped to
/// 0..=100. Pass 0 to disable. No-op in production builds.
#[cfg(feature = "simulator")]
pub fn set_simulator_loss_pct(pct: u8) {
    SIMULATOR_LOSS_PCT.store(pct.min(100), std::sync::atomic::Ordering::Relaxed);
}

/// Test-only: read the current loss probability.
#[cfg(feature = "simulator")]
pub fn simulator_loss_pct() -> u8 {
    SIMULATOR_LOSS_PCT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Decide whether to drop the current packet. Compiled out of production.
#[inline(always)]
fn should_drop_packet() -> bool {
    #[cfg(feature = "simulator")]
    {
        let pct = SIMULATOR_LOSS_PCT.load(std::sync::atomic::Ordering::Relaxed);
        if pct > 0 {
            // `rand::random` is already a dependency; using % keeps the
            // distribution good enough for a coarse loss simulator.
            return rand::random::<u8>() % 100 < pct;
        }
    }
    false
}

/// Sends RTP packets over UDP.
pub struct UdpSender {
    socket: UdpSocket,
    target: SocketAddr,
}

impl UdpSender {
    pub async fn new(target: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        apply_socket_opts(&socket, Some(DEFAULT_SEND_BUF), None);
        Ok(Self { socket, target })
    }

    pub async fn send(&self, data: &[u8]) -> std::io::Result<usize> {
        if should_drop_packet() {
            return Ok(data.len()); // pretend the send succeeded
        }
        self.socket.send_to(data, self.target).await
    }

    pub async fn send_all(&self, packets: &[super::rtp::RtpPacket]) -> std::io::Result<()> {
        for pkt in packets {
            if should_drop_packet() {
                continue;
            }
            self.socket.send_to(&pkt.data, self.target).await?;
        }
        Ok(())
    }
}

/// Receives UDP packets.
pub struct UdpReceiver {
    socket: UdpSocket,
}

impl UdpReceiver {
    pub async fn new(bind_addr: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(bind_addr).await?;
        apply_socket_opts(&socket, None, Some(DEFAULT_RECV_BUF));
        Ok(Self { socket })
    }

    /// Receive a single packet. Returns (data, sender address).
    pub async fn recv(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        self.socket.recv_from(buf).await
    }

    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_udp_send_recv() {
        let receiver = UdpReceiver::new("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let recv_addr = receiver.local_addr().unwrap();

        let sender = UdpSender::new(recv_addr).await.unwrap();

        let payload = b"hello focus vision";
        sender.send(payload).await.unwrap();

        let mut buf = [0u8; 1500];
        let (len, _from) = receiver.recv(&mut buf).await.unwrap();
        assert_eq!(&buf[..len], payload);
    }

    #[tokio::test]
    async fn test_udp_multiple_packets() {
        let receiver = UdpReceiver::new("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let recv_addr = receiver.local_addr().unwrap();
        let sender = UdpSender::new(recv_addr).await.unwrap();

        for i in 0..10u8 {
            sender.send(&[i; 100]).await.unwrap();
        }

        let mut buf = [0u8; 1500];
        for i in 0..10u8 {
            let (len, _) = receiver.recv(&mut buf).await.unwrap();
            assert_eq!(len, 100);
            assert_eq!(buf[0], i);
        }
    }

    #[cfg(feature = "simulator")]
    #[tokio::test]
    async fn test_simulator_loss_drops_packets() {
        // Bind sender + receiver, set 100% loss, confirm zero packets reach
        // the receiver. Reset to 0 at the end so other tests aren't poisoned.
        let receiver = UdpReceiver::new("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let recv_addr = receiver.local_addr().unwrap();
        let sender = UdpSender::new(recv_addr).await.unwrap();

        set_simulator_loss_pct(100);
        assert_eq!(simulator_loss_pct(), 100);

        for _ in 0..32 {
            sender.send(&[0xAB; 64]).await.unwrap();
        }

        // Pure 100% drop ⇒ recv_from with a small timeout must time out.
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            receiver.recv(&mut [0u8; 64]),
        ).await;
        set_simulator_loss_pct(0); // reset before assert so failure doesn't poison
        assert!(result.is_err(), "expected no packets through 100% loss, got one");
    }

    #[cfg(feature = "simulator")]
    #[tokio::test]
    async fn test_simulator_loss_zero_passes_through() {
        // With loss=0 the sender behaves exactly like the un-gated path.
        set_simulator_loss_pct(0);
        let receiver = UdpReceiver::new("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let sender = UdpSender::new(receiver.local_addr().unwrap()).await.unwrap();

        sender.send(b"healthy").await.unwrap();
        let mut buf = [0u8; 32];
        let (n, _) = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            receiver.recv(&mut buf),
        ).await.unwrap().unwrap();
        assert_eq!(&buf[..n], b"healthy");
    }
}
