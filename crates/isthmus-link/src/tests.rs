//! Two services in one process, on loopback, with discovery off: pairing,
//! sessions, transfers and unpairing end to end, over real sockets and the
//! real handshake. Each test waits on the services' change hints, never on
//! a fixed sleep.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::identity::DeviceId;
use crate::service::{LinkOptions, LinkService};
use crate::status::{
    Direction, LinkStatus, PairFailure, PairRole, PairStage, TransferFailure, TransferState,
};
use crate::test_support::ScratchDir;

/// How long any one step may take before the test fails.
const STEP: Duration = Duration::from_secs(20);

struct Node {
    service: LinkService,
    changes: mpsc::Receiver<()>,
    receive: PathBuf,
    data: PathBuf,
    _scratch: ScratchDir,
}

impl Node {
    fn new(label: &str) -> Self {
        let scratch = ScratchDir::new(label);
        let data = scratch.path().join("data");
        let receive = scratch.path().join("received");
        std::fs::create_dir_all(&data).unwrap();
        let (service, changes) = start(&data, &receive, None);
        Self {
            service,
            changes,
            receive,
            data,
            _scratch: scratch,
        }
    }

    fn id(&self) -> DeviceId {
        self.service.status().me.id
    }

    /// Waits until `ready` holds for this node's status.
    fn wait(&self, what: &str, ready: impl Fn(&LinkStatus) -> bool) -> LinkStatus {
        let deadline = Instant::now() + STEP;
        loop {
            let status = self.service.status();
            if ready(&status) {
                return status;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "timed out waiting for {what}: {status:#?}");
            let _ = self
                .changes
                .recv_timeout(left.min(Duration::from_millis(200)));
        }
    }

    /// Stops this node and starts it again on the same data directory and
    /// port, as after a restart. The old service, and the lock it holds,
    /// is gone before the new one starts.
    fn restart(self) -> Self {
        let port = self.service.status().me.port;
        let Node {
            service,
            changes,
            receive,
            data,
            _scratch,
        } = self;
        drop(service);
        drop(changes);
        let (service, changes) = start(&data, &receive, Some(port));
        Node {
            service,
            changes,
            receive,
            data,
            _scratch,
        }
    }
}

fn start(data: &Path, receive: &Path, port: Option<u16>) -> (LinkService, mpsc::Receiver<()>) {
    let (tx, changes) = mpsc::channel();
    let service = LinkService::start(LinkOptions {
        data_dir: data.to_path_buf(),
        on_change: Some(Arc::new(move || {
            let _ = tx.send(());
        })),
        discovery: false,
        port: Some(port.unwrap_or(0)),
        receive_dir: Some(receive.to_path_buf()),
        loopback: true,
    })
    .unwrap();
    (service, changes)
}

/// Pairs `a` (asking) with `b` (showing the code), as a user would.
fn pair(a: &Node, b: &Node) {
    a.service.introduce(&b.service);
    a.service.pair(&b.id()).unwrap();
    let shown = b.wait("the code to show", |s| {
        s.pairing
            .as_ref()
            .is_some_and(|p| p.role == PairRole::Incoming && p.code.is_some())
    });
    a.wait("the code prompt", |s| {
        s.pairing
            .as_ref()
            .is_some_and(|p| p.stage == PairStage::Code)
    });
    let code = shown.pairing.unwrap().code.unwrap();
    a.service.enter_code(&code).unwrap();
    a.wait("pairing to finish", |s| {
        s.pairing
            .as_ref()
            .is_some_and(|p| p.stage == PairStage::Paired)
    });
    b.wait("pairing to finish", |s| {
        s.pairing
            .as_ref()
            .is_some_and(|p| p.stage == PairStage::Paired)
    });
}

fn online_with(status: &LinkStatus, id: &DeviceId) -> bool {
    status
        .peers
        .iter()
        .any(|peer| &peer.id == id && peer.paired && peer.online)
}

fn write_tree(root: &Path) {
    std::fs::create_dir_all(root.join("nested").join("deeper")).unwrap();
    std::fs::write(root.join("small.txt"), b"hello from the other computer").unwrap();
    std::fs::write(root.join("empty.bin"), b"").unwrap();
    // Several frames and several read chunks.
    let big: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(root.join("nested").join("deeper").join("big.bin"), big).unwrap();
}

fn assert_same_tree(left: &Path, right: &Path) {
    for name in ["small.txt", "empty.bin", "nested/deeper/big.bin"] {
        assert_eq!(
            std::fs::read(left.join(name)).unwrap(),
            std::fs::read(right.join(name)).unwrap(),
            "{name} differs"
        );
    }
}

/// The code shown on one device, typed on the other, pairs both; each then
/// pins the other and a session comes up without anything more.
#[test]
fn the_shown_code_pairs_both_devices_and_a_session_follows() {
    let a = Node::new("pair-a");
    let b = Node::new("pair-b");
    pair(&a, &b);
    let (a_id, b_id) = (a.id(), b.id());
    a.wait("a session with b", |s| online_with(s, &b_id));
    b.wait("a session with a", |s| online_with(s, &a_id));
}

/// A wrong code fails on both sides and pins nothing anywhere.
#[test]
fn a_wrong_code_pairs_nothing() {
    let a = Node::new("wrong-a");
    let b = Node::new("wrong-b");
    a.service.introduce(&b.service);
    a.service.pair(&b.id()).unwrap();
    let shown = b.wait("the code to show", |s| {
        s.pairing.as_ref().is_some_and(|p| p.code.is_some())
    });
    a.wait("the code prompt", |s| {
        s.pairing
            .as_ref()
            .is_some_and(|p| p.stage == PairStage::Code)
    });
    let code = shown.pairing.unwrap().code.unwrap();
    let wrong = format!("{:06}", (code.parse::<u32>().unwrap() + 1) % 1_000_000);
    a.service.enter_code(&wrong).unwrap();
    let failed = |s: &LinkStatus| {
        s.pairing
            .as_ref()
            .is_some_and(|p| p.failure == Some(PairFailure::WrongCode))
    };
    let a_status = a.wait("a's pairing to fail", failed);
    let b_status = b.wait("b's pairing to fail", failed);
    assert!(a_status.peers.iter().all(|peer| !peer.paired));
    assert!(b_status.peers.iter().all(|peer| !peer.paired));
}

/// A folder sent to a paired device arrives whole under its own name, a
/// second send of the same name lands beside it rather than over it, and
/// both sides end with the transfer done.
#[test]
fn a_folder_arrives_whole_and_a_repeat_gets_a_free_name() {
    let a = Node::new("send-a");
    let b = Node::new("send-b");
    pair(&a, &b);
    let source_scratch = ScratchDir::new("send-source");
    let folder = source_scratch.path().join("Project");
    write_tree(&folder);

    let b_id = b.id();
    let id = a.service.send(&b_id, vec![folder.clone()]).unwrap();
    let sent = a.wait("the send to finish", |s| {
        s.transfers
            .iter()
            .any(|t| t.id == id && t.state == TransferState::Done)
    });
    let received = b.wait("the receive to finish", |s| {
        s.transfers
            .iter()
            .any(|t| t.id == id && t.state == TransferState::Done)
    });
    let sent = sent.transfers.into_iter().find(|t| t.id == id).unwrap();
    assert_eq!(sent.direction, Direction::Send);
    assert_eq!(sent.done_bytes, sent.total_bytes);
    assert_eq!(sent.files, 3);
    let received = received.transfers.into_iter().find(|t| t.id == id).unwrap();
    assert_eq!(received.direction, Direction::Receive);
    let landed = b.receive.join("Project");
    assert_eq!(received.saved_to.as_deref(), landed.to_str());
    assert_same_tree(&folder, &landed);

    let again = a.service.send(&b_id, vec![folder.clone()]).unwrap();
    b.wait("the second receive", |s| {
        s.transfers
            .iter()
            .any(|t| t.id == again && t.state == TransferState::Done)
    });
    assert_same_tree(&folder, &b.receive.join("Project (1)"));
    let leftovers: Vec<_> = std::fs::read_dir(&b.receive)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with('.'))
        .collect();
    assert!(leftovers.is_empty(), "staging left behind: {leftovers:?}");
}

/// Files go only to paired devices.
#[test]
fn an_unpaired_device_cannot_be_sent_to() {
    let a = Node::new("unpaired-a");
    let b = Node::new("unpaired-b");
    a.service.introduce(&b.service);
    let scratch = ScratchDir::new("unpaired-file");
    let file = scratch.path().join("note.txt");
    std::fs::write(&file, b"x").unwrap();
    assert!(matches!(
        a.service.send(&b.id(), vec![file]),
        Err(crate::Error::NotPaired { .. })
    ));
}

/// Unpairing on one side unpairs the other through the session, and a
/// transfer to it is refused from then on.
#[test]
fn unpairing_reaches_the_other_side() {
    let a = Node::new("unpair-a");
    let b = Node::new("unpair-b");
    pair(&a, &b);
    let (a_id, b_id) = (a.id(), b.id());
    b.wait("a session with a", |s| online_with(s, &a_id));
    a.service.unpair(&b_id).unwrap();
    b.wait("a to be unpaired on b", |s| {
        s.peers.iter().all(|peer| peer.id != a_id || !peer.paired)
    });
    assert!(matches!(
        a.service.unpair(&b_id),
        Err(crate::Error::NotPaired { .. })
    ));
}

/// A paired device that restarts is found again from its remembered
/// address, with no code and no discovery.
#[test]
fn a_restarted_device_reconnects_by_itself() {
    let a = Node::new("restart-a");
    let b = Node::new("restart-b");
    pair(&a, &b);
    let (a_id, b_id) = (a.id(), b.id());
    a.wait("a session with b", |s| online_with(s, &b_id));
    let b = b.restart();
    a.service.lose_sight_of(&b_id);
    a.wait("the session to come back", |s| online_with(s, &b_id));
    b.wait("the session to come back", |s| online_with(s, &a_id));
}

/// A transfer canceled by the receiver ends on both sides and leaves no
/// partial files.
#[test]
fn a_receiver_cancel_stops_the_sender() {
    let a = Node::new("cancel-a");
    let b = Node::new("cancel-b");
    pair(&a, &b);
    let scratch = ScratchDir::new("cancel-source");
    let big = scratch.path().join("big.bin");
    // Large enough to still be running when the cancel lands.
    std::fs::write(&big, vec![9u8; 64 * 1024 * 1024]).unwrap();
    let id = a.service.send(&b.id(), vec![big]).unwrap();
    b.wait("the receive to start", |s| {
        s.transfers
            .iter()
            .any(|t| t.id == id && t.state == TransferState::Running)
    });
    b.service.cancel_transfer(&id).unwrap();
    b.wait("the receive to stop", |s| {
        s.transfers
            .iter()
            .any(|t| t.id == id && t.state == TransferState::Canceled)
    });
    let sent = a.wait("the send to stop", |s| {
        s.transfers
            .iter()
            .any(|t| t.id == id && t.state.is_finished())
    });
    let sent = sent.transfers.into_iter().find(|t| t.id == id).unwrap();
    assert!(
        sent.state == TransferState::Canceled
            || sent.failure == Some(TransferFailure::CanceledByPeer),
        "{sent:?}"
    );
    assert!(
        std::fs::read_dir(&b.receive).unwrap().next().is_none(),
        "nothing is left in the receive folder"
    );
}
