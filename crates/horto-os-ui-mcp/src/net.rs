//! Local-network peer allowlist (loopback / RFC1918 / ULA / link-local).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// True for loopback, RFC1918, IPv6 ULA, and link-local peers.
#[must_use]
pub fn is_local_network_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_local_ipv4(v4),
        IpAddr::V6(v6) => is_local_ipv6(v6),
    }
}

const fn is_local_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
}

fn is_local_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unicast_link_local() {
        return true;
    }
    let octets = ip.octets();
    (octets[0] & 0xfe) == 0xfc || ip.to_ipv4_mapped().is_some_and(is_local_ipv4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_lan_rejects_public() {
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            192, 168, 0, 1
        ))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            172, 16, 0, 1
        ))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            169, 254, 1, 1
        ))));
        assert!(!is_local_network_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_local_network_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(is_local_network_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        let ula = Ipv6Addr::new(0xfd12, 0, 0, 0, 0, 0, 0, 1);
        assert!(is_local_network_ip(IpAddr::V6(ula)));
        let link = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
        assert!(is_local_network_ip(IpAddr::V6(link)));
        let global = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);
        assert!(!is_local_network_ip(IpAddr::V6(global)));
        let mapped_lan = Ipv4Addr::new(192, 168, 1, 5).to_ipv6_mapped();
        assert!(is_local_network_ip(IpAddr::V6(mapped_lan)));
        let mapped_pub = Ipv4Addr::new(8, 8, 8, 8).to_ipv6_mapped();
        assert!(!is_local_network_ip(IpAddr::V6(mapped_pub)));
    }
}
