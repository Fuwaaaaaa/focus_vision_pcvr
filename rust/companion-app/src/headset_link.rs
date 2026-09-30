//! Tell the headset app where to connect. Over adb (USB), the companion
//! starts the client with this PC's address on the headset's network and the
//! engine's pairing PIN; the client's MainActivity takes them from the
//! intent extras (client/app/src/main/cpp/launch_request.h). If the app is
//! already running, the same launch reaches it as a new intent and replaces
//! its connection.

use std::net::{IpAddr, Ipv4Addr, SocketAddrV4, UdpSocket};
use crate::adb;

pub const CLIENT_PACKAGE: &str = "com.focusvision.pcvr";

/// (control TCP port, UDP base port) when the engine publishes none.
pub const DEFAULT_PORTS: (u16, u16) = (fvp_common::DEFAULT_TCP_PORT, fvp_common::DEFAULT_UDP_PORT);

/// A PIN the engine is waiting for: six digits (status.json shows dashes
/// while there is none).
pub fn is_pin(pin: &str) -> bool {
    pin.len() == 6 && pin.bytes().all(|b| b.is_ascii_digit())
}

/// The headset's Wi-Fi IPv4 address from `ip -f inet addr show wlan0`:
/// the `inet 192.168.1.23/24 ...` line.
pub fn parse_wlan_ipv4(output: &str) -> Option<Ipv4Addr> {
    output
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("inet "))
        .and_then(|rest| rest.split(['/', ' ']).next())
        .and_then(|ip| ip.parse().ok())
}

/// The address this PC uses to reach `target`, chosen by the OS routing
/// table — so it is the one on the headset's network. Connecting a UDP
/// socket sends nothing.
pub fn local_address_toward(target: Ipv4Addr) -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((target, 9)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

/// adb arguments that start the client on `serial` with the pairing extras.
pub fn launch_args(serial: &str, server: SocketAddrV4, pin: &str, udp_port: u16) -> Vec<String> {
    [
        "-s", serial, "shell", "am", "start",
        "-n", &format!("{CLIENT_PACKAGE}/.MainActivity"),
        "--es", "fvp_server", &server.to_string(),
        "--es", "fvp_pin", pin,
        "--es", "fvp_udp_port", &udp_port.to_string(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Start the client on the headset `serial`, pointed at this PC with `pin`.
/// Runs adb twice (blocking): call it off the UI thread.
pub fn send_to_headset(
    adb_path: &str,
    serial: &str,
    pin: &str,
    (tcp_port, udp_port): (u16, u16),
) -> Result<String, String> {
    if !is_pin(pin) {
        return Err("No PIN to send yet: start SteamVR first.".to_string());
    }
    let output = adb::run(adb_path, &["-s", serial, "shell", "ip", "-f", "inet", "addr", "show", "wlan0"], adb::QUICK)?;
    let headset_ip = parse_wlan_ipv4(&String::from_utf8_lossy(&output.stdout)).ok_or_else(|| {
        "The headset has no Wi-Fi address. Connect it to the same network as this PC.".to_string()
    })?;
    let pc_ip = local_address_toward(headset_ip)
        .ok_or_else(|| format!("This PC has no route to the headset ({headset_ip})."))?;
    let server = SocketAddrV4::new(pc_ip, tcp_port);

    let args = launch_args(serial, server, pin, udp_port);
    let output = adb::run(adb_path, &args.iter().map(String::as_str).collect::<Vec<_>>(), adb::QUICK)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // `am start` reports a missing app on stdout with exit status 0.
    if !output.status.success() || stdout.contains("Error") || stderr.contains("Error") {
        return Err(format!("Could not start the headset app: {}{}", stdout.trim(), stderr.trim()));
    }
    Ok(format!("Sent to the headset: connect to {server} with PIN {pin}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_wifi_address_from_ip_output() {
        let output = "\
30: wlan0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc mq state UP group default qlen 3000
    inet 192.168.1.23/24 brd 192.168.1.255 scope global wlan0
       valid_lft forever preferred_lft forever
";
        assert_eq!(parse_wlan_ipv4(output), Some(Ipv4Addr::new(192, 168, 1, 23)));
    }

    #[test]
    fn no_wifi_address_is_none() {
        assert_eq!(parse_wlan_ipv4(""), None);
        assert_eq!(parse_wlan_ipv4("Device \"wlan0\" does not exist.\n"), None);
    }

    #[test]
    fn pins_are_six_digits() {
        assert!(is_pin("012345"));
        assert!(!is_pin("------"));
        assert!(!is_pin("----"));
        assert!(!is_pin("12345"));
        assert!(!is_pin("12345a"));
    }

    #[test]
    fn launch_carries_the_extras_the_client_reads() {
        let args = launch_args("HT123", SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 5), 9944), "012345", 9945);
        assert_eq!(
            args,
            [
                "-s", "HT123", "shell", "am", "start", "-n", "com.focusvision.pcvr/.MainActivity",
                "--es", "fvp_server", "10.0.0.5:9944",
                "--es", "fvp_pin", "012345",
                "--es", "fvp_udp_port", "9945",
            ]
        );
    }

    #[test]
    fn local_address_toward_loopback_is_loopback() {
        assert_eq!(local_address_toward(Ipv4Addr::LOCALHOST), Some(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn default_ports_match_the_engine() {
        assert_eq!(DEFAULT_PORTS, (9944, 9945));
    }
}
