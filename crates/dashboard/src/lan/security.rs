//! LAN handoff security helpers.

use std::net::IpAddr;

/// True for RFC1918, link-local, and loopback (dev).
#[must_use]
pub fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.octets()[0] == 169 && v4.octets()[1] == 254
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local(),
    }
}

/// Resolve peer IP for LAN ACL.
///
/// **Never trust `X-Forwarded-For` / `X-Real-IP` from the client** — the LAN
/// listener binds on the LAN interface and any peer can spoof those headers.
/// Only the TCP peer address (`remote`) is authoritative.
#[must_use]
pub fn peer_addr_from_headers(
    _forwarded: Option<&str>,
    _real_ip: Option<&str>,
    remote: Option<std::net::SocketAddr>,
) -> Option<IpAddr> {
    remote.map(|a| a.ip())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

    #[test]
    fn private_ipv4_detected() {
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10))));
        assert!(is_private_ip(&IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(!is_private_ip(&IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[test]
    fn ignores_spoofed_forwarded_headers() {
        let remote = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(8, 8, 8, 8), 9));
        let ip = peer_addr_from_headers(Some("192.168.1.50"), Some("10.0.0.1"), Some(remote));
        assert_eq!(ip, Some(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }
}
