//! Pairing over the network: the steps around [`crate::pairing`].
//!
//! The dialler (who asked) and the answerer (who shows the code) speak in
//! a fixed order: hello and welcome, the answerer says its code is up, the
//! dialler sends its SPAKE2 message once the user has typed the code, the
//! answerer answers with its own, the dialler confirms first, then the
//! answerer, and last the dialler says whether it accepted the answerer's
//! confirmation. Each side pins the other only after checking the other's
//! confirmation, so a wrong code pins nothing anywhere.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{oneshot, watch};
use tracing::debug;

use super::Core;
use super::connect::{self, OpenError, Reader, Writer};
use crate::hex;
use crate::identity::DeviceId;
use crate::pairing::{self, PairCode, Role};
use crate::status::{PairFailure, PairStage};
use crate::store::PeerOs;
use crate::wire::{Message, Purpose, Refusal, Remote, WireError};

/// How long the user has to type the code.
const CODE_WAIT: Duration = Duration::from_secs(120);

/// How long each step after the code may take.
const STEP: Duration = Duration::from_secs(10);

/// Asks `peer` to pair and runs this side to the end.
pub(super) async fn ask(
    core: Arc<Core>,
    serial: u64,
    peer: DeviceId,
    code: oneshot::Receiver<PairCode>,
    mut cancel: watch::Receiver<bool>,
) {
    let outcome = tokio::select! {
        outcome = ask_steps(&core, serial, &peer, code) => outcome,
        _ = cancel.changed() => Err(PairFailure::Canceled),
    };
    core.pairing_finished(serial, outcome);
}

async fn ask_steps(
    core: &Arc<Core>,
    serial: u64,
    peer: &DeviceId,
    code: oneshot::Receiver<PairCode>,
) -> Result<(), PairFailure> {
    let opened = connect::open(core, peer, Purpose::Pair, None)
        .await
        .map_err(|error| match error {
            OpenError::Unreachable => PairFailure::Unreachable,
            OpenError::Mismatch => PairFailure::Mismatch,
            OpenError::Refused(Refusal::Busy) => PairFailure::Busy,
            OpenError::Refused(_) => PairFailure::Unreachable,
            OpenError::Lost(error) => {
                debug!(%error, "the pairing connection failed");
                PairFailure::Lost
            }
        })?;
    let connect::Opened {
        mut reader,
        mut writer,
        remote,
        addr,
        name,
        os,
        ..
    } = opened;
    match reader.message_within(STEP).await {
        Ok(Message::PairCodeShown) => {}
        Ok(Message::Refused {
            reason: Refusal::Busy,
        }) => return Err(PairFailure::Busy),
        _ => return Err(PairFailure::Lost),
    }
    core.set_pair_stage(serial, PairStage::Code);
    let code = match tokio::time::timeout(CODE_WAIT, code).await {
        Ok(Ok(code)) => code,
        Ok(Err(_)) => return Err(PairFailure::Canceled),
        Err(_) => return Err(PairFailure::TimedOut),
    };
    core.set_pair_stage(serial, PairStage::Verifying);

    let (exchange, mine) = pairing::start(Role::Initiator, &code, core.identity.id(), peer);
    send(
        &mut writer,
        Message::PairSpake {
            message: hex::encode(&mine),
        },
    )
    .await?;
    let theirs = match reader.message_within(STEP).await {
        Ok(Message::PairSpake { message }) => hex::decode(&message).ok_or(PairFailure::Lost)?,
        Ok(Message::PairResult { ok: false }) => return Err(PairFailure::WrongCode),
        _ => return Err(PairFailure::Lost),
    };
    let key = exchange.finish(&theirs).map_err(|_| PairFailure::Lost)?;
    send(
        &mut writer,
        Message::PairConfirm {
            tag: hex::encode(&key.confirmation(Role::Initiator, &remote.handshake_hash)),
        },
    )
    .await?;
    let tag = match reader.message_within(STEP).await {
        Ok(Message::PairConfirm { tag }) => hex::decode(&tag).unwrap_or_default(),
        Ok(Message::PairResult { ok: false }) => return Err(PairFailure::WrongCode),
        _ => return Err(PairFailure::Lost),
    };
    if !key.verify(Role::Responder, &remote.handshake_hash, &tag) {
        let _ = writer.send(&Message::PairResult { ok: false }).await;
        return Err(PairFailure::WrongCode);
    }
    core.pin(peer, remote.public_key, &name, os, Some(addr));
    // The answerer pins this device when it reads this; if it never does,
    // its refusal of the next session unpairs this side again.
    let _ = writer.send(&Message::PairResult { ok: true }).await;
    writer.close().await;
    Ok(())
}

/// Answers a pairing request from `peer`, showing a code until the dialler
/// sends its guess.
#[expect(
    clippy::too_many_arguments,
    reason = "one call site, each argument distinct"
)]
pub(super) async fn answer(
    core: Arc<Core>,
    reader: Reader,
    mut writer: Writer,
    remote: Remote,
    peer: DeviceId,
    name: String,
    os: PeerOs,
    addr: SocketAddr,
) {
    let Some((serial, code, mut cancel)) = core.begin_incoming_pair(&peer, &name, os) else {
        let _ = writer
            .send(&Message::Refused {
                reason: Refusal::Busy,
            })
            .await;
        return;
    };
    let outcome = tokio::select! {
        outcome = answer_steps(&core, serial, reader, &mut writer, &remote, &peer, &code) => outcome,
        _ = cancel.changed() => Err(PairFailure::Canceled),
    };
    match outcome {
        Ok(()) => core.pin(&peer, remote.public_key, &name, os, Some(addr)),
        Err(_) => writer.close().await,
    }
    core.pairing_finished(serial, outcome);
}

async fn answer_steps(
    core: &Arc<Core>,
    serial: u64,
    mut reader: Reader,
    writer: &mut Writer,
    remote: &Remote,
    peer: &DeviceId,
    code: &PairCode,
) -> Result<(), PairFailure> {
    send(writer, core.welcome()).await?;
    send(writer, Message::PairCodeShown).await?;
    let theirs = match reader.message_within(CODE_WAIT + STEP).await {
        Ok(Message::PairSpake { message }) => hex::decode(&message).ok_or(PairFailure::Lost)?,
        Err(WireError::Timeout) => return Err(PairFailure::TimedOut),
        // The dialler hung up while the code showed: it canceled.
        Err(WireError::Closed) => return Err(PairFailure::Canceled),
        _ => return Err(PairFailure::Lost),
    };
    core.set_pair_stage(serial, PairStage::Verifying);
    let (exchange, mine) = pairing::start(Role::Responder, code, peer, core.identity.id());
    send(
        writer,
        Message::PairSpake {
            message: hex::encode(&mine),
        },
    )
    .await?;
    let key = exchange.finish(&theirs).map_err(|_| PairFailure::Lost)?;
    let tag = match reader.message_within(STEP).await {
        Ok(Message::PairConfirm { tag }) => hex::decode(&tag).unwrap_or_default(),
        _ => return Err(PairFailure::Lost),
    };
    if !key.verify(Role::Initiator, &remote.handshake_hash, &tag) {
        let _ = writer.send(&Message::PairResult { ok: false }).await;
        return Err(PairFailure::WrongCode);
    }
    send(
        writer,
        Message::PairConfirm {
            tag: hex::encode(&key.confirmation(Role::Responder, &remote.handshake_hash)),
        },
    )
    .await?;
    match reader.message_within(STEP).await {
        Ok(Message::PairResult { ok: true }) => Ok(()),
        Ok(Message::PairResult { ok: false }) => Err(PairFailure::WrongCode),
        _ => Err(PairFailure::Lost),
    }
}

async fn send(writer: &mut Writer, message: Message) -> Result<(), PairFailure> {
    writer.send(&message).await.map_err(|_| PairFailure::Lost)
}
