//! Sending a transfer: offer, stream every file from where the receiver
//! says it already is, and on a dropped or stalled connection do it again
//! over the best path left.
//!
//! The receiver answers an offer with the bytes it already holds of each
//! entry, so a retry after a drop (the cable pulled mid-file, say) carries
//! on from there over the LAN instead of starting again. A file whose
//! length changed since the offer fails the transfer: what would arrive is
//! neither the old file nor the new one.
//!
//! A path that dies without a word (Wi-Fi dropping, the other computer
//! sleeping) leaves TCP retransmitting for minutes before the system calls
//! the connection broken, so the sender does not wait for that: a write
//! that moves nothing for [`STALL`] ends the attempt, and the next one tries
//! the other paths first. Retries start fast and keep going while there is
//! hope, for [`RESUME_FOR`] after the last byte moved, and each one records
//! why it happened for the window to show.

use std::io::SeekFrom;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::watch;
use tracing::{debug, info};

use super::connect::{self, OpenError, Paths};
use super::{Core, Outcome};
use crate::files::Collected;
use crate::identity::DeviceId;
use crate::path::PathKind;
use crate::status::{RetryReason, RetryView, TransferFailure, TransferState};
use crate::wire::{MAX_BLOB, Message, Purpose, Refusal, WireError};

/// How much of a file is read, and handed to the connection, at a time.
const READ_CHUNK: usize = 256 * 1024;

/// How long a write may make no progress before the path counts as dead:
/// a quarter megabyte in ten seconds is far below any working network.
pub(super) const STALL: Duration = Duration::from_secs(10);

/// How long a transfer keeps retrying after bytes last moved.
pub(super) const RESUME_FOR: Duration = Duration::from_secs(120);

/// How long the receiver may take to answer an offer.
const OFFER_WAIT: Duration = Duration::from_secs(30);

/// How long the receiver may take to put everything in place at the end.
const FINISH_WAIT: Duration = Duration::from_secs(120);

/// The pause before retry `n` (from 1): a quarter second first, since a
/// dropped path usually has a working one beside it, then longer, never
/// more than three seconds.
fn retry_delay(retry: u32) -> Duration {
    match retry {
        0 | 1 => Duration::from_millis(250),
        2 => Duration::from_millis(500),
        3 => Duration::from_secs(1),
        4 => Duration::from_secs(2),
        _ => Duration::from_secs(3),
    }
}

/// What went wrong on one attempt, and where.
#[derive(Debug, Clone)]
struct Problem {
    reason: RetryReason,
    path: Option<PathKind>,
    interface: Option<String>,
    detail: Option<String>,
}

impl Problem {
    fn new(reason: RetryReason) -> Self {
        Self {
            reason,
            path: None,
            interface: None,
            detail: None,
        }
    }

    /// The failure a transfer ends with when this was its last trouble.
    fn failure(&self) -> TransferFailure {
        match self.reason {
            RetryReason::Unreachable => TransferFailure::Unreachable,
            RetryReason::Busy => TransferFailure::Refused,
            RetryReason::Stalled | RetryReason::Lost => TransferFailure::Lost,
        }
    }
}

/// Why one attempt stopped.
enum Stop {
    /// The user canceled here.
    Canceled,
    /// Worth another attempt.
    Retry(Problem),
    /// Final.
    Fail(TransferFailure),
}

/// Runs transfer `id` to `peer` until it is done, canceled or out of hope.
pub(super) async fn run(
    core: Arc<Core>,
    id: String,
    peer: DeviceId,
    collected: Collected,
    done: Arc<AtomicU64>,
    mut cancel: watch::Receiver<bool>,
) {
    let mut retries = 0;
    let mut paths = Paths::Any;
    let mut moved = done.load(Ordering::Relaxed);
    let mut give_up_at = Instant::now() + RESUME_FOR;
    let outcome = loop {
        if *cancel.borrow() {
            break Outcome::Canceled;
        }
        let stop = match attempt_once(&core, &id, &peer, &collected, &done, &cancel, paths).await {
            Ok(()) => break Outcome::Done,
            Err(stop) => stop,
        };
        let problem = match stop {
            Stop::Canceled => break Outcome::Canceled,
            Stop::Fail(failure) => break Outcome::Failed(failure),
            Stop::Retry(problem) => problem,
        };
        let now = Instant::now();
        let progress = done.load(Ordering::Relaxed);
        if progress > moved {
            // Bytes moved since the last trouble: a fresh start.
            moved = progress;
            retries = 0;
            give_up_at = now + RESUME_FOR;
        }
        retries += 1;
        info!(
            transfer = %id,
            retry = retries,
            reason = ?problem.reason,
            path = ?problem.path,
            interface = problem.interface.as_deref().unwrap_or("-"),
            detail = problem.detail.as_deref().unwrap_or("-"),
            "the transfer is trying again"
        );
        let view = RetryView {
            attempt: retries,
            reason: problem.reason,
            path: problem.path,
            interface: problem.interface.clone(),
            detail: problem.detail.clone(),
        };
        if now >= give_up_at {
            core.update_transfer(&id, |transfer| transfer.retry = Some(view));
            break Outcome::Failed(problem.failure());
        }
        core.update_transfer(&id, |transfer| {
            transfer.state = TransferState::Retrying;
            transfer.retry = Some(view);
        });
        // The path that went quiet is tried last next time; one that broke
        // outright gets no such doubt.
        paths = match (problem.reason, problem.path) {
            (RetryReason::Stalled, Some(kind)) => Paths::AvoidFirst(kind),
            _ => Paths::Any,
        };
        tokio::select! {
            () = tokio::time::sleep(retry_delay(retries)) => {}
            _ = cancel.changed() => {}
        }
    };
    core.finish_transfer(&id, outcome, None);
}

/// The system's whole account of a connection failure: the wire's sentence
/// and every cause below it ("the connection failed: Connection reset by
/// peer").
fn detail(error: &WireError) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

async fn attempt_once(
    core: &Arc<Core>,
    id: &str,
    peer: &DeviceId,
    collected: &Collected,
    done: &AtomicU64,
    cancel: &watch::Receiver<bool>,
    paths: Paths,
) -> Result<(), Stop> {
    let opened = connect::open(core, peer, Purpose::Transfer, paths)
        .await
        .map_err(|error| match error {
            OpenError::Unreachable => Stop::Retry(Problem::new(RetryReason::Unreachable)),
            OpenError::Mismatch => Stop::Fail(TransferFailure::NotPaired),
            OpenError::Refused(Refusal::NotPaired) => {
                core.forget(peer);
                Stop::Fail(TransferFailure::NotPaired)
            }
            OpenError::Refused(reason) => refusal(reason),
            OpenError::Lost(error) => {
                debug!(%error, "the transfer connection failed");
                Stop::Retry(Problem {
                    detail: Some(detail(&error)),
                    ..Problem::new(RetryReason::Lost)
                })
            }
        })?;
    let connect::Opened {
        mut reader,
        mut writer,
        kind,
        interface,
        ..
    } = opened;
    let at = |reason: RetryReason, detail: Option<String>| {
        Stop::Retry(Problem {
            reason,
            path: Some(kind),
            interface: interface.clone(),
            detail,
        })
    };
    let lost = |error: WireError| at(RetryReason::Lost, Some(detail(&error)));
    let stalled = || {
        at(
            RetryReason::Stalled,
            Some(format!("nothing moved for {} s", STALL.as_secs())),
        )
    };
    // A write that makes no progress within STALL ends the attempt.
    macro_rules! within_stall {
        ($write:expr) => {
            match tokio::time::timeout(STALL, $write).await {
                Ok(Ok(value)) => value,
                Ok(Err(error)) => return Err(lost(error)),
                Err(_) => return Err(stalled()),
            }
        };
    }

    within_stall!(writer.send(&Message::Offer {
        id: id.to_owned(),
        files: collected.files(),
        total: collected.total(),
    }));
    within_stall!(writer.send_blob(&collected.entries));
    match reader.message_within(OFFER_WAIT).await {
        Ok(Message::Accept) => {}
        Ok(Message::Refused { reason }) => return Err(refusal(reason)),
        Ok(_) => return Err(Stop::Fail(TransferFailure::Refused)),
        Err(WireError::Timeout) => return Err(stalled()),
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
        transfer.interface = interface.clone();
        transfer.retry = None;
        transfer.sample = (Instant::now(), already);
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
        within_stall!(writer.send(&Message::FileStart { index, offset }));
        let mut remaining = entry.size - offset;
        while remaining > 0 {
            if *cancel.borrow() {
                let _ = tokio::time::timeout(STALL, writer.send(&Message::Cancel)).await;
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
            within_stall!(writer.send_data(data));
            let read = read as u64;
            remaining -= read;
            done.fetch_add(read, Ordering::Relaxed);
            core.progressed();
        }
    }
    within_stall!(writer.send(&Message::Done));
    match reader.message_within(FINISH_WAIT).await {
        Ok(Message::Finished) => {
            writer.close().await;
            Ok(())
        }
        Ok(Message::Cancel) => Err(Stop::Fail(TransferFailure::CanceledByPeer)),
        Ok(Message::Refused { reason }) => Err(refusal(reason)),
        Ok(_) => Err(Stop::Fail(TransferFailure::Refused)),
        Err(WireError::Timeout) => Err(stalled()),
        Err(error) => Err(lost(error)),
    }
}

/// What a receiver's refusal means for the sender.
fn refusal(reason: Refusal) -> Stop {
    match reason {
        Refusal::NotPaired => Stop::Fail(TransferFailure::NotPaired),
        // Busy with a pairing does not stop a transfer, but say so if it
        // ever comes back; try again.
        Refusal::Busy => Stop::Retry(Problem::new(RetryReason::Busy)),
        Refusal::Canceled => Stop::Fail(TransferFailure::CanceledByPeer),
        Refusal::Disk => Stop::Fail(TransferFailure::Disk),
        Refusal::Invalid | Refusal::Other => Stop::Fail(TransferFailure::Refused),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Retries start within a quarter second and never wait more than three
    /// seconds, so a working second path takes over at once and a returning
    /// one is found soon.
    #[test]
    fn retries_start_fast_and_stay_short() {
        assert_eq!(retry_delay(1), Duration::from_millis(250));
        assert!((1..50).all(|n| retry_delay(n) <= Duration::from_secs(3)));
        assert!((1..50).all(|n| retry_delay(n) <= retry_delay(n + 1)));
    }

    /// An unreachable device fails as unreachable; a stalled or broken path
    /// that never came back fails as lost.
    #[test]
    fn the_last_trouble_names_the_failure() {
        assert_eq!(
            Problem::new(RetryReason::Unreachable).failure(),
            TransferFailure::Unreachable
        );
        assert_eq!(
            Problem::new(RetryReason::Stalled).failure(),
            TransferFailure::Lost
        );
    }
}
