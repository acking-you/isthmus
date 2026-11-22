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
use crate::wire::PROTOCOL_VERSION;

/// How long any one step may take before the test fails.
const STEP: Duration = Duration::from_secs(20);

struct Node {
    service: LinkService,
    changes: mpsc::Receiver<()>,
    /// What paired devices copied, as it would go onto this clipboard.
    clipboard: mpsc::Receiver<String>,
    receive: PathBuf,
    data: PathBuf,
    _scratch: ScratchDir,
}

impl Node {
    fn new(label: &str) -> Self {
        Self::speaking(label, PROTOCOL_VERSION)
    }

    /// A node that says it speaks `version`, standing in for an older
    /// device.
    fn speaking(label: &str, version: u32) -> Self {
        let scratch = ScratchDir::new(label);
        let data = scratch.path().join("data");
        let receive = scratch.path().join("received");
        std::fs::create_dir_all(&data).unwrap();
        let (service, changes, clipboard) = start_speaking(&data, &receive, None, version);
        Self {
            service,
            changes,
            clipboard,
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
        self.wait_for(what, STEP, ready)
    }

    /// [`Self::wait`], allowing `limit`.
    fn wait_for(
        &self,
        what: &str,
        limit: Duration,
        ready: impl Fn(&LinkStatus) -> bool,
    ) -> LinkStatus {
        let deadline = Instant::now() + limit;
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
            ..
        } = self;
        drop(service);
        drop(changes);
        // The old service's tasks may hold its lock a moment after it is
        // dropped; the app's own retry on `AnotherInstance` does the same.
        let deadline = Instant::now() + STEP;
        let (service, changes, clipboard) = loop {
            match try_start(&data, &receive, Some(port), PROTOCOL_VERSION) {
                Ok(started) => break started,
                Err(crate::Error::AnotherInstance) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => panic!("the node did not restart: {error}"),
            }
        };
        Node {
            service,
            changes,
            clipboard,
            receive,
            data,
            _scratch,
        }
    }
}

/// A service, its change hints, and what it would put on the clipboard.
type Started = (LinkService, mpsc::Receiver<()>, mpsc::Receiver<String>);

fn start_speaking(
    data: &Path,
    receive: &Path,
    port: Option<u16>,
    protocol_version: u32,
) -> Started {
    try_start(data, receive, port, protocol_version).unwrap()
}

fn try_start(
    data: &Path,
    receive: &Path,
    port: Option<u16>,
    protocol_version: u32,
) -> Result<Started, crate::Error> {
    let (tx, changes) = mpsc::channel();
    let (clipboard_tx, clipboard) = mpsc::channel();
    let service = LinkService::start(LinkOptions {
        data_dir: data.to_path_buf(),
        on_change: Some(Arc::new(move || {
            let _ = tx.send(());
        })),
        discovery: false,
        port: Some(port.unwrap_or(0)),
        receive_dir: Some(receive.to_path_buf()),
        loopback: true,
        protocol_version,
        on_clipboard: Some(Arc::new(move |_, text| {
            let _ = clipboard_tx.send(text);
        })),
    })?;
    Ok((service, changes, clipboard))
}

/// Pairs `a` (asking) with `b`, as a user would: both show the same code
/// and `b`'s user accepts.
fn pair(a: &Node, b: &Node) {
    a.service.introduce(&b.service);
    a.service.pair(&b.id()).unwrap();
    let confirming = |s: &LinkStatus| {
        s.pairing
            .as_ref()
            .is_some_and(|p| p.stage == PairStage::Confirm && p.code.is_some())
    };
    let asked = b.wait("the request to show", confirming);
    let asking = a.wait("the code to show here too", confirming);
    let shown = asked.pairing.unwrap();
    assert_eq!(shown.role, PairRole::Incoming);
    assert_eq!(
        shown.code,
        asking.pairing.unwrap().code,
        "both show one code"
    );
    b.service.accept_pairing().unwrap();
    finish_pairing(a, b);
}

/// Waits until both sides say the pairing is done.
fn finish_pairing(a: &Node, b: &Node) {
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

/// Pairs `a` (asking) with `b` (showing the code) by code, as with a
/// version 1 device: the user types the code `b` shows into `a`.
fn pair_by_code(a: &Node, b: &Node) {
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
    finish_pairing(a, b);
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

/// One click on the device being added pairs both: they show the same
/// code, each then pins the other, and a session comes up without anything
/// more.
#[test]
fn accepting_on_the_other_device_pairs_both_and_a_session_follows() {
    let a = Node::new("pair-a");
    let b = Node::new("pair-b");
    pair(&a, &b);
    let (a_id, b_id) = (a.id(), b.id());
    a.wait("a session with b", |s| online_with(s, &b_id));
    b.wait("a session with a", |s| online_with(s, &a_id));
}

/// Declining on the device being added fails the pairing on both sides as
/// declined, and pins nothing anywhere.
#[test]
fn declining_pairs_nothing() {
    let a = Node::new("decline-a");
    let b = Node::new("decline-b");
    a.service.introduce(&b.service);
    a.service.pair(&b.id()).unwrap();
    b.wait("the request to show", |s| {
        s.pairing
            .as_ref()
            .is_some_and(|p| p.stage == PairStage::Confirm)
    });
    b.service.cancel_pairing();
    let a_status = a.wait("a's pairing to be declined", |s| {
        s.pairing
            .as_ref()
            .is_some_and(|p| p.failure == Some(PairFailure::Declined))
    });
    assert!(a_status.peers.iter().all(|peer| !peer.paired));
    assert!(b.service.status().peers.iter().all(|peer| !peer.paired));
}

/// A device on version 1 is still paired with by code, whichever side
/// asks: an old dialler gets a code shown, and a new dialler follows an old
/// answerer's code.
#[test]
fn a_version_1_device_pairs_by_code_either_way() {
    let new = Node::new("mixed-new");
    let old = Node::speaking("mixed-old", 1);
    pair_by_code(&old, &new);
    let (new_id, old_id) = (new.id(), old.id());
    new.wait("a session with the old device", |s| online_with(s, &old_id));
    old.wait("a session with the new device", |s| online_with(s, &new_id));
}

/// A wrong code fails on both sides and pins nothing anywhere.
#[test]
fn a_wrong_code_pairs_nothing() {
    let a = Node::speaking("wrong-a", 1);
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

/// A transfer whose receiver goes away mid-file says why it is retrying,
/// carries on from where it stopped once the receiver is back, within
/// seconds rather than a TCP timeout, and the file arrives whole. The
/// resume is what is timed; the rest of the file gets as long as a debug
/// build on a slow runner needs.
#[test]
fn a_transfer_resumes_soon_after_its_receiver_comes_back() {
    let a = Node::new("resume-a");
    let b = Node::new("resume-b");
    pair(&a, &b);
    let b_id = b.id();
    a.wait("a session with b", |s| online_with(s, &b_id));
    let source = a._scratch.path().join("big.bin");
    let big: Vec<u8> = (0..96_000_000u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(&source, &big).unwrap();
    let id = a.service.send(&b_id, vec![source]).unwrap();
    let of = |s: &LinkStatus| s.transfers.iter().find(|t| t.id == id).cloned();
    a.wait("bytes to move", |s| {
        of(s).is_some_and(|t| t.done_bytes > 1_000_000 && t.state == TransferState::Running)
    });

    let b = b.restart();
    let retrying = a.wait("the sender to say why it retries", |s| {
        of(s).is_some_and(|t| t.retry.is_some())
    });
    let retry = of(&retrying).unwrap().retry.unwrap();
    assert!(retry.attempt >= 1, "{retry:?}");
    let back = Instant::now();
    a.wait("the transfer to resume", |s| {
        of(s).is_some_and(|t| {
            t.state.is_finished() || (t.state == TransferState::Running && t.retry.is_none())
        })
    });
    assert!(
        back.elapsed() < Duration::from_secs(15),
        "resumed after {:?}",
        back.elapsed()
    );
    let done = a.wait_for("the transfer to finish", Duration::from_secs(180), |s| {
        of(s).is_some_and(|t| t.state.is_finished())
    });
    let transfer = of(&done).unwrap();
    assert_eq!(transfer.state, TransferState::Done, "{transfer:?}");
    assert_eq!(std::fs::read(b.receive.join("big.bin")).unwrap(), big);
}

/// Text copied on one device lands on the other's clipboard, and both say
/// so, with the device's name and a preview.
#[test]
fn a_copy_reaches_the_paired_device_and_both_say_so() {
    let a = Node::new("clip-a");
    let b = Node::new("clip-b");
    pair(&a, &b);
    let (a_id, b_id) = (a.id(), b.id());
    a.wait("a session with b", |s| online_with(s, &b_id));
    b.wait("a session with a", |s| online_with(s, &a_id));

    // Right after connecting, the two may still be settling on one of two
    // connections, and a copy sent on the one being dropped is lost; the
    // next copy goes. So the copy is repeated until one lands.
    let deadline = Instant::now() + STEP;
    let landed = loop {
        assert_eq!(a.service.share_clipboard("hello from a\nsecond line"), 1);
        match b.clipboard.recv_timeout(Duration::from_millis(500)) {
            Ok(text) => break text,
            Err(_) if Instant::now() < deadline => continue,
            Err(error) => panic!("no copy landed: {error}"),
        }
    };
    assert_eq!(landed, "hello from a\nsecond line");
    let sent = a.service.status().clipboard.last.unwrap();
    assert_eq!(sent.direction, Direction::Send);
    assert_eq!(sent.preview, "hello from a…");
    let received = b.wait("b to say it received", |s| {
        s.clipboard
            .last
            .as_ref()
            .is_some_and(|event| event.direction == Direction::Receive)
    });
    assert_eq!(received.clipboard.last.unwrap().chars, 24);
    // The session carried on after the clipboard.
    assert!(online_with(&b.service.status(), &a_id));
}

/// Turned off on the receiving device, nothing lands there; turned off on
/// the copying one, nothing leaves.
#[test]
fn clipboard_sharing_off_keeps_copies_home() {
    let a = Node::new("clipoff-a");
    let b = Node::new("clipoff-b");
    pair(&a, &b);
    let b_id = b.id();
    a.wait("a session with b", |s| online_with(s, &b_id));
    b.service.set_clipboard_sharing(false).unwrap();
    assert!(!b.service.status().clipboard.enabled);
    a.service.share_clipboard("secret plans");
    assert!(b.clipboard.recv_timeout(Duration::from_secs(1)).is_err());

    a.service.set_clipboard_sharing(false).unwrap();
    assert_eq!(a.service.share_clipboard("more plans"), 0);
}

/// A version 1 device is never sent the clipboard, which would end its
/// session, and stays connected.
#[test]
fn a_version_1_device_is_not_sent_the_clipboard() {
    let new = Node::new("clipold-new");
    let old = Node::speaking("clipold-old", 1);
    pair_by_code(&old, &new);
    let old_id = old.id();
    new.wait("a session with the old device", |s| online_with(s, &old_id));
    assert_eq!(new.service.share_clipboard("not for you"), 0);
    assert!(online_with(&new.service.status(), &old_id));
}
