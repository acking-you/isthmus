//! A session with a paired device: one long-lived connection that says the
//! device is there, carries renames and unpairings, and ends when either
//! side goes quiet for too long.
//!
//! The reader runs in a task of its own and hands messages over a channel,
//! so the loop can wait on messages, commands, the ping timer and shutdown
//! at once without ever dropping a half-read frame.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use super::{Core, SessionCommand};
use crate::identity::DeviceId;
use crate::path::PathKind;
use crate::store::PeerOs;
use crate::wire::{Message, SecureReader, SecureWriter};

/// How often each side pings.
const PING_EVERY: Duration = Duration::from_secs(5);

/// Silence this long ends the session: three pings missed.
const SILENCE_LIMIT: Duration = Duration::from_secs(16);

/// How a session runs.
#[derive(Debug, Clone)]
pub(crate) struct Link {
    pub kind: PathKind,
    pub interface: Option<String>,
    /// Where the peer listens.
    pub addr: SocketAddr,
    pub dialed_by_me: bool,
}

/// Runs a session until it ends, if the service takes it.
pub(crate) async fn run<R, W>(
    core: Arc<Core>,
    peer: DeviceId,
    link: Link,
    mut reader: SecureReader<R>,
    mut writer: SecureWriter<W>,
    name: String,
    os: PeerOs,
) where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin,
{
    let conn = core.next_serial();
    let (tx, mut commands) = mpsc::unbounded_channel();
    if !core.adopt(&peer, conn, &link, tx, &name, os) {
        writer.close().await;
        return;
    }

    let (message_tx, mut messages) = mpsc::channel(16);
    let reading = core.handle.spawn(async move {
        while let Ok(message) = reader.message().await {
            if message_tx.send(message).await.is_err() {
                break;
            }
        }
    });

    let mut shutdown = core.shutdown_signal();
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut heard = Instant::now();
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            command = commands.recv() => match command {
                None | Some(SessionCommand::Close) => break,
                Some(SessionCommand::Unpair) => {
                    let _ = writer.send(&Message::Unpair).await;
                    break;
                }
                Some(SessionCommand::Rename(name)) => {
                    if writer.send(&Message::Renamed { name }).await.is_err() {
                        break;
                    }
                }
            },
            message = messages.recv() => {
                let Some(message) = message else { break };
                heard = Instant::now();
                match message {
                    Message::Ping => {
                        if writer.send(&Message::Pong).await.is_err() {
                            break;
                        }
                    }
                    Message::Unpair => {
                        core.forget(&peer);
                        break;
                    }
                    Message::Renamed { name } => core.peer_renamed(&peer, &name),
                    // Pongs only keep `heard` fresh; anything else is a
                    // later version's news this one ignores.
                    _ => {}
                }
            }
            _ = ping.tick() => {
                if heard.elapsed() > SILENCE_LIMIT
                    || writer.send(&Message::Ping).await.is_err()
                {
                    break;
                }
            }
        }
    }
    reading.abort();
    writer.close().await;
    core.session_ended(&peer, conn);
}
