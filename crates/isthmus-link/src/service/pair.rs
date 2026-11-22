//! Pairing over the network: the steps around [`crate::pairing`].
//!
//! The dialler (who asked) and the answerer (the device being added) open
//! with hello and welcome, then pair one of two ways, chosen by the
//! dialler's hello.
//!
//! **By confirmation** (the dialler speaks version 2): the answerer commits
//! to a nonce, the dialler sends its nonce, the answerer reveals its own,
//! and both show the same six digits. The answerer's user accepts (a
//! `PairResult { ok: true }` to the dialler) or declines (the connection
//! closes); the dialler pins the answerer and acknowledges, and the
//! answerer pins the dialler on that acknowledgement.
//!
//! **By code** (a version 1 dialler): the answerer says its code is up, the
//! dialler sends its SPAKE2 message once the user has typed the code, the
//! answerer answers with its own, the dialler confirms first, then the
//! answerer, and last the dialler says whether it accepted the answerer's
//! confirmation.
//!
//! Either way, each side pins the other only once it knows the pairing
//! succeeded, so a declined or failed pairing pins nothing anywhere. A new
//! dialler follows whichever way the answerer opens with, so it pairs with a
//! version 1 answerer by code.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{oneshot, watch};
use tracing::debug;

use super::Core;
use super::connect::{self, OpenError, Reader, Writer};
use crate::hex;
use crate::identity::DeviceId;
use crate::pairing::{self, Nonce, PairCode, Role};
use crate::status::{PairFailure, PairStage};
use crate::store::PeerOs;
use crate::wire::{CONFIRM_VERSION, Message, Purpose, Refusal, Remote, WireError};

/// How long the user has to type the code, or to accept.
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
    let opened = connect::open(core, peer, Purpose::Pair, connect::Paths::Any)
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
    let mut reader = opened.reader;
    let mut writer = opened.writer;
    let found = Found {
        remote: opened.remote,
        addr: opened.addr,
        name: opened.name,
        os: opened.os,
    };
    match reader.message_within(STEP).await {
        Ok(Message::PairCommit { commit }) => {
            let commit = hex::decode(&commit).ok_or(PairFailure::Lost)?;
            ask_by_confirmation(core, serial, peer, reader, writer, found, &commit).await
        }
        Ok(Message::PairCodeShown) => {
            ask_by_code(core, serial, peer, &mut reader, &mut writer, found, code).await
        }
        Ok(Message::Refused {
            reason: Refusal::Busy,
        }) => Err(PairFailure::Busy),
        _ => Err(PairFailure::Lost),
    }
}

/// What the dialler learned of the answerer while connecting.
struct Found {
    remote: Remote,
    addr: SocketAddr,
    name: String,
    os: PeerOs,
}

async fn ask_by_confirmation(
    core: &Arc<Core>,
    serial: u64,
    peer: &DeviceId,
    mut reader: Reader,
    mut writer: Writer,
    found: Found,
    commit: &[u8],
) -> Result<(), PairFailure> {
    core.set_pair_stage(serial, PairStage::Verifying);
    let mine = Nonce::random().map_err(|_| PairFailure::Lost)?;
    send(
        &mut writer,
        Message::PairNonce {
            nonce: hex::encode(mine.as_bytes()),
        },
    )
    .await?;
    let theirs = match reader.message_within(STEP).await {
        Ok(Message::PairReveal { nonce }) => hex::decode(&nonce)
            .and_then(|bytes| Nonce::from_bytes(&bytes))
            .ok_or(PairFailure::Lost)?,
        _ => return Err(PairFailure::Lost),
    };
    if !theirs.opens(commit) {
        // An answerer that changed its nonce after seeing this one's is
        // steering the digits.
        return Err(PairFailure::Mismatch);
    }
    let code = pairing::comparison_code(&found.remote.handshake_hash, &mine, &theirs);
    core.show_comparison(serial, code, None);
    match reader.message_within(CODE_WAIT + STEP).await {
        Ok(Message::PairResult { ok: true }) => {}
        Ok(Message::PairResult { ok: false }) | Err(WireError::Closed) => {
            return Err(PairFailure::Declined);
        }
        Err(WireError::Timeout) => return Err(PairFailure::TimedOut),
        _ => return Err(PairFailure::Lost),
    }
    core.pin(
        peer,
        found.remote.public_key,
        &found.name,
        found.os,
        Some(found.addr),
    );
    // The answerer pins this device when it reads this; if it never does,
    // its refusal of the next session unpairs this side again.
    let _ = writer.send(&Message::PairResult { ok: true }).await;
    writer.close().await;
    Ok(())
}

async fn ask_by_code(
    core: &Arc<Core>,
    serial: u64,
    peer: &DeviceId,
    reader: &mut Reader,
    writer: &mut Writer,
    found: Found,
    code: oneshot::Receiver<PairCode>,
) -> Result<(), PairFailure> {
    core.set_pair_stage(serial, PairStage::Code);
    let code = match tokio::time::timeout(CODE_WAIT, code).await {
        Ok(Ok(code)) => code,
        Ok(Err(_)) => return Err(PairFailure::Canceled),
        Err(_) => return Err(PairFailure::TimedOut),
    };
    core.set_pair_stage(serial, PairStage::Verifying);
    let hash = &found.remote.handshake_hash;

    let (exchange, mine) = pairing::start(Role::Initiator, &code, core.identity.id(), peer);
    send(
        writer,
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
        writer,
        Message::PairConfirm {
            tag: hex::encode(&key.confirmation(Role::Initiator, hash)),
        },
    )
    .await?;
    let tag = match reader.message_within(STEP).await {
        Ok(Message::PairConfirm { tag }) => hex::decode(&tag).unwrap_or_default(),
        Ok(Message::PairResult { ok: false }) => return Err(PairFailure::WrongCode),
        _ => return Err(PairFailure::Lost),
    };
    if !key.verify(Role::Responder, hash, &tag) {
        let _ = writer.send(&Message::PairResult { ok: false }).await;
        return Err(PairFailure::WrongCode);
    }
    core.pin(
        peer,
        found.remote.public_key,
        &found.name,
        found.os,
        Some(found.addr),
    );
    // The answerer pins this device when it reads this; if it never does,
    // its refusal of the next session unpairs this side again.
    let _ = writer.send(&Message::PairResult { ok: true }).await;
    writer.close().await;
    Ok(())
}

/// What the dialler said about itself, and where it can be reached.
pub(super) struct Hello {
    pub peer: DeviceId,
    pub name: String,
    pub os: PeerOs,
    pub addr: SocketAddr,
    /// The protocol version its hello said it speaks.
    pub version: u32,
}

/// Answers a pairing request: by confirmation for a version 2 dialler,
/// else by showing a code until the dialler sends its guess.
pub(super) async fn answer(
    core: Arc<Core>,
    reader: Reader,
    mut writer: Writer,
    remote: Remote,
    hello: Hello,
) {
    let by_code = hello.version < CONFIRM_VERSION;
    let Some((serial, code, mut cancel)) =
        core.begin_incoming_pair(&hello.peer, &hello.name, hello.os, by_code)
    else {
        let _ = writer
            .send(&Message::Refused {
                reason: Refusal::Busy,
            })
            .await;
        return;
    };
    let steps = async {
        match code {
            Some(code) => {
                answer_by_code(
                    &core,
                    serial,
                    reader,
                    &mut writer,
                    &remote,
                    &hello.peer,
                    &code,
                )
                .await
            }
            None => answer_by_confirmation(&core, serial, reader, &mut writer, &remote).await,
        }
    };
    let outcome = tokio::select! {
        outcome = steps => outcome,
        _ = cancel.changed() => Err(PairFailure::Canceled),
    };
    match outcome {
        Ok(()) => core.pin(
            &hello.peer,
            remote.public_key,
            &hello.name,
            hello.os,
            Some(hello.addr),
        ),
        Err(_) => writer.close().await,
    }
    core.pairing_finished(serial, outcome);
}

async fn answer_by_confirmation(
    core: &Arc<Core>,
    serial: u64,
    mut reader: Reader,
    writer: &mut Writer,
    remote: &Remote,
) -> Result<(), PairFailure> {
    send(writer, core.welcome()).await?;
    let mine = Nonce::random().map_err(|_| PairFailure::Lost)?;
    send(
        writer,
        Message::PairCommit {
            commit: hex::encode(&mine.commitment()),
        },
    )
    .await?;
    let theirs = match reader.message_within(STEP).await {
        Ok(Message::PairNonce { nonce }) => hex::decode(&nonce)
            .and_then(|bytes| Nonce::from_bytes(&bytes))
            .ok_or(PairFailure::Lost)?,
        _ => return Err(PairFailure::Lost),
    };
    send(
        writer,
        Message::PairReveal {
            nonce: hex::encode(mine.as_bytes()),
        },
    )
    .await?;
    let code = pairing::comparison_code(&remote.handshake_hash, &theirs, &mine);
    let (accept_tx, accept_rx) = oneshot::channel();
    core.show_comparison(serial, code, Some(accept_tx));
    // The user's answer, or the dialler hanging up (it canceled), whichever
    // comes first. Declining is a cancel, which the caller's select takes.
    tokio::select! {
        accepted = tokio::time::timeout(CODE_WAIT, accept_rx) => match accepted {
            Ok(Ok(())) => {}
            Ok(Err(_)) => return Err(PairFailure::Canceled),
            Err(_) => return Err(PairFailure::TimedOut),
        },
        message = reader.message_within(CODE_WAIT + STEP) => {
            return Err(match message {
                Err(WireError::Closed) => PairFailure::Canceled,
                Err(WireError::Timeout) => PairFailure::TimedOut,
                _ => PairFailure::Lost,
            });
        }
    }
    send(writer, Message::PairResult { ok: true }).await?;
    match reader.message_within(STEP).await {
        Ok(Message::PairResult { ok: true }) => Ok(()),
        Ok(Message::PairResult { ok: false }) => Err(PairFailure::Canceled),
        _ => Err(PairFailure::Lost),
    }
}

async fn answer_by_code(
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
