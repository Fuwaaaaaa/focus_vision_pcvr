use std::net::SocketAddr;
use tokio::net::UdpSocket;

/// Sends RTP packets over UDP.
pub struct UdpSender {
    socket: UdpSocket,
    target: SocketAddr,
}

impl UdpSender {
    pub async fn new(target: SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
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
