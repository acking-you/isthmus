//! Reaching a peer: racing its candidate addresses best first, then the
//! handshake and the hello that say who answered and why we came.
//!
//! Candidates start a quarter second apart in rank order and the first to
//! connect wins, the rest are dropped. A working cable connects in a
//! millisecond, long before the LAN attempt starts, so it wins whenever it
//! works; a cable that is unplugged or whose other end is off costs the
//! LAN a quarter second, not a timeout.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::{TcpSocket, TcpStream};
use tokio::task::JoinSet;

use super::Core;
use crate::identity::DeviceId;
use crate::path::{Candidate, LocalInterface, PathKind};
use crate::platform;
use crate::store::PeerOs;
use crate::wire::{self, Message, Purpose, Refusal, Remote, SecureReader, SecureWriter, WireError};

/// How long after one candidate the next one starts.
const STAGGER: Duration = Duration::from_millis(250);

/// How long one connect attempt may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// How long the answerer has to reply to a hello.
pub(super) const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// The halves of a connection over TCP.
pub(super) type Reader = SecureReader<tokio::io::ReadHalf<TcpStream>>;
pub(super) type Writer = SecureWriter<tokio::io::WriteHalf<TcpStream>>;

/// A connection to a peer that accepted our hello.
pub(super) struct Opened {
    pub reader: Reader,
    pub writer: Writer,
    pub remote: Remote,
    /// The path it took and the interface, from the local address.
    pub kind: PathKind,
    pub interface: Option<String>,
    /// Where the peer listens (the candidate that won).
    pub addr: SocketAddr,
    /// The peer's name and system from its welcome.
    pub name: String,
    pub os: PeerOs,
    /// The protocol version its welcome said it speaks.
    pub version: u32,
}

/// Why a connection could not be opened.
#[derive(Debug)]
pub(super) enum OpenError {
    /// No candidate connected.
    Unreachable,
    /// Whoever answered does not hold the key we expected.
    Mismatch,
    /// The peer refused the hello.
    Refused(Refusal),
    /// The connection failed after it was made.
    Lost(WireError),
}

/// Which of a peer's paths a connection may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Paths {
    /// Every path, best first.
    Any,
    /// That kind alone (moving a LAN session to the cable).
    Only(PathKind),
    /// Every other kind first, then that one: the path that just stalled
    /// is tried last, though it may still be the only one.
    AvoidFirst(PathKind),
}

/// Opens a connection to `peer` for `purpose` over the candidates `paths`
/// allows. The key is checked against the pinned one, or for a pairing
/// against the id (an unpaired device has no pin yet).
pub(super) async fn open(
    core: &Arc<Core>,
    peer: &DeviceId,
    purpose: Purpose,
    paths: Paths,
) -> Result<Opened, OpenError> {
    let mut candidates = core.candidates(peer);
    match paths {
        Paths::Any => {}
        Paths::Only(only) => candidates.retain(|candidate| candidate.kind == only),
        // A stable sort keeps the rank within each group.
        Paths::AvoidFirst(avoid) => candidates.sort_by_key(|candidate| candidate.kind == avoid),
    }
    let locals = core.locals();
    let (stream, won) = race(candidates, locals)
        .await
        .ok_or(OpenError::Unreachable)?;
    let (kind, interface) = match (stream.local_addr(), stream.peer_addr()) {
        (Ok(local), Ok(remote)) => core.path_of(local, remote),
        _ => (won.kind, won.interface.clone()),
    };
    let (mut reader, mut writer, remote) = wire::initiate(stream, &core.identity)
        .await
        .map_err(OpenError::Lost)?;
    let expected = match purpose {
        Purpose::Pair => DeviceId::of(&remote.public_key) == *peer,
        Purpose::Session | Purpose::Transfer | Purpose::Unknown => {
            core.pinned_key(peer) == Some(remote.public_key)
        }
    };
    if !expected {
        writer.close().await;
        return Err(OpenError::Mismatch);
    }
    writer
        .send(&core.hello(purpose))
        .await
        .map_err(OpenError::Lost)?;
    match reader.message_within(HELLO_TIMEOUT).await {
        Ok(Message::Welcome {
            name, os, version, ..
        }) => Ok(Opened {
            reader,
            writer,
            remote,
            kind,
            interface,
            addr: won.addr,
            name,
            os,
            version,
        }),
        Ok(Message::Refused { reason }) => Err(OpenError::Refused(reason)),
        Ok(other) => Err(OpenError::Lost(WireError::Unexpected { got: other.name() })),
        Err(error) => Err(OpenError::Lost(error)),
    }
}

/// Races `candidates` (best first, staggered); the first connection wins.
pub(super) async fn race(
    candidates: Vec<Candidate>,
    locals: Vec<LocalInterface>,
) -> Option<(TcpStream, Candidate)> {
    let locals = Arc::new(locals);
    let mut attempts = JoinSet::new();
    for (position, candidate) in candidates.into_iter().enumerate() {
        let locals = locals.clone();
        let delay = STAGGER.saturating_mul(u32::try_from(position).unwrap_or(u32::MAX));
        attempts.spawn(async move {
            tokio::time::sleep(delay).await;
            let stream = tokio::time::timeout(CONNECT_TIMEOUT, connect(&candidate, &locals))
                .await
                .ok()?
                .ok()?;
            Some((stream, candidate))
        });
    }
    while let Some(attempt) = attempts.join_next().await {
        if let Ok(Some(won)) = attempt {
            // Dropping the set aborts the attempts still running.
            return Some(won);
        }
    }
    None
}

/// One connect. A direct candidate's socket is pinned to the interface that
/// saw it: every cable shares 169.254/16, and the routing table would send
/// them all down one.
async fn connect(candidate: &Candidate, locals: &[LocalInterface]) -> std::io::Result<TcpStream> {
    let socket = match candidate.addr {
        SocketAddr::V4(_) => TcpSocket::new_v4()?,
        SocketAddr::V6(_) => TcpSocket::new_v6()?,
    };
    if candidate.kind == PathKind::Direct
        && let Some(interface) = candidate
            .interface_index
            .and_then(|index| locals.iter().find(|local| local.index == index))
    {
        let pinned = socket2::SockRef::from(&socket);
        // Best effort: unpinned, the connect still works on a desk with one
        // cable.
        let _ = platform::bind_to_interface(&pinned, interface, candidate.addr.is_ipv6());
    }
    let stream = socket.connect(candidate.addr).await?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(addr: SocketAddr, kind: PathKind) -> Candidate {
        Candidate {
            addr,
            kind,
            interface: None,
            interface_index: None,
        }
    }

    /// When the best path does not answer (the cable unplugged, its port
    /// closed), the race falls back to the next candidate.
    #[tokio::test]
    async fn the_race_falls_back_when_the_best_path_is_down() {
        let live = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead_addr = dead.local_addr().unwrap();
        drop(dead);
        let candidates = vec![
            candidate(dead_addr, PathKind::Direct),
            candidate(live.local_addr().unwrap(), PathKind::Lan),
        ];
        let (_, won) = race(candidates, Vec::new()).await.unwrap();
        assert_eq!(won.kind, PathKind::Lan);
    }

    /// When both answer, the best path wins: the next one has not even
    /// started when the first connects.
    #[tokio::test]
    async fn the_best_path_wins_when_both_answer() {
        let direct = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let lan = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let candidates = vec![
            candidate(direct.local_addr().unwrap(), PathKind::Direct),
            candidate(lan.local_addr().unwrap(), PathKind::Lan),
        ];
        let (_, won) = race(candidates, Vec::new()).await.unwrap();
        assert_eq!(won.kind, PathKind::Direct);
    }

    /// No candidate at all is no connection, not a hang.
    #[tokio::test]
    async fn no_candidates_is_unreachable() {
        assert!(race(Vec::new(), Vec::new()).await.is_none());
    }
}
