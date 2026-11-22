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
use crate::wire::{ClipboardText, MAX_CLIPBOARD_BLOB, Message, SecureReader, SecureWriter};

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
    /// The protocol version the peer said it speaks.
    pub version: u32,
}

/// What the reader task hands the loop.
enum Incoming {
    Message(Message),
    /// The other device's clipboard text.
    Clipboard(String),
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
            let incoming = match message {
                Message::Clipboard => {
                    match reader.blob::<ClipboardText>(MAX_CLIPBOARD_BLOB).await {
                        Ok(clipboard) => Incoming::Clipboard(clipboard.text),
                        Err(_) => break,
                    }
                }
                message => Incoming::Message(message),
            };
            if message_tx.send(incoming).await.is_err() {
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
                Some(SessionCommand::Clipboard(text)) => {
                    let clipboard = ClipboardText { text: text.to_string() };
                    if writer.send(&Message::Clipboard).await.is_err()
                        || writer.send_blob(&clipboard).await.is_err()
                    {
                        break;
                    }
                }
            },
            incoming = messages.recv() => {
                let Some(incoming) = incoming else { break };
                heard = Instant::now();
                let message = match incoming {
                    Incoming::Clipboard(text) => {
                        core.clipboard_received(&peer, text);
                        continue;
                    }
                    Incoming::Message(message) => message,
                };
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
