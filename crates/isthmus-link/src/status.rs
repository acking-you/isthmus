//! What the window shows about file sharing, as one snapshot.
//!
//! The service builds a [`LinkStatus`] from its state on every `link_status`
//! call; the app re-reads it on each `link` change hint. The JSON is
//! camelCase, and each enum has a catch-all on the Dart side, so a later
//! value reads as something the app can show.

use serde::Serialize;

use crate::identity::DeviceId;
use crate::path::PathKind;
use crate::store::PeerOs;

/// Everything about file sharing at one moment.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkStatus {
    pub me: Me,
    /// Where received files go.
    pub receive_dir: String,
    /// Paired devices (online or not) and unpaired ones seen right now;
    /// paired first, then online, then by name.
    pub peers: Vec<PeerView>,
    pub pairing: Option<PairingView>,
    /// Newest first.
    pub transfers: Vec<TransferView>,
    pub clipboard: ClipboardView,
}

/// This device.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    pub id: DeviceId,
    pub name: String,
    pub os: PeerOs,
    pub port: u16,
    /// Where other devices can reach this one.
    pub paths: Vec<LocalPath>,
}

/// One of this device's addresses and the kind of path it is.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPath {
    pub interface: String,
    pub address: String,
    pub kind: PathKind,
}

/// Another device.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerView {
    pub id: DeviceId,
    pub name: String,
    pub os: PeerOs,
    pub paired: bool,
    /// Paired: a session is up. Unpaired: it is seen on the network.
    pub online: bool,
    /// The session's path, while one is up.
    pub connection: Option<ConnectionView>,
    /// The kinds of path it is seen on, best first.
    pub routes: Vec<PathKind>,
}

/// How a session runs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionView {
    pub kind: PathKind,
    pub interface: Option<String>,
    pub address: String,
}

/// Which side of a pairing this device is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PairRole {
    /// This device asked: it waits for the other device to accept, or (with
    /// a version 1 device) the user types the code here.
    Outgoing,
    /// The other device asked: the user accepts here, or (with a version 1
    /// device) this one shows the code.
    Incoming,
}

/// Where a pairing is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PairStage {
    /// Reaching the other device.
    Connecting,
    /// With a version 1 device. Outgoing: waiting for the user to type the
    /// code shown on the other device. Incoming: showing the code.
    Code,
    /// Both devices show the same code. Incoming: waiting for the user to
    /// accept or decline. Outgoing: waiting for the other device's user.
    Confirm,
    /// Checking the code.
    Verifying,
    Paired,
    Failed,
}

/// Why a pairing failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PairFailure {
    WrongCode,
    /// The other device's user declined.
    Declined,
    Unreachable,
    /// The other device is pairing with someone else, or was asked too
    /// often.
    Busy,
    Canceled,
    TimedOut,
    /// The connection dropped part way.
    Lost,
    /// The other device answered with a different key than it announced.
    Mismatch,
}

/// A pairing in progress, or just finished.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingView {
    pub role: PairRole,
    pub peer_id: DeviceId,
    pub peer_name: String,
    pub peer_os: PeerOs,
    /// The code to show: in a confirmation pairing on both devices, in a
    /// pairing by code on the incoming one.
    pub code: Option<String>,
    pub stage: PairStage,
    pub failure: Option<PairFailure>,
}

/// Which way files go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Send,
    Receive,
}

/// Where a transfer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferState {
    /// Reaching the other device.
    Connecting,
    /// Bytes are moving.
    Running,
    /// The path dropped; trying again on the next one.
    Retrying,
    Done,
    Failed,
    Canceled,
}

impl TransferState {
    /// Whether nothing more will happen to it.
    pub fn is_finished(self) -> bool {
        match self {
            TransferState::Done | TransferState::Failed | TransferState::Canceled => true,
            TransferState::Connecting | TransferState::Running | TransferState::Retrying => false,
        }
    }
}

/// Why a transfer failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferFailure {
    /// The other device no longer pairs with this one.
    NotPaired,
    /// It could not be reached on any path.
    Unreachable,
    /// The connection dropped and retrying did not bring it back.
    Lost,
    /// The receiver cannot store the files.
    Disk,
    /// A file changed or vanished while it was being sent.
    Source,
    /// The other device refused the offer.
    Refused,
    /// The other device canceled it.
    CanceledByPeer,
}

/// Sharing the clipboard with paired devices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardView {
    /// Whether this device shares its clipboard with paired devices: what
    /// is copied here goes to them, and what they copy lands here.
    pub enabled: bool,
    /// The latest copy that crossed, either way.
    pub last: Option<ClipboardEvent>,
}

/// One copy that crossed to or from this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardEvent {
    /// Rises with every copy, so the window can tell a new one from the
    /// last and say so once.
    pub serial: u64,
    pub direction: Direction,
    /// Received: the device it came from. Sent: the devices it went to.
    pub peers: Vec<String>,
    /// How many characters it has.
    pub chars: u64,
    /// Its first line, shortened, for the notice.
    pub preview: String,
}

/// Why a transfer is trying again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RetryReason {
    /// Nothing moved for a while: the path went quiet (Wi-Fi dropped, the
    /// cable was pulled, the other computer slept) without saying so.
    Stalled,
    /// The connection broke, and the system said why.
    Lost,
    /// No path to the other device answered.
    Unreachable,
    /// The other device was busy.
    Busy,
}

/// A transfer's latest trouble, while it retries and after it fails: what
/// the window shows instead of a bare "trying again".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryView {
    /// How many times it has started again since bytes last moved, from 1.
    pub attempt: u32,
    pub reason: RetryReason,
    /// The path that failed, when one was in use.
    pub path: Option<PathKind>,
    /// Its interface on this computer (`en8`, `Ethernet 2`).
    pub interface: Option<String>,
    /// The system's own words, when it gave any.
    pub detail: Option<String>,
}

/// One transfer, either way.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferView {
    pub id: String,
    pub peer_id: DeviceId,
    pub peer_name: String,
    pub direction: Direction,
    /// The first item's name.
    pub name: String,
    /// How many files (not folders).
    pub files: u32,
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub bytes_per_second: u64,
    pub state: TransferState,
    /// The path the bytes take or last took.
    pub path: Option<PathKind>,
    /// That path's interface on this computer.
    pub interface: Option<String>,
    pub failure: Option<TransferFailure>,
    /// Why it is retrying, or why its last attempt stopped.
    pub retry: Option<RetryView>,
    /// Received: the one item, or the folder several landed in.
    pub saved_to: Option<String>,
    /// Seconds since 1970.
    pub started_at: u64,
}
