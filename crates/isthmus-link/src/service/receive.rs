//! Receiving a transfer: check the offer, say how much of each file is
//! already here, write what arrives into the staging folder, and move
//! everything into place once all of it is complete.
//!
//! A receiver keeps its staging folder when the connection drops, so the
//! sender's retry (over another path, perhaps) continues from there; until
//! then the transfer shows as waiting for the sender, with the reason, and
//! fails only once the sender would have given up. A retry may arrive
//! before this side noticed the drop; the newer connection takes the
//! transfer over and the older one stops at its next frame. An offer for a transfer this side already finished is answered
//! with "all here", so a sender that missed the last reply does not make a
//! second copy.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncSeekExt, AsyncWriteExt, BufWriter};
use tracing::warn;

use super::connect::{Reader, Writer};
use super::{Core, Outcome, Transfer};
use crate::files::{self, Entry, MAX_ENTRIES};
use crate::identity::DeviceId;
use crate::path::PathKind;
use crate::status::{Direction, RetryReason, RetryView, TransferFailure, TransferState};
use crate::wire::{Frame, MAX_BLOB, Message, Refusal};

/// How long the sender may take to send its offer.
const OFFER_WAIT: Duration = Duration::from_secs(30);

/// How long a transfer may go without a frame: twice the sender's own
/// stall limit, so the sender gives up on a dead path first.
const IDLE: Duration = Duration::from_secs(20);

/// How long a receive waits for its sender to come back: as long as the
/// sender keeps trying, and a little more.
const RESUME_WAIT: Duration = Duration::from_secs(135);

/// How much is buffered before a write to disk.
const WRITE_BUFFER: usize = 1024 * 1024;

/// An offer, checked: every entry with a safe relative path.
#[derive(Clone)]
struct Plan {
    entries: Vec<Entry>,
    relative: Vec<PathBuf>,
}

/// What this side already knows about an offered transfer.
enum Prior {
    Fresh,
    Refuse(Refusal),
    AlreadyDone,
}

/// The file being written.
struct Current {
    file: BufWriter<tokio::fs::File>,
    remaining: u64,
}

/// Receives one transfer from `peer`.
pub(super) async fn run(
    core: Arc<Core>,
    mut reader: Reader,
    mut writer: Writer,
    peer: DeviceId,
    kind: PathKind,
    interface: Option<String>,
) {
    let (id, files, total) = match reader.message_within(OFFER_WAIT).await {
        Ok(Message::Offer { id, files, total }) => (id, files, total),
        _ => return,
    };
    let Ok(entries) = reader.blob::<Vec<Entry>>(MAX_BLOB).await else {
        return;
    };
    let Some(plan) = Plan::new(&id, entries) else {
        refuse(&mut writer, Refusal::Invalid).await;
        return;
    };
    match core.prior_receive(&id, &peer) {
        Prior::Fresh => {}
        Prior::Refuse(reason) => {
            refuse(&mut writer, reason).await;
            return;
        }
        Prior::AlreadyDone => {
            let sizes: Vec<u64> = plan.entries.iter().map(|entry| entry.size).collect();
            if writer.send(&Message::Accept).await.is_ok()
                && writer.send_blob(&sizes).await.is_ok()
                && matches!(reader.message_within(IDLE).await, Ok(Message::Done))
            {
                let _ = writer.send(&Message::Finished).await;
            }
            return;
        }
    }

    let receive_dir = core.receive_dir();
    let staging = files::staging_dir(&receive_dir, &id);
    let prepared = {
        let (plan, staging) = (plan.clone(), staging.clone());
        tokio::task::spawn_blocking(move || plan.prepare(&staging)).await
    };
    let Ok(Ok(offsets)) = prepared else {
        refuse(&mut writer, Refusal::Disk).await;
        return;
    };
    let already: u64 = plan
        .entries
        .iter()
        .zip(&offsets)
        .map(|(entry, offset)| (*offset).min(entry.size))
        .sum();
    let name = files::summary_name(&plan.entries);
    let receiving = core.begin_receive(
        &id,
        &peer,
        name,
        files,
        total,
        already,
        (kind, interface.clone()),
    );
    let (done, attempt, mine, mut cancel) = receiving;
    if writer.send(&Message::Accept).await.is_err() || writer.send_blob(&offsets).await.is_err() {
        core.finish_transfer(&id, Outcome::Failed(TransferFailure::Lost), None);
        return;
    }

    let superseded = || attempt.load(Ordering::Relaxed) != mine;
    let mut shutdown = core.shutdown_signal();
    let mut current: Option<Current> = None;
    loop {
        let frame = tokio::select! {
            biased;
            _ = cancel.changed() => {
                let _ = writer.send(&Message::Cancel).await;
                drop(current);
                discard(staging).await;
                core.finish_transfer(&id, Outcome::Canceled, None);
                return;
            }
            // Quitting keeps the staging folder for a resume after restart.
            _ = shutdown.changed() => return,
            frame = tokio::time::timeout(IDLE, reader.recv()) => frame,
        };
        if superseded() {
            return;
        }
        let frame = match frame {
            Ok(Ok(frame)) => frame,
            Ok(Err(error)) => {
                if let Some(current) = current.take() {
                    let _ = close(current).await;
                }
                let problem = RetryView {
                    attempt: 0,
                    reason: RetryReason::Lost,
                    path: Some(kind),
                    interface: interface.clone(),
                    detail: Some(error.to_string()),
                };
                core.await_resume(&id, mine, problem);
                return;
            }
            Err(_) => {
                if let Some(current) = current.take() {
                    let _ = close(current).await;
                }
                let problem = RetryView {
                    attempt: 0,
                    reason: RetryReason::Stalled,
                    path: Some(kind),
                    interface: interface.clone(),
                    detail: Some(format!("nothing arrived for {} s", IDLE.as_secs())),
                };
                core.await_resume(&id, mine, problem);
                return;
            }
        };
        match frame {
            Frame::Data(bytes) => {
                let Some(open) = current.as_mut() else {
                    core.finish_transfer(&id, Outcome::Failed(TransferFailure::Lost), None);
                    return;
                };
                let len = bytes.len() as u64;
                if len > open.remaining {
                    core.finish_transfer(&id, Outcome::Failed(TransferFailure::Lost), None);
                    return;
                }
                if open.file.write_all(bytes).await.is_err() {
                    disk_full(&core, &id, &mut writer, current.take(), staging).await;
                    return;
                }
                open.remaining -= len;
                done.fetch_add(len, Ordering::Relaxed);
                core.progressed();
            }
            Frame::Message(Message::FileStart { index, offset }) => {
                if let Some(open) = current.take()
                    && close(open).await.is_err()
                {
                    disk_full(&core, &id, &mut writer, None, staging).await;
                    return;
                }
                let index = usize::try_from(index).unwrap_or(usize::MAX);
                let target = plan
                    .entries
                    .get(index)
                    .zip(plan.relative.get(index))
                    .zip(offsets.get(index))
                    .filter(|((entry, _), expected)| !entry.dir && **expected == offset);
                let Some(((entry, relative), _)) = target else {
                    core.finish_transfer(&id, Outcome::Failed(TransferFailure::Lost), None);
                    return;
                };
                match open_at(&staging.join(relative), offset).await {
                    Ok(file) => {
                        current = Some(Current {
                            file: BufWriter::with_capacity(WRITE_BUFFER, file),
                            remaining: entry.size - offset,
                        });
                    }
                    Err(_) => {
                        disk_full(&core, &id, &mut writer, None, staging).await;
                        return;
                    }
                }
            }
            Frame::Message(Message::Done) => {
                if let Some(open) = current.take()
                    && close(open).await.is_err()
                {
                    disk_full(&core, &id, &mut writer, None, staging).await;
                    return;
                }
                let placed = {
                    let (plan, staging, receive_dir) =
                        (plan.clone(), staging.clone(), receive_dir.clone());
                    tokio::task::spawn_blocking(move || plan.place(&staging, &receive_dir)).await
                };
                match placed {
                    Ok(Ok(saved)) => {
                        let _ = writer.send(&Message::Finished).await;
                        core.finish_transfer(&id, Outcome::Done, Some(saved));
                    }
                    Ok(Err(Placing::Incomplete)) => {
                        core.finish_transfer(&id, Outcome::Failed(TransferFailure::Lost), None);
                    }
                    Ok(Err(Placing::Disk(error))) => {
                        warn!(%error, "could not put the received files in place");
                        disk_full(&core, &id, &mut writer, None, staging).await;
                    }
                    Err(_) => {
                        disk_full(&core, &id, &mut writer, None, staging).await;
                    }
                }
                return;
            }
            Frame::Message(Message::Cancel) => {
                drop(current);
                discard(staging).await;
                core.finish_transfer(&id, Outcome::Failed(TransferFailure::CanceledByPeer), None);
                return;
            }
            Frame::Message(_) => {
                core.finish_transfer(&id, Outcome::Failed(TransferFailure::Lost), None);
                return;
            }
        }
    }
}

async fn refuse(writer: &mut Writer, reason: Refusal) {
    let _ = writer.send(&Message::Refused { reason }).await;
}

/// The receiver cannot write: tell the sender, free the space the partial
/// files took, and fail.
async fn disk_full(
    core: &Core,
    id: &str,
    writer: &mut Writer,
    current: Option<Current>,
    staging: PathBuf,
) {
    drop(current);
    refuse(writer, Refusal::Disk).await;
    discard(staging).await;
    core.finish_transfer(id, Outcome::Failed(TransferFailure::Disk), None);
}

async fn discard(staging: PathBuf) {
    let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(staging)).await;
}

async fn open_at(path: &Path, offset: u64) -> std::io::Result<tokio::fs::File> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .await?;
    file.set_len(offset).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    Ok(file)
}

async fn close(mut current: Current) -> std::io::Result<()> {
    current.file.flush().await?;
    current.file.into_inner().flush().await
}

/// Why the files could not be put in place.
#[derive(Debug)]
enum Placing {
    /// A file is shorter than offered: the sender's `done` came early.
    Incomplete,
    Disk(std::io::Error),
}

impl Plan {
    /// The offer checked: a well-formed id, a bounded count, and every path
    /// safe.
    fn new(id: &str, entries: Vec<Entry>) -> Option<Self> {
        if !files::valid_transfer_id(id) || entries.is_empty() || entries.len() > MAX_ENTRIES {
            return None;
        }
        let relative = entries
            .iter()
            .map(|entry| files::safe_relative(&entry.path))
            .collect::<Option<Vec<_>>>()?;
        Some(Self { entries, relative })
    }

    /// Creates the staging tree and answers how many bytes of each entry it
    /// already holds; a file longer than offered starts again.
    fn prepare(&self, staging: &Path) -> std::io::Result<Vec<u64>> {
        std::fs::create_dir_all(staging)?;
        let mut offsets = Vec::with_capacity(self.entries.len());
        for (entry, relative) in self.entries.iter().zip(&self.relative) {
            let path = staging.join(relative);
            if entry.dir {
                std::fs::create_dir_all(&path)?;
                offsets.push(0);
                continue;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let offset = match std::fs::metadata(&path) {
                Ok(metadata) if metadata.is_file() && metadata.len() <= entry.size => {
                    metadata.len()
                }
                Ok(metadata) if metadata.is_dir() => {
                    return Err(std::io::ErrorKind::AlreadyExists.into());
                }
                Ok(_) => {
                    std::fs::File::create(&path)?;
                    0
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::File::create(&path)?;
                    0
                }
                Err(error) => return Err(error),
            };
            offsets.push(offset);
        }
        Ok(offsets)
    }

    /// Checks every file is whole, then moves each top-level item into
    /// `receive_dir` under a free name. Answers the one item, or the folder
    /// when there are several.
    fn place(&self, staging: &Path, receive_dir: &Path) -> Result<PathBuf, Placing> {
        for (entry, relative) in self.entries.iter().zip(&self.relative) {
            if entry.dir {
                continue;
            }
            let length = std::fs::metadata(staging.join(relative))
                .map_err(Placing::Disk)?
                .len();
            if length != entry.size {
                return Err(Placing::Incomplete);
            }
        }
        let mut tops: Vec<(PathBuf, bool)> = Vec::new();
        for (entry, relative) in self.entries.iter().zip(&self.relative) {
            let mut components = relative.components();
            let Some(first) = components.next() else {
                continue;
            };
            let is_dir = entry.dir || components.next().is_some();
            let first = PathBuf::from(first.as_os_str());
            match tops.iter_mut().find(|(top, _)| *top == first) {
                Some((_, dir)) => *dir |= is_dir,
                None => tops.push((first, is_dir)),
            }
        }
        let mut saved = Vec::with_capacity(tops.len());
        for (top, is_dir) in tops {
            let name = top.to_string_lossy();
            let destination = files::free_destination(receive_dir, &name, is_dir);
            std::fs::rename(staging.join(&top), &destination).map_err(Placing::Disk)?;
            saved.push(destination);
        }
        let _ = std::fs::remove_dir_all(staging);
        Ok(match saved.len() {
            1 => saved.remove(0),
            _ => receive_dir.to_path_buf(),
        })
    }
}

impl Core {
    /// Receive `id`'s connection `attempt` broke: show it waiting for the
    /// sender with `problem`, and fail it as lost if no newer connection
    /// has taken it over within [`RESUME_WAIT`].
    fn await_resume(self: &Arc<Self>, id: &str, attempt: u64, mut problem: RetryView) {
        problem.attempt = u32::try_from(attempt).unwrap_or(u32::MAX);
        self.update_transfer(id, |transfer| {
            if transfer.attempt.load(Ordering::Relaxed) == attempt {
                transfer.state = TransferState::Retrying;
                transfer.speed = 0;
                transfer.retry = Some(problem);
            }
        });
        let core = self.clone();
        let id = id.to_owned();
        self.handle.spawn(async move {
            tokio::time::sleep(RESUME_WAIT).await;
            let still_waiting = core.state.lock().transfers.iter().any(|transfer| {
                transfer.id == id
                    && transfer.direction == Direction::Receive
                    && transfer.state == TransferState::Retrying
                    && transfer.attempt.load(Ordering::Relaxed) == attempt
            });
            if still_waiting {
                core.finish_transfer(&id, Outcome::Failed(TransferFailure::Lost), None);
            }
        });
    }

    fn prior_receive(&self, id: &str, peer: &DeviceId) -> Prior {
        let state = self.state.lock();
        let known = state.transfers.iter().find(|transfer| {
            transfer.id == id && transfer.direction == Direction::Receive && &transfer.peer == peer
        });
        match known.map(|transfer| (transfer.state, transfer.failure)) {
            None => Prior::Fresh,
            Some((TransferState::Canceled, _)) => Prior::Refuse(Refusal::Canceled),
            Some((TransferState::Failed, Some(TransferFailure::Disk))) => {
                Prior::Refuse(Refusal::Disk)
            }
            Some((TransferState::Done, _)) => Prior::AlreadyDone,
            Some(_) => Prior::Fresh,
        }
    }

    /// Starts (or resumes) receiving transfer `id`: answers its progress
    /// counter, its attempt counter and this attempt's number, and its
    /// cancel signal.
    #[expect(
        clippy::too_many_arguments,
        reason = "the offer's fields, each distinct"
    )]
    fn begin_receive(
        &self,
        id: &str,
        peer: &DeviceId,
        name: String,
        files: u32,
        total: u64,
        already: u64,
        (kind, interface): (PathKind, Option<String>),
    ) -> (
        Arc<AtomicU64>,
        Arc<AtomicU64>,
        u64,
        tokio::sync::watch::Receiver<bool>,
    ) {
        let mut state = self.state.lock();
        let now = Instant::now();
        if let Some(transfer) = state.transfer_mut(id) {
            transfer.state = TransferState::Running;
            transfer.path = Some(kind);
            transfer.interface = interface;
            transfer.failure = None;
            transfer.retry = None;
            transfer.done.store(already, Ordering::Relaxed);
            transfer.sample = (now, already);
            let mine = transfer.attempt.fetch_add(1, Ordering::Relaxed) + 1;
            let receiving = (
                transfer.done.clone(),
                transfer.attempt.clone(),
                mine,
                transfer.cancel.subscribe(),
            );
            drop(state);
            self.changed();
            return receiving;
        }
        let peer_name = state
            .paired(peer)
            .map_or_else(|| peer.to_string(), |paired| paired.name.clone());
        let (cancel, cancel_rx) = tokio::sync::watch::channel(false);
        let transfer = Transfer {
            id: id.to_owned(),
            peer: peer.clone(),
            peer_name,
            direction: Direction::Receive,
            name,
            files,
            total,
            done: Arc::new(AtomicU64::new(already)),
            state: TransferState::Running,
            path: Some(kind),
            interface,
            failure: None,
            retry: None,
            saved_to: None,
            started_at: super::now_seconds(),
            cancel,
            speed: 0,
            sample: (now, already),
            attempt: Arc::new(AtomicU64::new(1)),
        };
        let receiving = (
            transfer.done.clone(),
            transfer.attempt.clone(),
            1,
            cancel_rx,
        );
        state.add_transfer(transfer);
        drop(state);
        self.changed();
        receiving
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScratchDir;

    const ID: &str = "0123456789abcdef0123456789abcdef";

    fn entry(path: &str, size: u64, dir: bool) -> Entry {
        Entry {
            path: path.to_owned(),
            size,
            dir,
        }
    }

    /// A resumed transfer continues from the bytes already staged; a file
    /// longer than offered (from another transfer, or changed) starts over,
    /// and a missing one starts at zero.
    #[test]
    fn prepare_reports_what_is_already_staged() {
        let scratch = ScratchDir::new("prepare");
        let staging = scratch.path().join("stage");
        std::fs::create_dir_all(staging.join("A")).unwrap();
        std::fs::write(staging.join("A").join("part.bin"), b"12345").unwrap();
        std::fs::write(staging.join("A").join("long.bin"), b"123456789").unwrap();
        let plan = Plan::new(
            ID,
            vec![
                entry("A", 0, true),
                entry("A/part.bin", 10, false),
                entry("A/long.bin", 4, false),
                entry("A/new/fresh.bin", 3, false),
            ],
        )
        .unwrap();
        assert_eq!(plan.prepare(&staging).unwrap(), vec![0, 5, 0, 0]);
        assert_eq!(
            std::fs::metadata(staging.join("A/long.bin")).unwrap().len(),
            0
        );
        assert!(staging.join("A/new/fresh.bin").is_file());
    }

    /// Nothing moves until every file is whole; then each top-level item
    /// lands under a free name and the staging folder is gone.
    #[test]
    fn place_moves_only_complete_transfers() {
        let scratch = ScratchDir::new("place");
        let receive = scratch.path().join("in");
        let staging = files::staging_dir(&receive, ID);
        let plan = Plan::new(ID, vec![entry("a.txt", 3, false), entry("b.txt", 1, false)]).unwrap();
        plan.prepare(&staging).unwrap();
        std::fs::write(staging.join("a.txt"), b"abc").unwrap();
        assert!(matches!(
            plan.place(&staging, &receive),
            Err(Placing::Incomplete)
        ));
        std::fs::write(staging.join("b.txt"), b"b").unwrap();
        std::fs::write(receive.join("a.txt"), b"older").unwrap();
        assert_eq!(plan.place(&staging, &receive).unwrap(), receive);
        assert_eq!(std::fs::read(receive.join("a (1).txt")).unwrap(), b"abc");
        assert_eq!(std::fs::read(receive.join("a.txt")).unwrap(), b"older");
        assert!(!staging.exists());
    }

    /// An offer with a bad id or an unsafe path is refused whole.
    #[test]
    fn unsafe_offers_are_refused() {
        assert!(Plan::new("../x", vec![entry("a", 1, false)]).is_none());
        assert!(Plan::new(ID, vec![entry("../a", 1, false)]).is_none());
        assert!(Plan::new(ID, Vec::new()).is_none());
    }
}
