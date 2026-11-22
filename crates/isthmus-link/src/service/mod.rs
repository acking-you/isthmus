//! The link service: one per process, owning the listener, discovery, the
//! sessions with paired devices, pairings and transfers.
//!
//! Everything runs on a small tokio runtime the service owns, so its
//! callers (the FFI's worker isolate, the tests) stay synchronous. State
//! lives in one `parking_lot` mutex that is never held across an `.await`;
//! tasks read it, act, and write their outcome back. Every visible change
//! calls the change sink, except transfer progress, which a ticker folds
//! into at most four hints a second.
//!
//! Connections, by purpose:
//!
//! - **Session**: one long-lived connection per paired device, for presence
//!   and for news (a rename, an unpairing). Both sides dial when none is up;
//!   when two meet, both keep the better one by the same rule (a direct
//!   path over the LAN, then the one the lower id dialled), so neither side
//!   ever drops the one the other kept. A session on the LAN keeps looking
//!   for the cable and moves to it when it answers.
//! - **Pair**: a pairing with a code (`pair.rs`).
//! - **Transfer**: one connection per transfer (`send.rs`, `receive.rs`),
//!   dialled over the best path at that moment, and again over the next
//!   one when it drops.

mod connect;
mod pair;
mod receive;
mod send;
mod session;

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use isthmus_core::instance::InstanceLock;
use parking_lot::Mutex;
use snafu::ResultExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::{Handle, Runtime};
use tokio::sync::{Notify, mpsc, oneshot, watch};
use tracing::{debug, info, warn};

use crate::discovery::{Announcement, Discovery, DiscoveryEvent, Sighting};
use crate::error::{Error, ListenSnafu, LockSnafu, RuntimeSnafu};
use crate::files;
use crate::identity::{DeviceId, Identity, PublicKey};
use crate::pairing::PairCode;
use crate::path::{self, Candidate, LocalInterface, PathKind};
use crate::platform;
use crate::status::{
    ClipboardEvent, ClipboardView, ConnectionView, Direction, LinkStatus, LocalPath, Me,
    PairFailure, PairRole, PairStage, PairingView, PeerView, RetryView, TransferFailure,
    TransferState, TransferView,
};
use crate::store::{
    self, IDENTITY_FILE, LOCK_FILE, LinkSettings, PEERS_FILE, PairedPeer, PeerOs, PeersFile,
    SETTINGS_FILE,
};
use crate::wire::{self, CONFIRM_VERSION, Message, PROTOCOL_VERSION, Purpose, Refusal};

/// The port Isthmus listens on when it is free; any free port otherwise.
/// A fixed port lets a remembered address work after a restart and lets a
/// firewall rule name it.
pub const DEFAULT_PORT: u16 = 47821;

/// How often the connector looks for paired devices to dial and re-reads
/// the interfaces.
const CONNECT_TICK: Duration = Duration::from_secs(2);

/// The first and the longest wait before dialling a paired device again.
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

/// How often a session on the LAN tries the cable.
const UPGRADE_EVERY: Duration = Duration::from_secs(15);

/// How often progress may reach the window.
const PROGRESS_TICK: Duration = Duration::from_millis(250);

/// How many finished transfers the list keeps.
const KEEP_FINISHED: usize = 50;

/// A staging folder untouched this long belongs to a transfer that will not
/// come back.
const STALE_STAGING: Duration = Duration::from_secs(3 * 24 * 60 * 60);

/// Called after every change the window should show.
pub type ChangeSink = Arc<dyn Fn() + Send + Sync>;

/// How to start the service.
pub struct LinkOptions {
    /// The data directory (`isthmus_core::paths::data_dir`).
    pub data_dir: PathBuf,
    pub on_change: Option<ChangeSink>,
    /// Announce and browse with mDNS. Tests turn it off and introduce
    /// devices to each other by hand.
    pub discovery: bool,
    /// The port to listen on; `None` for [`DEFAULT_PORT`], falling back to
    /// any free one.
    pub port: Option<u16>,
    /// Where received files go when the settings do not say; `None` for the
    /// Downloads folder.
    pub receive_dir: Option<PathBuf>,
    /// Listen on loopback only and accept loopback addresses as a path:
    /// two services in one test process.
    pub loopback: bool,
    /// The protocol version this device says it speaks
    /// ([`crate::wire::PROTOCOL_VERSION`]); lower only to stand in for an
    /// older device in a test.
    pub protocol_version: u32,
    /// Where text another device copied goes, to be put on this
    /// computer's clipboard; `None` drops it.
    pub on_clipboard: Option<ClipboardSink>,
}

/// Puts text a paired device copied onto this computer's clipboard.
pub type ClipboardSink = Arc<dyn Fn(&DeviceId, String) + Send + Sync>;

impl LinkOptions {
    /// The options the app uses: the real network, the default port.
    pub fn new(data_dir: PathBuf, on_change: Option<ChangeSink>) -> Self {
        Self {
            data_dir,
            on_change,
            discovery: true,
            port: None,
            receive_dir: None,
            loopback: false,
            protocol_version: PROTOCOL_VERSION,
            on_clipboard: None,
        }
    }
}

/// File sharing, running.
pub struct LinkService {
    core: Arc<Core>,
    runtime: Mutex<Option<Runtime>>,
}

/// What every task shares.
pub(crate) struct Core {
    identity: Identity,
    data_dir: PathBuf,
    default_receive_dir: PathBuf,
    computer_name: String,
    os: PeerOs,
    port: u16,
    loopback: bool,
    /// What hellos and welcomes say ([`LinkOptions::protocol_version`]).
    protocol_version: u32,
    state: Mutex<State>,
    on_change: Option<ChangeSink>,
    on_clipboard: Option<ClipboardSink>,
    progress_dirty: AtomicBool,
    discovery: Mutex<Option<Discovery>>,
    shutdown: watch::Sender<bool>,
    handle: Handle,
    wake: Notify,
    serial: AtomicU64,
    _lock: InstanceLock,
}

/// Everything that changes.
#[derive(Default)]
struct State {
    settings: LinkSettings,
    peers: PeersFile,
    seen: HashMap<DeviceId, Seen>,
    sessions: HashMap<DeviceId, Session>,
    dialing: HashSet<DeviceId>,
    retry: HashMap<DeviceId, Retry>,
    pairing: Option<Pairing>,
    last_incoming_pair: Option<Instant>,
    /// The latest copy that crossed, either way.
    clipboard: Option<ClipboardEvent>,
    transfers: Vec<Transfer>,
    locals: Vec<LocalInterface>,
}

/// A device mDNS sees.
struct Seen {
    name: String,
    os: PeerOs,
    addrs: Vec<(SocketAddr, Option<u32>)>,
}

/// A live session.
struct Session {
    conn: u64,
    kind: PathKind,
    interface: Option<String>,
    addr: SocketAddr,
    dialed_by_lower: bool,
    /// The protocol version the peer said it speaks.
    version: u32,
    tx: mpsc::UnboundedSender<SessionCommand>,
}

impl Session {
    /// The keep-order: lower is better. The path first, then whether the
    /// lower id dialled; both sides compute the same for one connection.
    fn rank(kind: PathKind, dialed_by_lower: bool) -> (PathKind, bool) {
        (kind, !dialed_by_lower)
    }
}

/// What a session task is told.
pub(crate) enum SessionCommand {
    Close,
    Unpair,
    Rename(String),
    /// Text copied here, for a version 2 device.
    Clipboard(Arc<str>),
}

/// When to dial a paired device again.
struct Retry {
    next: Instant,
    delay: Duration,
    last_upgrade: Option<Instant>,
}

impl Default for Retry {
    fn default() -> Self {
        Self {
            next: Instant::now(),
            delay: RETRY_FIRST,
            last_upgrade: None,
        }
    }
}

/// A pairing in progress or just finished.
struct Pairing {
    serial: u64,
    role: PairRole,
    peer: DeviceId,
    peer_name: String,
    peer_os: PeerOs,
    /// The code shown: by confirmation on both sides, by code on the
    /// incoming one.
    code: Option<PairCode>,
    stage: PairStage,
    failure: Option<PairFailure>,
    /// Outgoing, by code: where the typed code goes, until it is used.
    code_tx: Option<oneshot::Sender<PairCode>>,
    /// Incoming, by confirmation: where the user's acceptance goes.
    accept_tx: Option<oneshot::Sender<()>>,
    cancel: watch::Sender<bool>,
}

impl Pairing {
    fn is_active(&self) -> bool {
        match self.stage {
            PairStage::Connecting | PairStage::Code | PairStage::Confirm | PairStage::Verifying => {
                true
            }
            PairStage::Paired | PairStage::Failed => false,
        }
    }
}

/// One transfer, either way.
struct Transfer {
    id: String,
    peer: DeviceId,
    peer_name: String,
    direction: Direction,
    name: String,
    files: u32,
    total: u64,
    done: Arc<AtomicU64>,
    state: TransferState,
    path: Option<PathKind>,
    /// The path's interface on this computer.
    interface: Option<String>,
    failure: Option<TransferFailure>,
    /// Why it is retrying, or why its last attempt stopped.
    retry: Option<RetryView>,
    saved_to: Option<PathBuf>,
    started_at: u64,
    cancel: watch::Sender<bool>,
    speed: u64,
    sample: (Instant, u64),
    /// Receiving: which connection owns it. A retry that arrives before
    /// the old connection's drop was noticed bumps it, and the old one
    /// stops at its next frame.
    attempt: Arc<AtomicU64>,
}

/// How a transfer ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Done,
    Canceled,
    Failed(TransferFailure),
}

impl LinkService {
    /// Starts file sharing on `options.data_dir`: takes the link lock,
    /// reads the identity and state files, listens, and (unless turned off)
    /// announces and browses.
    pub fn start(options: LinkOptions) -> Result<Self, Error> {
        let data_dir = options.data_dir;
        let lock = InstanceLock::try_acquire(&data_dir.join(LOCK_FILE))
            .context(LockSnafu)?
            .ok_or(Error::AnotherInstance)?;
        let identity = Identity::load_or_create(&data_dir.join(IDENTITY_FILE))?;
        let settings: LinkSettings = store::read(&data_dir.join(SETTINGS_FILE))?;
        let peers: PeersFile = store::read(&data_dir.join(PEERS_FILE))?;
        let default_receive_dir = options
            .receive_dir
            .or_else(platform::downloads_dir)
            .unwrap_or_else(|| data_dir.join("Received"));

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("isthmus-link")
            .enable_all()
            .build()
            .context(RuntimeSnafu)?;
        let (listeners, port) = runtime
            .block_on(listen(
                options.port.unwrap_or(DEFAULT_PORT),
                options.loopback,
            ))
            .context(ListenSnafu)?;

        let (shutdown, _) = watch::channel(false);
        let core = Arc::new(Core {
            computer_name: platform::computer_name()
                .and_then(|name| store::valid_name(&name))
                .unwrap_or_else(|| identity.id().to_string()),
            identity,
            data_dir,
            default_receive_dir,
            os: platform::this_os(),
            port,
            loopback: options.loopback,
            protocol_version: options.protocol_version,
            state: Mutex::new(State {
                settings,
                peers,
                locals: path::local_interfaces(),
                ..State::default()
            }),
            on_change: options.on_change,
            on_clipboard: options.on_clipboard,
            progress_dirty: AtomicBool::new(false),
            discovery: Mutex::new(None),
            shutdown,
            handle: runtime.handle().clone(),
            wake: Notify::new(),
            serial: AtomicU64::new(1),
            _lock: lock,
        });
        info!(id = %core.identity.id(), port, "file sharing listening");

        for listener in listeners {
            runtime.spawn(accept(core.clone(), listener));
        }
        runtime.spawn(connector(core.clone()));
        runtime.spawn(progress(core.clone()));
        let receive_dir = core.receive_dir();
        runtime.spawn_blocking(move || prepare_receive_dir(&receive_dir));

        if options.discovery {
            let (tx, rx) = mpsc::unbounded_channel();
            let discovery = Discovery::start(&core.announcement(), move |event| {
                let _ = tx.send(event);
            })?;
            *core.discovery.lock() = Some(discovery);
            runtime.spawn(discovered(core.clone(), rx));
        }
        Ok(Self {
            core,
            runtime: Mutex::new(Some(runtime)),
        })
    }

    /// What the window shows.
    pub fn status(&self) -> LinkStatus {
        self.core.status()
    }

    /// Asks `peer` to pair. Both devices then show the same code and the
    /// other device's user accepts there; a version 1 device shows a code
    /// instead, which the user enters here with [`Self::enter_code`].
    pub fn pair(&self, peer: &DeviceId) -> Result<(), Error> {
        let core = &self.core;
        let (serial, code_rx, cancel) = {
            let mut state = core.state.lock();
            if state.pairing.as_ref().is_some_and(Pairing::is_active) {
                return Err(Error::PairingBusy);
            }
            let (peer_name, peer_os) = match (state.seen.get(peer), state.paired(peer)) {
                (Some(seen), _) => (seen.name.clone(), seen.os),
                (None, Some(paired)) => (paired.name.clone(), paired.os),
                (None, None) => {
                    return Err(Error::UnknownPeer {
                        id: peer.to_string(),
                    });
                }
            };
            let serial = core.next_serial();
            let (code_tx, code_rx) = oneshot::channel();
            let (cancel, cancel_rx) = watch::channel(false);
            state.pairing = Some(Pairing {
                serial,
                role: PairRole::Outgoing,
                peer: peer.clone(),
                peer_name,
                peer_os,
                code: None,
                stage: PairStage::Connecting,
                failure: None,
                code_tx: Some(code_tx),
                accept_tx: None,
                cancel,
            });
            (serial, code_rx, cancel_rx)
        };
        core.changed();
        core.handle.spawn(pair::ask(
            core.clone(),
            serial,
            peer.clone(),
            code_rx,
            cancel,
        ));
        Ok(())
    }

    /// The code the user read off the other device, for the pairing this
    /// device asked for.
    pub fn enter_code(&self, code: &str) -> Result<(), Error> {
        let code = PairCode::parse(code).ok_or(Error::InvalidCode)?;
        let mut state = self.core.state.lock();
        let pairing = state
            .pairing
            .as_mut()
            .filter(|pairing| {
                pairing.role == PairRole::Outgoing && pairing.stage == PairStage::Code
            })
            .ok_or(Error::NoPairing)?;
        let sender = pairing.code_tx.take().ok_or(Error::NoPairing)?;
        sender.send(code).map_err(|_| Error::NoPairing)
    }

    /// Accepts the pairing another device asked for, once both show their
    /// code.
    pub fn accept_pairing(&self) -> Result<(), Error> {
        let mut state = self.core.state.lock();
        let pairing = state
            .pairing
            .as_mut()
            .filter(|pairing| {
                pairing.role == PairRole::Incoming && pairing.stage == PairStage::Confirm
            })
            .ok_or(Error::NoPairing)?;
        let accept = pairing.accept_tx.take().ok_or(Error::NoPairing)?;
        pairing.stage = PairStage::Verifying;
        drop(state);
        self.core.changed();
        accept.send(()).map_err(|_| Error::NoPairing)
    }

    /// Cancels the pairing in progress, declines one waiting to be
    /// accepted, or dismisses one that finished.
    pub fn cancel_pairing(&self) {
        let mut state = self.core.state.lock();
        if let Some(pairing) = state.pairing.take() {
            let _ = pairing.cancel.send(true);
        }
        drop(state);
        self.core.changed();
    }

    /// Unpairs `peer` here and, if it is connected, there too.
    pub fn unpair(&self, peer: &DeviceId) -> Result<(), Error> {
        let mut state = self.core.state.lock();
        let before = state.peers.peers.len();
        state.peers.peers.retain(|paired| &paired.id != peer);
        if state.peers.peers.len() == before {
            return Err(Error::NotPaired {
                id: peer.to_string(),
            });
        }
        if let Some(session) = state.sessions.remove(peer) {
            let _ = session.tx.send(SessionCommand::Unpair);
        }
        state.retry.remove(peer);
        let saved = self.core.save_peers(&state);
        drop(state);
        self.core.changed();
        saved
    }

    /// Sends `paths` (files and folders) to the paired `peer`; answers the
    /// transfer's id. The transfer runs on, and is retried over the next
    /// path when one drops.
    pub fn send(&self, peer: &DeviceId, paths: Vec<PathBuf>) -> Result<String, Error> {
        let core = &self.core;
        let peer_name = core
            .state
            .lock()
            .paired(peer)
            .map(|paired| paired.name.clone())
            .ok_or_else(|| Error::NotPaired {
                id: peer.to_string(),
            })?;
        let collected = files::collect(&paths)?;
        let id = files::new_transfer_id().map_err(|_| Error::Runtime {
            source: std::io::Error::other("no random source"),
        })?;
        let (cancel, cancel_rx) = watch::channel(false);
        let transfer = Transfer {
            id: id.clone(),
            peer: peer.clone(),
            peer_name,
            direction: Direction::Send,
            name: files::summary_name(&collected.entries),
            files: collected.files(),
            total: collected.total(),
            done: Arc::new(AtomicU64::new(0)),
            state: TransferState::Connecting,
            path: None,
            interface: None,
            failure: None,
            retry: None,
            saved_to: None,
            started_at: now_seconds(),
            cancel,
            speed: 0,
            sample: (Instant::now(), 0),
            attempt: Arc::new(AtomicU64::new(0)),
        };
        let done = transfer.done.clone();
        core.state.lock().add_transfer(transfer);
        core.changed();
        core.handle.spawn(send::run(
            core.clone(),
            id.clone(),
            peer.clone(),
            collected,
            done,
            cancel_rx,
        ));
        Ok(id)
    }

    /// Stops a transfer; the other side is told.
    pub fn cancel_transfer(&self, id: &str) -> Result<(), Error> {
        let state = self.core.state.lock();
        let transfer = state
            .transfers
            .iter()
            .find(|transfer| transfer.id == id)
            .ok_or_else(|| Error::UnknownTransfer { id: id.to_owned() })?;
        let _ = transfer.cancel.send(true);
        // A receive waiting for its sender to come back has no task to hear
        // the cancel: it ends here, and its partial files go.
        let waiting =
            transfer.direction == Direction::Receive && transfer.state == TransferState::Retrying;
        drop(state);
        if waiting {
            self.core.finish_transfer(id, Outcome::Canceled, None);
            let staging = files::staging_dir(&self.core.receive_dir(), id);
            self.core
                .handle
                .spawn_blocking(move || std::fs::remove_dir_all(staging));
        }
        Ok(())
    }

    /// Shares `text`, just copied on this computer, with every paired device
    /// connected now that can take it; answers how many it went to. Nothing
    /// goes while sharing is off.
    pub fn share_clipboard(&self, text: &str) -> usize {
        let mut state = self.core.state.lock();
        if !state.settings.shares_clipboard() {
            return 0;
        }
        let shared: Arc<str> = Arc::from(text);
        let mut peers = Vec::new();
        for (id, session) in &state.sessions {
            if session.version >= CONFIRM_VERSION
                && session
                    .tx
                    .send(SessionCommand::Clipboard(shared.clone()))
                    .is_ok()
            {
                let name = state
                    .paired(id)
                    .map_or_else(|| id.to_string(), |paired| paired.name.clone());
                peers.push(name);
            }
        }
        let sent = peers.len();
        if sent > 0 {
            state.clipboard = Some(clipboard_event(
                self.core.next_serial(),
                Direction::Send,
                peers,
                text,
            ));
            drop(state);
            self.core.changed();
        }
        sent
    }

    /// Turns sharing the clipboard with paired devices on or off, and
    /// remembers it in `link.json`.
    pub fn set_clipboard_sharing(&self, enabled: bool) -> Result<(), Error> {
        let mut state = self.core.state.lock();
        let mut settings = state.settings.clone();
        // Unset means on, so the file says nothing until it is turned off.
        settings.clipboard = (!enabled).then_some(false);
        store::write(&self.core.data_dir.join(SETTINGS_FILE), &settings)?;
        state.settings = settings;
        drop(state);
        self.core.changed();
        Ok(())
    }

    /// Drops every finished transfer from the list.
    pub fn clear_transfers(&self) {
        self.core
            .state
            .lock()
            .transfers
            .retain(|transfer| !transfer.state.is_finished());
        self.core.changed();
    }

    /// Renames this device everywhere: the settings file, the announcement
    /// and every session.
    pub fn rename(&self, name: &str) -> Result<(), Error> {
        let name = store::valid_name(name).ok_or(Error::InvalidName {
            max: store::MAX_NAME_CHARS,
        })?;
        let mut state = self.core.state.lock();
        let mut settings = state.settings.clone();
        settings.name = (name != self.core.computer_name).then(|| name.clone());
        store::write(&self.core.data_dir.join(SETTINGS_FILE), &settings)?;
        state.settings = settings;
        for session in state.sessions.values() {
            let _ = session.tx.send(SessionCommand::Rename(name.clone()));
        }
        drop(state);
        if let Some(discovery) = self.core.discovery.lock().as_ref()
            && let Err(error) = discovery.announce(&self.core.announcement())
        {
            warn!(%error, "could not announce the new name");
        }
        self.core.changed();
        Ok(())
    }

    /// Shows a received transfer's files in the file manager, or the
    /// receive folder when `id` is `None`.
    pub fn reveal(&self, id: Option<&str>) -> Result<(), Error> {
        let receive_dir = self.core.receive_dir();
        let target = match id {
            None => None,
            Some(id) => {
                let state = self.core.state.lock();
                let transfer = state
                    .transfers
                    .iter()
                    .find(|transfer| transfer.id == id)
                    .ok_or_else(|| Error::UnknownTransfer { id: id.to_owned() })?;
                transfer.saved_to.clone()
            }
        };
        let result = match &target {
            Some(path) if path != &receive_dir => platform::reveal(path),
            _ => {
                let _ = std::fs::create_dir_all(&receive_dir);
                platform::open_folder(&receive_dir)
            }
        };
        result.map_err(|source| Error::Reveal {
            path: target.unwrap_or(receive_dir),
            source,
        })
    }

    /// Says goodbye on the network and stops every task; in-flight
    /// transfers keep their staging folders for a later resume. Idempotent.
    pub fn shutdown(&self) {
        let Some(runtime) = self.runtime.lock().take() else {
            return;
        };
        let _ = self.core.shutdown.send(true);
        if let Some(discovery) = self.core.discovery.lock().take() {
            discovery.shutdown(self.core.identity.id());
        }
        runtime.shutdown_timeout(Duration::from_secs(1));
    }

    /// Tells this service where `other` listens, as discovery would.
    #[cfg(test)]
    pub(crate) fn introduce(&self, other: &LinkService) {
        let me = other.status().me;
        self.core.seen(Sighting {
            id: me.id,
            name: me.name,
            os: me.os,
            version: PROTOCOL_VERSION,
            addrs: vec![(SocketAddr::from((Ipv4Addr::LOCALHOST, me.port)), None)],
        });
    }

    /// Forgets where `other` was seen, as when it leaves the network.
    #[cfg(test)]
    pub(crate) fn lose_sight_of(&self, other: &DeviceId) {
        self.core.gone(other);
    }
}

impl Drop for LinkService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl State {
    fn paired(&self, id: &DeviceId) -> Option<&PairedPeer> {
        self.peers.peers.iter().find(|peer| &peer.id == id)
    }

    fn paired_mut(&mut self, id: &DeviceId) -> Option<&mut PairedPeer> {
        self.peers.peers.iter_mut().find(|peer| &peer.id == id)
    }

    fn transfer_mut(&mut self, id: &str) -> Option<&mut Transfer> {
        self.transfers.iter_mut().find(|transfer| transfer.id == id)
    }

    /// Adds a transfer at the top, dropping the oldest finished ones past
    /// the limit.
    fn add_transfer(&mut self, transfer: Transfer) {
        self.transfers.insert(0, transfer);
        let mut finished = 0;
        self.transfers.retain(|transfer| {
            if transfer.state.is_finished() {
                finished += 1;
                finished <= KEEP_FINISHED
            } else {
                true
            }
        });
    }
}

impl Core {
    fn next_serial(&self) -> u64 {
        self.serial.fetch_add(1, Ordering::Relaxed)
    }

    /// Tells the window something changed.
    fn changed(&self) {
        if let Some(sink) = &self.on_change {
            sink();
        }
    }

    /// Notes progress for the ticker to report.
    fn progressed(&self) {
        self.progress_dirty.store(true, Ordering::Relaxed);
    }

    fn shutdown_signal(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    /// The name others see.
    /// `peer` copied `text`: onto this computer's clipboard with it, unless
    /// sharing is off here, and say so.
    fn clipboard_received(&self, peer: &DeviceId, text: String) {
        let mut state = self.state.lock();
        if !state.settings.shares_clipboard() {
            return;
        }
        let name = state
            .paired(peer)
            .map_or_else(|| peer.to_string(), |paired| paired.name.clone());
        state.clipboard = Some(clipboard_event(
            self.next_serial(),
            Direction::Receive,
            vec![name],
            &text,
        ));
        drop(state);
        if let Some(sink) = &self.on_clipboard {
            sink(peer, text);
        }
        self.changed();
    }

    fn name(&self, settings: &LinkSettings) -> String {
        settings
            .name
            .clone()
            .unwrap_or_else(|| self.computer_name.clone())
    }

    fn announcement(&self) -> Announcement {
        Announcement {
            id: self.identity.id().clone(),
            name: self.name(&self.state.lock().settings),
            os: self.os,
            port: self.port,
        }
    }

    fn hello(&self, purpose: Purpose) -> Message {
        Message::Hello {
            purpose,
            name: self.name(&self.state.lock().settings),
            os: self.os,
            version: self.protocol_version,
            port: self.port,
        }
    }

    fn welcome(&self) -> Message {
        Message::Welcome {
            name: self.name(&self.state.lock().settings),
            os: self.os,
            version: self.protocol_version,
            port: self.port,
        }
    }

    fn receive_dir(&self) -> PathBuf {
        self.state
            .lock()
            .settings
            .receive_dir
            .clone()
            .unwrap_or_else(|| self.default_receive_dir.clone())
    }

    fn locals(&self) -> Vec<LocalInterface> {
        self.state.lock().locals.clone()
    }

    /// The pinned key of `peer`, if paired.
    fn pinned_key(&self, peer: &DeviceId) -> Option<PublicKey> {
        self.state
            .lock()
            .paired(peer)
            .map(|paired| paired.public_key)
    }

    /// Whether `key` is `peer`'s pinned key.
    fn is_pinned(&self, peer: &DeviceId, key: &PublicKey) -> bool {
        self.pinned_key(peer).as_ref() == Some(key)
    }

    /// Every address to try for `peer`, best first.
    fn candidates(&self, peer: &DeviceId) -> Vec<Candidate> {
        let state = self.state.lock();
        self.candidates_in(&state, peer)
    }

    fn candidates_in(&self, state: &State, peer: &DeviceId) -> Vec<Candidate> {
        let mut found = Vec::new();
        let mut consider = |addr: SocketAddr, via: Option<u32>| {
            if self.loopback && addr.ip().is_loopback() {
                found.push(Candidate {
                    addr,
                    kind: PathKind::Lan,
                    interface: None,
                    interface_index: None,
                });
            } else if let Some(candidate) = path::classify(addr, via, &state.locals) {
                found.push(candidate);
            }
        };
        if let Some(seen) = state.seen.get(peer) {
            for (addr, via) in &seen.addrs {
                consider(*addr, *via);
            }
        }
        if let Some(paired) = state.paired(peer) {
            for addr in paired.remembered() {
                consider(addr, None);
            }
        }
        path::rank(&mut found);
        found
    }

    /// The kind of an established connection, by its local address.
    fn path_of(&self, local: SocketAddr, remote: SocketAddr) -> (PathKind, Option<String>) {
        if local.ip().is_loopback() {
            return (PathKind::Lan, None);
        }
        path::kind_of_local(local.ip(), remote.ip(), &self.state.lock().locals)
    }

    /// Writes `peers.json`; a failure is logged and reported, and the
    /// state in memory stands.
    fn save_peers(&self, state: &State) -> Result<(), Error> {
        let result = store::write(&self.data_dir.join(PEERS_FILE), &state.peers);
        if let Err(error) = &result {
            warn!(%error, "could not save the paired devices");
        }
        result
    }

    /// Pins `peer` after a pairing.
    fn pin(
        &self,
        peer: &DeviceId,
        key: PublicKey,
        name: &str,
        os: PeerOs,
        addr: Option<SocketAddr>,
    ) {
        let mut state = self.state.lock();
        state.peers.peers.retain(|paired| &paired.id != peer);
        let mut paired = PairedPeer {
            id: peer.clone(),
            public_key: key,
            name: store::valid_name(name).unwrap_or_else(|| peer.to_string()),
            os,
            paired_at: now_seconds(),
            addresses: Vec::new(),
            unknown: serde_json::Map::new(),
        };
        if let Some(addr) = addr {
            paired.remember(addr);
        }
        state.peers.peers.push(paired);
        state.retry.remove(peer);
        let _ = self.save_peers(&state);
        drop(state);
        info!(%peer, "paired");
        self.wake.notify_one();
        self.changed();
    }

    /// Forgets `peer` because it no longer pairs with this device: it said
    /// so in a session, or refused one.
    fn forget(&self, peer: &DeviceId) {
        let mut state = self.state.lock();
        let before = state.peers.peers.len();
        state.peers.peers.retain(|paired| &paired.id != peer);
        if state.peers.peers.len() == before {
            return;
        }
        if let Some(session) = state.sessions.remove(peer) {
            let _ = session.tx.send(SessionCommand::Close);
        }
        state.retry.remove(peer);
        let _ = self.save_peers(&state);
        drop(state);
        info!(%peer, "the other device unpaired");
        self.changed();
    }

    /// A paired device's new name, said in a session.
    fn peer_renamed(&self, peer: &DeviceId, name: &str) {
        let Some(name) = store::valid_name(name) else {
            return;
        };
        let mut state = self.state.lock();
        let Some(paired) = state.paired_mut(peer) else {
            return;
        };
        if paired.name == name {
            return;
        }
        paired.name = name;
        let _ = self.save_peers(&state);
        drop(state);
        self.changed();
    }

    /// mDNS saw a device.
    fn seen(&self, sighting: Sighting) {
        let mut state = self.state.lock();
        let paired = state.paired(&sighting.id).is_some();
        state.seen.insert(
            sighting.id.clone(),
            Seen {
                name: sighting.name,
                os: sighting.os,
                addrs: sighting.addrs,
            },
        );
        if paired {
            // News of a paired device: dial it now rather than at the end
            // of its backoff.
            state.retry.remove(&sighting.id);
            self.wake.notify_one();
        }
        drop(state);
        self.changed();
    }

    /// mDNS lost a device.
    fn gone(&self, id: &DeviceId) {
        if self.state.lock().seen.remove(id).is_some() {
            self.changed();
        }
    }

    /// Takes a new session with `peer`, unless the one already up is
    /// better; answers whether it was taken. The one it replaces is told to
    /// close.
    fn adopt(
        &self,
        peer: &DeviceId,
        conn: u64,
        link: &session::Link,
        tx: mpsc::UnboundedSender<SessionCommand>,
        name: &str,
        os: PeerOs,
    ) -> bool {
        let mut state = self.state.lock();
        let Some(paired) = state.paired_mut(peer) else {
            return false;
        };
        if let Some(name) = store::valid_name(name) {
            paired.name = name;
        }
        paired.os = os;
        let remembered = paired.remember(link.addr);
        let me = self.identity.id();
        let dialed_by_lower = if link.dialed_by_me {
            me < peer
        } else {
            peer < me
        };
        let rank = Session::rank(link.kind, dialed_by_lower);
        if let Some(old) = state.sessions.get(peer) {
            if rank > Session::rank(old.kind, old.dialed_by_lower) {
                return false;
            }
            let _ = old.tx.send(SessionCommand::Close);
        }
        state.sessions.insert(
            peer.clone(),
            Session {
                conn,
                kind: link.kind,
                interface: link.interface.clone(),
                addr: link.addr,
                dialed_by_lower,
                version: link.version,
                tx,
            },
        );
        state.retry.remove(peer);
        if remembered {
            let _ = self.save_peers(&state);
        }
        drop(state);
        info!(%peer, kind = ?link.kind, addr = %link.addr, "session up");
        self.changed();
        true
    }

    /// A session ended; it is dropped from the map only if it is still the
    /// one there (a replaced session ends after its successor took over).
    fn session_ended(&self, peer: &DeviceId, conn: u64) {
        let mut state = self.state.lock();
        if state
            .sessions
            .get(peer)
            .is_some_and(|session| session.conn == conn)
        {
            state.sessions.remove(peer);
            drop(state);
            info!(%peer, "session down");
            self.wake.notify_one();
            self.changed();
        }
    }

    /// Paired devices to dial now: each without a session whose backoff
    /// has passed, and each with a session on the LAN while a cable
    /// candidate exists (at most every [`UPGRADE_EVERY`]). Marks them as
    /// dialling.
    fn due_dials(&self) -> Vec<(DeviceId, Option<PathKind>)> {
        let mut state = self.state.lock();
        let now = Instant::now();
        let ids: Vec<DeviceId> = state
            .peers
            .peers
            .iter()
            .map(|peer| peer.id.clone())
            .collect();
        let mut due = Vec::new();
        for id in ids {
            if state.dialing.contains(&id) {
                continue;
            }
            let candidates = self.candidates_in(&state, &id);
            if candidates.is_empty() {
                continue;
            }
            let session_kind = state.sessions.get(&id).map(|session| session.kind);
            let retry = state.retry.entry(id.clone()).or_default();
            let dial = match session_kind {
                Some(PathKind::Direct) => None,
                Some(PathKind::Lan) => {
                    let has_cable = candidates.iter().any(|c| c.kind == PathKind::Direct);
                    let upgrade_due = retry
                        .last_upgrade
                        .is_none_or(|last| now.duration_since(last) >= UPGRADE_EVERY);
                    (has_cable && upgrade_due).then(|| {
                        retry.last_upgrade = Some(now);
                        Some(PathKind::Direct)
                    })
                }
                None => (now >= retry.next).then_some(None),
            };
            if let Some(only) = dial {
                state.dialing.insert(id.clone());
                due.push((id, only));
            }
        }
        due
    }

    /// A dial finished; a failure waits longer before the next one.
    fn dialed(&self, peer: &DeviceId, failed: bool, upgrade: bool) {
        let mut state = self.state.lock();
        state.dialing.remove(peer);
        if failed && !upgrade {
            let retry = state.retry.entry(peer.clone()).or_default();
            retry.next = Instant::now() + retry.delay;
            retry.delay = (retry.delay * 2).min(RETRY_MAX);
        }
    }

    fn set_pair_stage(&self, serial: u64, stage: PairStage) {
        let mut state = self.state.lock();
        if let Some(pairing) = state.pairing.as_mut().filter(|p| p.serial == serial) {
            pairing.stage = stage;
            drop(state);
            self.changed();
        }
    }

    /// Records how pairing `serial` ended, if it is still the one shown.
    fn pairing_finished(&self, serial: u64, outcome: Result<(), PairFailure>) {
        let mut state = self.state.lock();
        if let Some(pairing) = state.pairing.as_mut().filter(|p| p.serial == serial) {
            match outcome {
                Ok(()) => pairing.stage = PairStage::Paired,
                Err(failure) => {
                    pairing.stage = PairStage::Failed;
                    pairing.failure = Some(failure);
                }
            }
            pairing.code_tx = None;
            pairing.accept_tx = None;
            drop(state);
            self.changed();
        }
    }

    /// Shows `code` for pairing `serial`, now waiting on a user's
    /// confirmation: this one's, through `accept_tx`, when it is incoming.
    fn show_comparison(&self, serial: u64, code: PairCode, accept_tx: Option<oneshot::Sender<()>>) {
        let mut state = self.state.lock();
        if let Some(pairing) = state.pairing.as_mut().filter(|p| p.serial == serial) {
            pairing.code = Some(code);
            pairing.stage = PairStage::Confirm;
            pairing.accept_tx = accept_tx;
            drop(state);
            self.changed();
        }
    }

    /// Starts `peer`'s pairing request, unless a pairing is already running
    /// or the last request came under two seconds ago (one guess per code,
    /// and not many codes a minute). By code, it shows a fresh one at once;
    /// by confirmation, the code comes once the nonces are exchanged.
    fn begin_incoming_pair(
        &self,
        peer: &DeviceId,
        name: &str,
        os: PeerOs,
        by_code: bool,
    ) -> Option<(u64, Option<PairCode>, watch::Receiver<bool>)> {
        let mut state = self.state.lock();
        let now = Instant::now();
        let busy = state.pairing.as_ref().is_some_and(Pairing::is_active)
            || state
                .last_incoming_pair
                .is_some_and(|last| now.duration_since(last) < Duration::from_secs(2));
        if busy {
            return None;
        }
        let code = if by_code {
            Some(PairCode::random().ok()?)
        } else {
            None
        };
        let serial = self.next_serial();
        let (cancel, cancel_rx) = watch::channel(false);
        state.last_incoming_pair = Some(now);
        state.pairing = Some(Pairing {
            serial,
            role: PairRole::Incoming,
            peer: peer.clone(),
            peer_name: store::valid_name(name).unwrap_or_else(|| peer.to_string()),
            peer_os: os,
            code: code.clone(),
            stage: if by_code {
                PairStage::Code
            } else {
                PairStage::Connecting
            },
            failure: None,
            code_tx: None,
            accept_tx: None,
            cancel,
        });
        drop(state);
        self.changed();
        Some((serial, code, cancel_rx))
    }

    /// Updates transfer `id` in place.
    fn update_transfer(&self, id: &str, update: impl FnOnce(&mut Transfer)) {
        let mut state = self.state.lock();
        if let Some(transfer) = state.transfer_mut(id) {
            update(transfer);
            drop(state);
            self.changed();
        }
    }

    /// Records how transfer `id` ended.
    fn finish_transfer(&self, id: &str, outcome: Outcome, saved_to: Option<PathBuf>) {
        self.update_transfer(id, |transfer| {
            transfer.speed = 0;
            match outcome {
                Outcome::Done => {
                    transfer.state = TransferState::Done;
                    transfer.done.store(transfer.total, Ordering::Relaxed);
                }
                Outcome::Canceled => transfer.state = TransferState::Canceled,
                Outcome::Failed(failure) => {
                    transfer.state = TransferState::Failed;
                    transfer.failure = Some(failure);
                }
            }
            if saved_to.is_some() {
                transfer.saved_to = saved_to;
            }
        });
    }

    /// Refreshes each running transfer's speed about once a second;
    /// answers whether any is running.
    fn sample_speeds(&self) -> bool {
        let mut state = self.state.lock();
        let now = Instant::now();
        let mut running = false;
        for transfer in &mut state.transfers {
            if transfer.state != TransferState::Running {
                continue;
            }
            running = true;
            let elapsed = now.duration_since(transfer.sample.0);
            if elapsed < Duration::from_millis(900) {
                continue;
            }
            let done = transfer.done.load(Ordering::Relaxed);
            let moved = done.saturating_sub(transfer.sample.1);
            let rate = (moved as f64 / elapsed.as_secs_f64()) as u64;
            // A light smoothing, so the figure does not jump every second.
            transfer.speed = if transfer.speed == 0 {
                rate
            } else {
                (transfer.speed + rate * 3) / 4
            };
            transfer.sample = (now, done);
        }
        running
    }

    fn status(&self) -> LinkStatus {
        let state = self.state.lock();
        let paths = state
            .locals
            .iter()
            .flat_map(|local| {
                local.v4.iter().map(move |(ip, _)| LocalPath {
                    interface: local.name.clone(),
                    address: ip.to_string(),
                    kind: local.kind(),
                })
            })
            .collect();
        let me = Me {
            id: self.identity.id().clone(),
            name: self.name(&state.settings),
            os: self.os,
            port: self.port,
            paths,
        };
        let mut peers: Vec<PeerView> = Vec::new();
        for paired in &state.peers.peers {
            let session = state.sessions.get(&paired.id);
            peers.push(PeerView {
                id: paired.id.clone(),
                name: paired.name.clone(),
                os: paired.os,
                paired: true,
                online: session.is_some(),
                connection: session.map(|session| ConnectionView {
                    kind: session.kind,
                    interface: session.interface.clone(),
                    address: session.addr.ip().to_string(),
                }),
                routes: self.routes(&state, &paired.id),
            });
        }
        for (id, seen) in &state.seen {
            if state.paired(id).is_some() {
                continue;
            }
            peers.push(PeerView {
                id: id.clone(),
                name: seen.name.clone(),
                os: seen.os,
                paired: false,
                online: true,
                connection: None,
                routes: self.routes(&state, id),
            });
        }
        peers.sort_by(|a, b| {
            (!a.paired, !a.online, a.name.to_lowercase(), a.id.as_str()).cmp(&(
                !b.paired,
                !b.online,
                b.name.to_lowercase(),
                b.id.as_str(),
            ))
        });
        let pairing = state.pairing.as_ref().map(|pairing| PairingView {
            role: pairing.role,
            peer_id: pairing.peer.clone(),
            peer_name: pairing.peer_name.clone(),
            peer_os: pairing.peer_os,
            code: pairing
                .code
                .as_ref()
                .filter(|_| matches!(pairing.stage, PairStage::Code | PairStage::Confirm))
                .map(|code| code.as_str().to_owned()),
            stage: pairing.stage,
            failure: pairing.failure,
        });
        let transfers = state
            .transfers
            .iter()
            .map(|transfer| TransferView {
                id: transfer.id.clone(),
                peer_id: transfer.peer.clone(),
                peer_name: transfer.peer_name.clone(),
                direction: transfer.direction,
                name: transfer.name.clone(),
                files: transfer.files,
                total_bytes: transfer.total,
                done_bytes: transfer.done.load(Ordering::Relaxed).min(transfer.total),
                bytes_per_second: transfer.speed,
                state: transfer.state,
                path: transfer.path,
                interface: transfer.interface.clone(),
                failure: transfer.failure,
                retry: transfer.retry.clone(),
                saved_to: transfer
                    .saved_to
                    .as_ref()
                    .map(|path| path.display().to_string()),
                started_at: transfer.started_at,
            })
            .collect();
        LinkStatus {
            me,
            receive_dir: state
                .settings
                .receive_dir
                .clone()
                .unwrap_or_else(|| self.default_receive_dir.clone())
                .display()
                .to_string(),
            peers,
            pairing,
            transfers,
            clipboard: ClipboardView {
                enabled: state.settings.shares_clipboard(),
                last: state.clipboard.clone(),
            },
        }
    }

    /// The kinds of path `peer` can be reached on, best first.
    fn routes(&self, state: &State, peer: &DeviceId) -> Vec<PathKind> {
        let mut kinds: Vec<PathKind> = self
            .candidates_in(state, peer)
            .into_iter()
            .map(|candidate| candidate.kind)
            .collect();
        kinds.dedup();
        kinds
    }
}

/// The listeners: IPv4 on the preferred port (any free port if it is
/// taken), and IPv6 on the same port when the system allows. Loopback only
/// for tests.
async fn listen(preferred: u16, loopback: bool) -> std::io::Result<(Vec<TcpListener>, u16)> {
    let ip = if loopback {
        Ipv4Addr::LOCALHOST
    } else {
        Ipv4Addr::UNSPECIFIED
    };
    let v4 = match TcpListener::bind((ip, preferred)).await {
        Ok(listener) => listener,
        Err(error) if preferred != 0 => {
            warn!(%error, preferred, "the usual port is taken; using another");
            TcpListener::bind((ip, 0)).await?
        }
        Err(error) => return Err(error),
    };
    let port = v4.local_addr()?.port();
    let mut listeners = vec![v4];
    if !loopback {
        match listen_v6(port) {
            Ok(v6) => listeners.push(v6),
            Err(error) => debug!(%error, "no IPv6 listener"),
        }
    }
    Ok((listeners, port))
}

/// An IPv6-only listener, so it does not clash with the IPv4 one.
fn listen_v6(port: u16) -> std::io::Result<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_only_v6(true)?;
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)).into())?;
    socket.listen(64)?;
    socket.set_nonblocking(true)?;
    TcpListener::from_std(socket.into())
}

/// Accepts connections until shutdown.
async fn accept(core: Arc<Core>, listener: TcpListener) {
    let mut shutdown = core.shutdown_signal();
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, from)) => {
                    core.handle.spawn(serve(core.clone(), stream, from));
                }
                Err(error) => {
                    debug!(%error, "accept failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
        }
    }
}

/// Answers one incoming connection: the handshake, the hello, then the
/// purpose's own exchange.
async fn serve(core: Arc<Core>, stream: TcpStream, from: SocketAddr) {
    let _ = stream.set_nodelay(true);
    let (kind, interface) = match stream.local_addr() {
        Ok(local) => core.path_of(local, from),
        Err(_) => (PathKind::Lan, None),
    };
    let (mut reader, mut writer, remote) = match wire::respond(stream, &core.identity).await {
        Ok(secured) => secured,
        Err(error) => {
            debug!(%from, %error, "handshake failed");
            return;
        }
    };
    let peer = DeviceId::of(&remote.public_key);
    let (purpose, name, os, port, version) =
        match reader.message_within(connect::HELLO_TIMEOUT).await {
            Ok(Message::Hello {
                purpose,
                name,
                os,
                port,
                version,
            }) => (purpose, name, os, port, version),
            _ => return,
        };
    let addr = SocketAddr::new(from.ip(), port);
    match purpose {
        Purpose::Session | Purpose::Transfer if !core.is_pinned(&peer, &remote.public_key) => {
            let _ = writer
                .send(&Message::Refused {
                    reason: Refusal::NotPaired,
                })
                .await;
        }
        Purpose::Session => {
            if writer.send(&core.welcome()).await.is_ok() {
                let link = session::Link {
                    kind,
                    interface,
                    addr,
                    dialed_by_me: false,
                    version,
                };
                session::run(core, peer, link, reader, writer, name, os).await;
            }
        }
        Purpose::Transfer => {
            if writer.send(&core.welcome()).await.is_ok() {
                receive::run(core, reader, writer, peer, kind, interface).await;
            }
        }
        Purpose::Pair => {
            let hello = pair::Hello {
                peer,
                name,
                os,
                addr,
                version,
            };
            pair::answer(core, reader, writer, remote, hello).await;
        }
        Purpose::Unknown => {
            let _ = writer
                .send(&Message::Refused {
                    reason: Refusal::Other,
                })
                .await;
        }
    }
}

/// Dials paired devices that need it, every tick or when woken.
async fn connector(core: Arc<Core>) {
    let mut shutdown = core.shutdown_signal();
    let mut tick = tokio::time::interval(CONNECT_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            _ = tick.tick() => {}
            () = core.wake.notified() => {}
        }
        let locals = path::local_interfaces();
        core.state.lock().locals = locals;
        for (peer, only) in core.due_dials() {
            core.handle.spawn(dial_session(core.clone(), peer, only));
        }
    }
}

/// Dials a session with `peer`, over `only` that kind of path when given
/// (moving a LAN session to the cable).
async fn dial_session(core: Arc<Core>, peer: DeviceId, only: Option<PathKind>) {
    let upgrade = only.is_some();
    let paths = only.map_or(connect::Paths::Any, connect::Paths::Only);
    match connect::open(&core, &peer, Purpose::Session, paths).await {
        Ok(opened) => {
            core.dialed(&peer, false, upgrade);
            let link = session::Link {
                kind: opened.kind,
                interface: opened.interface,
                addr: opened.addr,
                dialed_by_me: true,
                version: opened.version,
            };
            session::run(
                core,
                peer,
                link,
                opened.reader,
                opened.writer,
                opened.name,
                opened.os,
            )
            .await;
        }
        Err(connect::OpenError::Refused(Refusal::NotPaired)) => {
            core.dialed(&peer, true, upgrade);
            core.forget(&peer);
        }
        Err(error) => {
            debug!(%peer, ?error, upgrade, "could not open a session");
            core.dialed(&peer, true, upgrade);
        }
    }
}

/// Folds transfer progress into a hint at most every [`PROGRESS_TICK`].
async fn progress(core: Arc<Core>) {
    let mut shutdown = core.shutdown_signal();
    let mut tick = tokio::time::interval(PROGRESS_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            _ = tick.tick() => {}
        }
        let running = core.sample_speeds();
        if core.progress_dirty.swap(false, Ordering::Relaxed) || running {
            core.changed();
        }
    }
}

/// Feeds discovery's events into the state.
async fn discovered(core: Arc<Core>, mut events: mpsc::UnboundedReceiver<DiscoveryEvent>) {
    let mut shutdown = core.shutdown_signal();
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            event = events.recv() => match event {
                Some(DiscoveryEvent::Seen(sighting)) => core.seen(sighting),
                Some(DiscoveryEvent::Gone(id)) => core.gone(&id),
                None => break,
            },
        }
    }
}

/// Creates the receive folder and lists it once, so macOS asks for access
/// to Downloads now, while the user is at this computer, rather than when
/// the first file arrives; and removes staging folders that no transfer
/// will come back to.
fn prepare_receive_dir(dir: &Path) {
    if let Err(error) = std::fs::create_dir_all(dir) {
        warn!(dir = %dir.display(), %error, "the receive folder cannot be created");
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        warn!(dir = %dir.display(), "the receive folder cannot be read");
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_prefix(files::STAGING_PREFIX))
        else {
            continue;
        };
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > STALE_STAGING);
        if files::valid_transfer_id(id) && stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Seconds since 1970, 0 for a clock before it.
fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The longest preview a clipboard notice shows, in characters.
const CLIPBOARD_PREVIEW: usize = 48;

/// A copy that crossed, for the status: its first line, shortened.
fn clipboard_event(
    serial: u64,
    direction: Direction,
    peers: Vec<String>,
    text: &str,
) -> ClipboardEvent {
    let first = text.trim().lines().next().unwrap_or_default().trim();
    let mut preview: String = first.chars().take(CLIPBOARD_PREVIEW).collect();
    if first.chars().count() > CLIPBOARD_PREVIEW || text.trim().lines().nth(1).is_some() {
        preview.push('…');
    }
    ClipboardEvent {
        serial,
        direction,
        peers,
        chars: text.chars().count() as u64,
        preview,
    }
}
