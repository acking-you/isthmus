//! Sending a transfer: offer, stream every file from where the receiver
//! says it already is, and on a dropped connection do it again over the
//! best path left.
//!
//! The receiver answers an offer with the bytes it already holds of each
//! entry, so a retry after a drop (the cable pulled mid-file, say) carries
//! on from there over the LAN instead of starting again. A file whose
//! length changed since the offer fails the transfer: what would arrive is
//! neither the old file nor the new one.

use std::io::SeekFrom;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::watch;
use tracing::debug;

use super::connect::{self, OpenError};
use super::{Core, Outcome};
use crate::files::Collected;
use crate::identity::DeviceId;
use crate::status::{TransferFailure, TransferState};
use crate::wire::{MAX_BLOB, Message, Purpose, Refusal, WireError};

/// How many times a transfer is tried before it fails.
const ATTEMPTS: u32 = 6;

/// How much of a file is read at a time.
const READ_CHUNK: usize = 1024 * 1024;

/// How long the receiver may take to answer an offer.
const OFFER_WAIT: Duration = Duration::from_secs(30);

/// How long the receiver may take to put everything in place at the end.
const FINISH_WAIT: Duration = Duration::from_secs(120);

/// Why one attempt stopped.
enum Stop {
    /// The user canceled here.
    Canceled,
    /// Worth another attempt, failing with this if none is left.
    Retry(TransferFailure),
    /// Final.
    Fail(TransferFailure),
}

/// Runs transfer `id` to `peer` until it is done, canceled or out of
/// attempts.
pub(super) async fn run(
    core: Arc<Core>,
    id: String,
    peer: DeviceId,
    collected: Collected,
    done: Arc<AtomicU64>,
    mut cancel: watch::Receiver<bool>,
) {
    let mut attempt = 0;
    let outcome = loop {
        if *cancel.borrow() {
            break Outcome::Canceled;
        }
        let stop = match attempt_once(&core, &id, &peer, &collected, &done, &cancel).await {
            Ok(()) => break Outcome::Done,
            Err(stop) => stop,
        };
        match stop {
            Stop::Canceled => break Outcome::Canceled,
            Stop::Fail(failure) => break Outcome::Failed(failure),
            Stop::Retry(failure) => {
                attempt += 1;
                if attempt >= ATTEMPTS {
                    break Outcome::Failed(failure);
                }
                core.update_transfer(&id, |transfer| transfer.state = TransferState::Retrying);
                let delay = Duration::from_millis(500) * 2u32.pow(attempt.min(5));
                tokio::select! {
                    () = tokio::time::sleep(delay) => {}
                    _ = cancel.changed() => {}
                }
            }
        }
    };
    core.finish_transfer(&id, outcome, None);
}

async fn attempt_once(
    core: &Arc<Core>,
    id: &str,
    peer: &DeviceId,
    collected: &Collected,
    done: &AtomicU64,
    cancel: &watch::Receiver<bool>,
) -> Result<(), Stop> {
    let opened = connect::open(core, peer, Purpose::Transfer, None)
        .await
        .map_err(|error| match error {
            OpenError::Unreachable => Stop::Retry(TransferFailure::Unreachable),
            OpenError::Mismatch => Stop::Fail(TransferFailure::NotPaired),
            OpenError::Refused(Refusal::NotPaired) => {
                core.forget(peer);
                Stop::Fail(TransferFailure::NotPaired)
            }
            OpenError::Refused(reason) => refusal(reason),
            OpenError::Lost(error) => {
                debug!(%error, "the transfer connection failed");
                Stop::Retry(TransferFailure::Lost)
            }
        })?;
    let connect::Opened {
        mut reader,
        mut writer,
        kind,
        ..
    } = opened;
    let lost = |_: WireError| Stop::Retry(TransferFailure::Lost);

    writer
        .send(&Message::Offer {
            id: id.to_owned(),
            files: collected.files(),
            total: collected.total(),
        })
        .await
        .map_err(lost)?;
    writer.send_blob(&collected.entries).await.map_err(lost)?;
    match reader.message_within(OFFER_WAIT).await {
        Ok(Message::Accept) => {}
        Ok(Message::Refused { reason }) => return Err(refusal(reason)),
        Ok(_) => return Err(Stop::Fail(TransferFailure::Refused)),
        Err(error) => return Err(lost(error)),
    }
    let offsets: Vec<u64> = reader.blob(MAX_BLOB).await.map_err(lost)?;
    if offsets.len() != collected.entries.len() {
        return Err(Stop::Fail(TransferFailure::Refused));
    }
    let already: u64 = collected
        .entries
        .iter()
        .zip(&offsets)
        .map(|(entry, offset)| (*offset).min(entry.size))
        .sum();
    done.store(already, Ordering::Relaxed);
    core.update_transfer(id, |transfer| {
        transfer.state = TransferState::Running;
        transfer.path = Some(kind);
        transfer.sample = (std::time::Instant::now(), already);
    });

    let mut buffer = vec![0; READ_CHUNK];
    for (index, (entry, source)) in collected.entries.iter().zip(&collected.sources).enumerate() {
        let (Some(source), Some(&offset)) = (source, offsets.get(index)) else {
            continue;
        };
        if offset >= entry.size {
            continue;
        }
        let source_failed = |_| Stop::Fail(TransferFailure::Source);
        let mut file = tokio::fs::File::open(source).await.map_err(source_failed)?;
        let length = file.metadata().await.map_err(source_failed)?.len();
        if length != entry.size {
            return Err(Stop::Fail(TransferFailure::Source));
        }
        file.seek(SeekFrom::Start(offset))
            .await
            .map_err(source_failed)?;
        let index = u32::try_from(index).map_err(|_| Stop::Fail(TransferFailure::Source))?;
        writer
            .send(&Message::FileStart { index, offset })
            .await
            .map_err(lost)?;
        let mut remaining = entry.size - offset;
        while remaining > 0 {
            if *cancel.borrow() {
                let _ = writer.send(&Message::Cancel).await;
                return Err(Stop::Canceled);
            }
            let want = usize::try_from(remaining)
                .unwrap_or(usize::MAX)
                .min(buffer.len());
            let Some(chunk) = buffer.get_mut(..want) else {
                return Err(Stop::Fail(TransferFailure::Source));
            };
            let read = file.read(chunk).await.map_err(source_failed)?;
            if read == 0 {
                // The file shrank after it was measured.
                return Err(Stop::Fail(TransferFailure::Source));
            }
            let Some(data) = chunk.get(..read) else {
                return Err(Stop::Fail(TransferFailure::Source));
            };
            writer.send_data(data).await.map_err(lost)?;
            let read = read as u64;
            remaining -= read;
            done.fetch_add(read, Ordering::Relaxed);
            core.progressed();
        }
    }
    writer.send(&Message::Done).await.map_err(lost)?;
    match reader.message_within(FINISH_WAIT).await {
        Ok(Message::Finished) => {
            writer.close().await;
            Ok(())
        }
        Ok(Message::Cancel) => Err(Stop::Fail(TransferFailure::CanceledByPeer)),
        Ok(Message::Refused { reason }) => Err(refusal(reason)),
        Ok(_) => Err(Stop::Fail(TransferFailure::Refused)),
        Err(error) => Err(lost(error)),
    }
}

/// What a receiver's refusal means for the sender.
fn refusal(reason: Refusal) -> Stop {
    match reason {
        Refusal::NotPaired => Stop::Fail(TransferFailure::NotPaired),
        // Busy with a pairing does not stop a transfer, but say so if it
        // ever comes back; try again.
        Refusal::Busy => Stop::Retry(TransferFailure::Refused),
        Refusal::Canceled => Stop::Fail(TransferFailure::CanceledByPeer),
        Refusal::Disk => Stop::Fail(TransferFailure::Disk),
        Refusal::Invalid | Refusal::Other => Stop::Fail(TransferFailure::Refused),
    }
}
