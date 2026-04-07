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
        self.socket.send_to(data, self.target).await
    }

    pub async fn send_all(&self, packets: &[super::rtp::RtpPacket]) -> std::io::Result<()> {
        for pkt in packets {
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
}
