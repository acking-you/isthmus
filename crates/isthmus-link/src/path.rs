//! Which way to reach a peer: every address it answers on, ranked.
//!
//! Two kinds of path, in order: **direct**, a cable straight to the other
//! computer, and **LAN**, the network both are on. A direct cable is told
//! apart by its addresses: nothing hands out addresses on a bare cable, so
//! both ends assign themselves one in 169.254/16 (macOS and Windows alike),
//! while any network with a router has DHCP or manual addresses. A KVM
//! switch carries no data between its computers, but a USB Ethernet adapter
//! on its hub, cabled to the other computer's Ethernet port, is exactly such
//! a cable, and is gigabit where Wi-Fi rarely is.
//!
//! The ranking is plain data so it is tested on any host; reading the
//! interfaces is the one call into the system.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};

use serde::{Deserialize, Serialize};

/// How a peer is reached, best first: the derived order is the ranking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathKind {
    /// A cable straight to the other computer.
    Direct,
    /// The local network both computers are on, wired or Wi-Fi.
    Lan,
}

/// One of this computer's network interfaces, with its addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalInterface {
    pub name: String,
    pub index: u32,
    /// IPv4 addresses and their prefix lengths.
    pub v4: Vec<(Ipv4Addr, u8)>,
    /// IPv6 addresses and their prefix lengths.
    pub v6: Vec<(Ipv6Addr, u8)>,
}

impl LocalInterface {
    /// Whether this is a bare cable to another computer: it has IPv4 and
    /// every IPv4 address is self-assigned. An interface with IPv6 alone is
    /// not counted (tunnels and Apple's peer-to-peer links look like that).
    pub fn is_direct(&self) -> bool {
        !self.v4.is_empty() && self.v4.iter().all(|(ip, _)| is_self_assigned(*ip))
    }

    /// The kind of path through this interface.
    pub fn kind(&self) -> PathKind {
        if self.is_direct() {
            PathKind::Direct
        } else {
            PathKind::Lan
        }
    }

    /// Whether `ip` is on one of this interface's subnets.
    pub fn reaches(&self, ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(ip) => self
                .v4
                .iter()
                .any(|(own, prefix)| same_prefix(own.to_bits(), ip.to_bits(), *prefix, 32)),
            IpAddr::V6(ip) => self
                .v6
                .iter()
                .any(|(own, prefix)| same_prefix(own.to_bits(), ip.to_bits(), *prefix, 128)),
        }
    }

    /// Whether `ip` is one of this interface's own addresses.
    pub fn owns(&self, ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(ip) => self.v4.iter().any(|(own, _)| *own == ip),
            IpAddr::V6(ip) => self.v6.iter().any(|(own, _)| *own == ip),
        }
    }

    /// The first IPv4 address, for binding a socket on systems that pick
    /// the interface by source address.
    pub fn first_v4(&self) -> Option<Ipv4Addr> {
        self.v4.first().map(|(ip, _)| *ip)
    }
}

/// Whether the top `prefix` bits of `a` and `b` agree, on `width`-bit
/// addresses.
fn same_prefix<T>(a: T, b: T, prefix: u8, width: u32) -> bool
where
    T: Copy + std::ops::BitXor<Output = T> + std::ops::Shr<u32, Output = T> + PartialEq + Default,
{
    let prefix = u32::from(prefix).min(width);
    if prefix == 0 {
        return true;
    }
    (a ^ b) >> (width - prefix) == T::default()
}

/// 169.254/16, which a computer gives itself when nothing else does.
pub fn is_self_assigned(ip: Ipv4Addr) -> bool {
    ip.is_link_local()
}

/// Whether an IPv6 address is link-local (fe80::/10) and so needs a scope.
pub fn is_v6_link_local(ip: Ipv6Addr) -> bool {
    ip.is_unicast_link_local()
}

/// One way to try reaching a peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub addr: SocketAddr,
    pub kind: PathKind,
    /// The local interface it goes through, when known.
    pub interface: Option<String>,
    /// That interface's index, for binding the socket to it.
    pub interface_index: Option<u32>,
}

/// Classifies `addr`, which the peer answers on, seen through the local
/// interface `via` (its index) when mDNS said which. Without `via`, the
/// interface whose subnet holds the address decides; with none, a
/// self-assigned address is still a cable and anything else is the LAN.
/// An IPv6 link-local address gets the interface as its scope, and without
/// one it cannot be dialled, so it is dropped.
pub fn classify(
    addr: SocketAddr,
    via: Option<u32>,
    locals: &[LocalInterface],
) -> Option<Candidate> {
    let ip = addr.ip();
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return None;
    }
    let interface = via
        .and_then(|index| locals.iter().find(|local| local.index == index))
        .or_else(|| locals.iter().find(|local| local.reaches(ip)));
    let kind = match (interface, ip) {
        (Some(local), _) => local.kind(),
        (None, IpAddr::V4(v4)) if is_self_assigned(v4) => PathKind::Direct,
        (None, _) => PathKind::Lan,
    };
    let addr = match (addr, interface) {
        (SocketAddr::V6(v6), Some(local)) if is_v6_link_local(*v6.ip()) => {
            SocketAddr::V6(SocketAddrV6::new(*v6.ip(), v6.port(), 0, local.index))
        }
        (SocketAddr::V6(v6), None) if is_v6_link_local(*v6.ip()) && v6.scope_id() == 0 => {
            return None;
        }
        (addr, _) => addr,
    };
    Some(Candidate {
        addr,
        kind,
        interface: interface.map(|local| local.name.clone()),
        interface_index: interface.map(|local| local.index),
    })
}

/// Sorts candidates best first (direct before LAN, IPv4 before IPv6) and
/// drops repeats of an address, keeping its best entry.
pub fn rank(candidates: &mut Vec<Candidate>) {
    candidates.sort_by_key(|candidate| (candidate.kind, !candidate.addr.is_ipv4()));
    let mut seen = Vec::with_capacity(candidates.len());
    candidates.retain(|candidate| {
        let fresh = !seen.contains(&candidate.addr);
        if fresh {
            seen.push(candidate.addr);
        }
        fresh
    });
}

/// The kind of a connection already made, from the local address it uses:
/// the interface owning that address decides.
pub fn kind_of_local(
    local: IpAddr,
    remote: IpAddr,
    locals: &[LocalInterface],
) -> (PathKind, Option<String>) {
    let interface = locals
        .iter()
        .find(|interface| interface.owns(local))
        .or_else(|| locals.iter().find(|interface| interface.reaches(remote)));
    match interface {
        Some(interface) => (interface.kind(), Some(interface.name.clone())),
        None => match remote {
            IpAddr::V4(v4) if is_self_assigned(v4) => (PathKind::Direct, None),
            _ => (PathKind::Lan, None),
        },
    }
}

/// This computer's interfaces that are up, without loopback, grouped by
/// index. Empty if the system will not say.
pub fn local_interfaces() -> Vec<LocalInterface> {
    let Ok(addresses) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut interfaces: Vec<LocalInterface> = Vec::new();
    for address in addresses {
        if address.is_loopback() || address.oper_status == if_addrs::IfOperStatus::Down {
            continue;
        }
        let Some(index) = address.index else {
            continue;
        };
        let position = match interfaces.iter().position(|known| known.index == index) {
            Some(position) => position,
            None => {
                interfaces.push(LocalInterface {
                    name: address.name.clone(),
                    index,
                    v4: Vec::new(),
                    v6: Vec::new(),
                });
                interfaces.len() - 1
            }
        };
        let Some(interface) = interfaces.get_mut(position) else {
            continue;
        };
        match address.addr {
            if_addrs::IfAddr::V4(v4) => interface.v4.push((v4.ip, v4.prefixlen)),
            if_addrs::IfAddr::V6(v6) => interface.v6.push((v6.ip, v6.prefixlen)),
        }
    }
    interfaces
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The desk this was built for: Wi-Fi on a home network, and a USB
    /// Ethernet adapter on the KVM's hub cabled to the PC, self-assigned.
    fn desk() -> Vec<LocalInterface> {
        vec![
            LocalInterface {
                name: "en1".into(),
                index: 11,
                v4: vec![(Ipv4Addr::new(192, 168, 21, 37), 24)],
                v6: vec![("fe80::1c".parse().unwrap(), 64)],
            },
            LocalInterface {
                name: "en8".into(),
                index: 23,
                v4: vec![(Ipv4Addr::new(169, 254, 149, 93), 16)],
                v6: vec![("fe80::8".parse().unwrap(), 64)],
            },
        ]
    }

    fn addr(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    /// The cable is direct and the home network is the LAN, whether mDNS
    /// said which interface saw the address or the subnet has to tell.
    #[test]
    fn a_self_assigned_cable_is_direct_and_the_home_network_is_lan() {
        let locals = desk();
        let cable = classify(addr("169.254.10.20:47821"), Some(23), &locals).unwrap();
        assert_eq!(cable.kind, PathKind::Direct);
        assert_eq!(cable.interface.as_deref(), Some("en8"));
        let wifi = classify(addr("192.168.21.50:47821"), None, &locals).unwrap();
        assert_eq!(wifi.kind, PathKind::Lan);
        assert_eq!(wifi.interface.as_deref(), Some("en1"));
        let routed = classify(addr("10.1.2.3:47821"), None, &locals).unwrap();
        assert_eq!(routed.kind, PathKind::Lan);
        assert!(routed.interface.is_none());
    }

    /// An IPv6 link-local address takes the scope of the interface that saw
    /// it (fe80:: exists on every interface, so the interface decides the
    /// kind), and is dropped when no interface is known.
    #[test]
    fn ipv6_link_local_takes_the_interface_as_its_scope() {
        let locals = desk();
        let cable = classify(addr("[fe80::42]:47821"), Some(23), &locals).unwrap();
        assert_eq!(cable.kind, PathKind::Direct);
        match cable.addr {
            SocketAddr::V6(v6) => assert_eq!(v6.scope_id(), 23),
            SocketAddr::V4(_) => panic!("expected IPv6"),
        }
        let wifi = classify(addr("[fe80::42]:47821"), Some(11), &locals).unwrap();
        assert_eq!(wifi.kind, PathKind::Lan);
        assert!(classify(addr("[fe80::42]:47821"), None, &[]).is_none());
    }

    /// Loopback and other addresses no peer answers on are never tried.
    #[test]
    fn loopback_is_never_a_candidate() {
        assert!(classify(addr("127.0.0.1:1"), None, &desk()).is_none());
        assert!(classify(addr("0.0.0.0:1"), None, &desk()).is_none());
    }

    /// Direct comes before LAN and IPv4 before IPv6, each address once.
    #[test]
    fn ranking_puts_the_cable_first_and_drops_repeats() {
        let locals = desk();
        let mut candidates: Vec<Candidate> = [
            ("192.168.21.50:47821", None),
            ("[fe80::42]:47821", Some(23)),
            ("169.254.10.20:47821", Some(23)),
            ("192.168.21.50:47821", Some(11)),
        ]
        .into_iter()
        .filter_map(|(text, via)| classify(addr(text), via, &locals))
        .collect();
        rank(&mut candidates);
        let order: Vec<String> = candidates.iter().map(|c| c.addr.to_string()).collect();
        assert_eq!(
            order,
            vec![
                "169.254.10.20:47821",
                "[fe80::42%23]:47821",
                "192.168.21.50:47821"
            ]
        );
    }

    /// An interface with a router-given address is never a cable, even if
    /// it also has a self-assigned one; one with IPv6 alone is not either.
    #[test]
    fn only_all_self_assigned_ipv4_makes_a_cable() {
        let mixed = LocalInterface {
            name: "en0".into(),
            index: 4,
            v4: vec![
                (Ipv4Addr::new(169, 254, 3, 4), 16),
                (Ipv4Addr::new(10, 0, 0, 2), 24),
            ],
            v6: Vec::new(),
        };
        assert!(!mixed.is_direct());
        let tunnel = LocalInterface {
            name: "utun3".into(),
            index: 30,
            v4: Vec::new(),
            v6: vec![("fe80::9".parse().unwrap(), 64)],
        };
        assert!(!tunnel.is_direct());
    }

    /// A connection's kind follows the interface owning its local address.
    #[test]
    fn a_connection_is_classified_by_its_local_address() {
        let locals = desk();
        let (kind, name) = kind_of_local(
            "169.254.149.93".parse().unwrap(),
            "169.254.10.20".parse().unwrap(),
            &locals,
        );
        assert_eq!((kind, name.as_deref()), (PathKind::Direct, Some("en8")));
        let (kind, _) = kind_of_local(
            "192.168.21.37".parse().unwrap(),
            "192.168.21.50".parse().unwrap(),
            &locals,
        );
        assert_eq!(kind, PathKind::Lan);
    }
}
