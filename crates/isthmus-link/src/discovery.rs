//! Announcing this device and finding the others with mDNS.
//!
//! Each device registers `<id>._isthmus._tcp.local.` with its port and a TXT
//! record (`id`, `name`, `os`, `v`) on every interface, a bare cable
//! included, and browses for the others. The id doubles as the instance
//! name, so two devices with the same name never collide and a removal names
//! the id directly. An announcement grants nothing: whoever answers must
//! still prove the id's key on connect, and only a paired key is trusted.
//!
//! mdns-sd runs its own thread; a second thread here turns its events into
//! [`Sighting`]s for the service. Interfaces are re-read every few seconds,
//! so the self-assigned address a cable gets a few seconds after it is
//! plugged in is announced without a restart.
//!
//! Every [`REFRESH`] the browse is started again and this device announced
//! again. mdns-sd backs its queries off towards an hour and announces only
//! when it registers, so without this a query or announcement lost early
//! (before macOS's Local Network permission was granted, or before a cable
//! had its address) would leave two devices unaware of each other for a
//! long time. A repeated browse keeps mdns-sd's cache and restarts its
//! schedule; a repeated registration just announces again.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mdns_sd::{
    DaemonStatus, IfKind, RecvTimeoutError, ResolvedService, ScopedIp, ServiceDaemon, ServiceEvent,
    ServiceInfo,
};
use parking_lot::Mutex;
use snafu::ResultExt;
use tracing::{info, warn};

use crate::error::{DiscoverySnafu, Error};
use crate::identity::DeviceId;
use crate::store::PeerOs;
use crate::wire::PROTOCOL_VERSION;

/// The DNS-SD service type.
pub const SERVICE_TYPE: &str = "_isthmus._tcp.local.";

/// How often mdns-sd re-reads the interfaces, in seconds.
const INTERFACE_CHECK_SECONDS: u32 = 5;

/// How often the browse restarts and this device announces again: a few
/// packets a minute on the local network.
pub const REFRESH: Duration = Duration::from_secs(30);

/// What this device announces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    pub id: DeviceId,
    pub name: String,
    pub os: PeerOs,
    pub port: u16,
}

/// A device seen on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sighting {
    pub id: DeviceId,
    pub name: String,
    pub os: PeerOs,
    pub version: u32,
    /// Every address it answers on, with the index of the local interface
    /// that saw it when mDNS says.
    pub addrs: Vec<(SocketAddr, Option<u32>)>,
}

/// What discovery tells the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// A device appeared or its record changed.
    Seen(Sighting),
    /// A device said goodbye or its record expired.
    Gone(DeviceId),
}

/// The running announcement and browse.
pub struct Discovery {
    daemon: ServiceDaemon,
    /// What is announced now, which the refresh announces again.
    current: Arc<Mutex<ServiceInfo>>,
}

impl Discovery {
    /// Announces `me` and starts browsing; `sink` gets every event for
    /// another device, on discovery's own thread.
    pub fn start(
        me: &Announcement,
        sink: impl Fn(DiscoveryEvent) + Send + 'static,
    ) -> Result<Self, Error> {
        let daemon = ServiceDaemon::new().context(DiscoverySnafu)?;
        // Loopback answers would hand peers 127.0.0.1, which is never them.
        daemon
            .disable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])
            .context(DiscoverySnafu)?;
        daemon
            .set_ip_check_interval(INTERFACE_CHECK_SECONDS)
            .context(DiscoverySnafu)?;
        let info = service_info(me)?;
        daemon.register(info.clone()).context(DiscoverySnafu)?;
        let current = Arc::new(Mutex::new(info));
        let mut events = daemon.browse(SERVICE_TYPE).context(DiscoverySnafu)?;
        let own = me.id.clone();
        let (refresher, announced) = (daemon.clone(), current.clone());
        std::thread::Builder::new()
            .name("isthmus-discovery".into())
            .spawn(move || {
                report_state(&refresher);
                let mut next = Instant::now() + REFRESH;
                loop {
                    match events.recv_deadline(next) {
                        Ok(event) => {
                            if let Some(event) = translate(event, &own) {
                                sink(event);
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            next = Instant::now() + REFRESH;
                            // A daemon that has shut down refuses both, and
                            // the thread ends with it.
                            let info = announced.lock().clone();
                            let _ = refresher.register(info);
                            match refresher.browse(SERVICE_TYPE) {
                                Ok(fresh) => events = fresh,
                                Err(_) => break,
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .map_err(|source| Error::Runtime { source })?;
        Ok(Self { daemon, current })
    }

    /// Announces `me` again, after a rename: registering the same instance
    /// replaces its record.
    pub fn announce(&self, me: &Announcement) -> Result<(), Error> {
        let info = service_info(me)?;
        *self.current.lock() = info.clone();
        self.daemon.register(info).context(DiscoverySnafu)
    }

    /// Says goodbye on the network and stops.
    pub fn shutdown(&self, me: &DeviceId) {
        let _ = self.daemon.unregister(&fullname(me));
        let _ = self.daemon.shutdown();
    }
}

/// Logs what mdns-sd set up: whether its sockets are bound and which
/// interfaces joined the mDNS group. The first thing to read when two
/// devices do not see each other.
fn report_state(daemon: &ServiceDaemon) {
    let Ok(status) = daemon.status() else {
        return;
    };
    let Ok(DaemonStatus::Running(state)) = status.recv_timeout(Duration::from_secs(2)) else {
        return;
    };
    for (version, socket) in [("IPv4", state.ipv4_socket()), ("IPv6", state.ipv6_socket())] {
        if let Some(error) = socket.bind_error() {
            warn!(version, error, "the mDNS socket is not bound");
        }
    }
    for interface in state.enabled_intfs() {
        for membership in [interface.ipv4(), interface.ipv6()].into_iter().flatten() {
            match membership.join_error() {
                None => info!(
                    interface = interface.name(),
                    addrs = ?membership.addrs(),
                    "mDNS on this interface"
                ),
                Some(error) => warn!(
                    interface = interface.name(),
                    error, "mDNS could not join the group on this interface"
                ),
            }
        }
    }
}

fn fullname(id: &DeviceId) -> String {
    format!("{id}.{SERVICE_TYPE}")
}

fn service_info(me: &Announcement) -> Result<ServiceInfo, Error> {
    let version = PROTOCOL_VERSION.to_string();
    let os = os_text(me.os);
    let properties = [
        ("id", me.id.as_str()),
        ("name", me.name.as_str()),
        ("os", os),
        ("v", version.as_str()),
    ];
    let host = format!("isthmus-{}.local.", me.id);
    ServiceInfo::new(
        SERVICE_TYPE,
        me.id.as_str(),
        &host,
        "",
        me.port,
        &properties[..],
    )
    .map(ServiceInfo::enable_addr_auto)
    .context(DiscoverySnafu)
}

fn os_text(os: PeerOs) -> &'static str {
    match os {
        PeerOs::Macos => "macos",
        PeerOs::Windows => "windows",
        PeerOs::Linux => "linux",
        PeerOs::Other => "other",
    }
}

fn parse_os(text: Option<&str>) -> PeerOs {
    match text {
        Some("macos") => PeerOs::Macos,
        Some("windows") => PeerOs::Windows,
        Some("linux") => PeerOs::Linux,
        _ => PeerOs::Other,
    }
}

/// An mdns-sd event as one for the service, or `None` for this device's
/// own record and events that say nothing about a device.
fn translate(event: ServiceEvent, own: &DeviceId) -> Option<DiscoveryEvent> {
    match event {
        ServiceEvent::ServiceResolved(resolved) => sighting(&resolved)
            .filter(|seen| &seen.id != own)
            .map(DiscoveryEvent::Seen),
        ServiceEvent::ServiceRemoved(_, fullname) => {
            let instance = fullname.strip_suffix(&format!(".{SERVICE_TYPE}"))?;
            let id = DeviceId::try_from(instance.to_owned()).ok()?;
            (&id != own).then_some(DiscoveryEvent::Gone(id))
        }
        ServiceEvent::SearchStarted(_)
        | ServiceEvent::ServiceFound(..)
        | ServiceEvent::SearchStopped(_) => None,
        // mdns-sd marks its events non-exhaustive.
        _ => None,
    }
}

/// A resolved record as a sighting, or `None` when it is not a well-formed
/// Isthmus record (an id that is not one, or one that is not the instance
/// name).
pub fn sighting(resolved: &ResolvedService) -> Option<Sighting> {
    let id = DeviceId::try_from(
        resolved
            .txt_properties
            .get_property_val_str("id")?
            .to_owned(),
    )
    .ok()?;
    let instance = resolved
        .fullname
        .strip_suffix(&format!(".{SERVICE_TYPE}"))?;
    if instance != id.as_str() {
        return None;
    }
    let name = resolved
        .txt_properties
        .get_property_val_str("name")
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(id.as_str())
        .to_owned();
    let os = parse_os(resolved.txt_properties.get_property_val_str("os"));
    let version = resolved
        .txt_properties
        .get_property_val_str("v")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut addrs = Vec::new();
    for scoped in &resolved.addresses {
        match scoped {
            ScopedIp::V4(v4) => {
                let addr = SocketAddr::new(IpAddr::V4(*v4.addr()), resolved.port);
                if v4.interface_ids().is_empty() {
                    addrs.push((addr, None));
                }
                for interface in v4.interface_ids() {
                    addrs.push((addr, Some(interface.index)));
                }
            }
            ScopedIp::V6(v6) => {
                let addr = SocketAddr::new(IpAddr::V6(*v6.addr()), resolved.port);
                addrs.push((addr, Some(v6.scope_id().index)));
            }
            // mdns-sd marks the address kinds non-exhaustive.
            _ => {}
        }
    }
    Some(Sighting {
        id,
        name,
        os,
        version,
        addrs,
    })
}
