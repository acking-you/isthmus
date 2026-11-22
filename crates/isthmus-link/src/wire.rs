//! The bytes on a connection: an 8-byte preamble naming the protocol, a
//! Noise XX handshake, then encrypted frames.
//!
//! A frame is a big-endian `u16` length and a Noise transport message whose
//! plaintext is one kind byte and its payload: a JSON [`Message`] or raw
//! file data. Every frame fits Noise's 65535-byte limit, so a payload that
//! might not (a long list of files) goes as a [`Message::Blob`] header
//! followed by data frames. The transport is Noise's stateless form with a
//! nonce counter on each side, so a connection's reader and writer are
//! separate values that can live in separate tasks.
//!
//! The preamble is also the Noise prologue: a peer speaking another version
//! fails the handshake rather than misreading the frames.

use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use snow::StatelessTransportState;
use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, ReadHalf, WriteHalf,
};

use crate::files::Entry;
use crate::identity::{Identity, PublicKey};
use crate::store::PeerOs;

/// The protocol's name and version, sent first and bound into the
/// handshake.
pub const PREAMBLE: [u8; 8] = *b"ISTHMUS\x01";

/// The Noise protocol: XX (both static keys exchanged, encrypted), X25519,
/// ChaCha20-Poly1305, BLAKE2s.
pub const NOISE_PARAMS: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

/// The protocol version in announcements and hellos. 2 added pairing by
/// confirmation (`PairCommit`, `PairNonce`, `PairReveal`) and the
/// clipboard; a peer's version decides which pairing it is offered and
/// whether it is sent the clipboard, so both keep talking to version 1.
pub const PROTOCOL_VERSION: u32 = 2;

/// The first version that pairs by confirmation and shares the clipboard.
pub const CONFIRM_VERSION: u32 = 2;

/// Noise's largest message.
const MAX_FRAME: usize = 65535;
/// The AEAD tag on every transport message.
const TAG_LEN: usize = 16;
/// The largest payload of one frame: the kind byte and the tag come off.
pub const MAX_PAYLOAD: usize = MAX_FRAME - TAG_LEN - 1;

/// How long a handshake may take before the connection is dropped.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// The largest blob a peer may announce: an offer of the most files with
/// long paths fits well inside it.
pub const MAX_BLOB: u64 = 64 * 1024 * 1024;

/// The largest clipboard blob: four megabytes of text, escaped.
pub const MAX_CLIPBOARD_BLOB: u64 = 16 * 1024 * 1024;

/// What a [`Message::Clipboard`]'s blob holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardText {
    pub text: String,
}

/// How big the buffers around the socket are: a few frames, so data frames
/// go out in large writes.
const IO_BUFFER: usize = 256 * 1024;

/// What the kind byte says the payload is.
const KIND_MESSAGE: u8 = 1;
const KIND_DATA: u8 = 2;

/// A connection failing.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum WireError {
    /// The socket failed.
    #[snafu(display("the connection failed"))]
    Io { source: std::io::Error },
    /// The handshake or a frame did not authenticate.
    #[snafu(display("the connection is not secure"))]
    Noise { source: snow::Error },
    /// The other end does not speak this protocol.
    #[snafu(display("the other end is not Isthmus or another version"))]
    Preamble,
    /// The other end closed the connection.
    #[snafu(display("the other end closed the connection"))]
    Closed,
    /// A frame broke the framing rules.
    #[snafu(display("the other end sent a malformed frame"))]
    Malformed,
    /// A message was not valid JSON for this version.
    #[snafu(display("the other end sent a message this version cannot read"))]
    Decode { source: serde_json::Error },
    /// The other end sent something other than what this step expects.
    #[snafu(display("the other end sent {got} where it should not"))]
    Unexpected { got: &'static str },
    /// A step took too long.
    #[snafu(display("the other end stopped answering"))]
    Timeout,
}

impl WireError {
    /// Whether the connection was lost rather than refused: worth retrying
    /// on another path.
    pub fn is_lost(&self) -> bool {
        match self {
            WireError::Io { .. } | WireError::Closed | WireError::Timeout => true,
            WireError::Noise { .. }
            | WireError::Preamble
            | WireError::Malformed
            | WireError::Decode { .. }
            | WireError::Unexpected { .. } => false,
        }
    }
}

/// Why a connection opens: its first encrypted message says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Purpose {
    /// The long-lived connection between two paired devices: presence,
    /// renames, unpairing.
    Session,
    /// A pairing: by confirmation between two version 2 devices, else
    /// with a code.
    Pair,
    /// One transfer of files.
    Transfer,
    /// A purpose a later version added; refused.
    #[serde(other)]
    Unknown,
}

/// Why a connection was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Refusal {
    /// The device does not hold this device's key as paired.
    NotPaired,
    /// It is busy with another pairing, or pairing too often.
    Busy,
    /// The transfer was canceled on the receiving side.
    Canceled,
    /// The receiver cannot store the files.
    Disk,
    /// The offer was malformed (an unsafe path, say).
    Invalid,
    /// A reason a later version added.
    #[serde(other)]
    Other,
}

/// Everything said on a connection, as JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Message {
    /// The dialler's first message.
    Hello {
        purpose: Purpose,
        name: String,
        os: PeerOs,
        version: u32,
        /// The port the dialler listens on, so the answerer can dial back.
        port: u16,
    },
    /// The answerer's reply to a hello it accepts.
    Welcome {
        name: String,
        os: PeerOs,
        version: u32,
        port: u16,
    },
    /// The answerer's reply to a hello it does not accept, or a receiver
    /// refusing an offer.
    Refused {
        reason: Refusal,
    },
    /// Session keep-alive.
    Ping,
    Pong,
    /// The sender of this no longer pairs with the receiver.
    Unpair,
    /// The sender's new name.
    Renamed {
        name: String,
    },
    /// Pairing: the answerer shows its code now.
    PairCodeShown,
    /// Pairing: a SPAKE2 message, hex.
    PairSpake {
        message: String,
    },
    /// Pairing: a key confirmation, hex.
    PairConfirm {
        tag: String,
    },
    /// Pairing: whether the dialler accepted the answerer's confirmation;
    /// by confirmation, whether the answerer's user accepted, and the
    /// dialler's acknowledgement.
    PairResult {
        ok: bool,
    },
    /// Pairing by confirmation: the answerer's commitment to its nonce,
    /// hex, sent before it sees the dialler's.
    PairCommit {
        commit: String,
    },
    /// Pairing by confirmation: the dialler's nonce, hex.
    PairNonce {
        nonce: String,
    },
    /// Pairing by confirmation: the answerer's nonce, hex, which must open
    /// its commitment.
    PairReveal {
        nonce: String,
    },
    /// Transfer: what is coming. The entries follow as a blob.
    Offer {
        id: String,
        files: u32,
        total: u64,
    },
    /// Transfer: the receiver takes it; the offsets (bytes already held per
    /// entry) follow as a blob.
    Accept,
    /// Transfer: the next data frames are entry `index` from `offset`.
    FileStart {
        index: u32,
        offset: u64,
    },
    /// Transfer: every file was sent.
    Done,
    /// Transfer: the receiver has every file in place.
    Finished,
    /// Transfer: the sender of this gives up on the transfer.
    Cancel,
    /// `bytes` of JSON follow in data frames.
    Blob {
        bytes: u64,
    },
    /// Session: the sender's clipboard, a [`ClipboardText`] blob, follows.
    /// Sent only to version 2 devices: a version 1 session would end at the
    /// blob's data frames.
    Clipboard,
    /// A message a later version added; ignored where it can be.
    #[serde(other)]
    Unknown,
}

impl Message {
    /// The message's name, for errors.
    pub fn name(&self) -> &'static str {
        match self {
            Message::Hello { .. } => "hello",
            Message::Welcome { .. } => "welcome",
            Message::Refused { .. } => "refused",
            Message::Ping => "ping",
            Message::Pong => "pong",
            Message::Unpair => "unpair",
            Message::Renamed { .. } => "renamed",
            Message::PairCodeShown => "pairCodeShown",
            Message::PairSpake { .. } => "pairSpake",
            Message::PairConfirm { .. } => "pairConfirm",
            Message::PairResult { .. } => "pairResult",
            Message::PairCommit { .. } => "pairCommit",
            Message::PairNonce { .. } => "pairNonce",
            Message::PairReveal { .. } => "pairReveal",
            Message::Offer { .. } => "offer",
            Message::Accept => "accept",
            Message::FileStart { .. } => "fileStart",
            Message::Done => "done",
            Message::Finished => "finished",
            Message::Cancel => "cancel",
            Message::Blob { .. } => "blob",
            Message::Clipboard => "clipboard",
            Message::Unknown => "an unknown message",
        }
    }
}

/// A frame as received.
#[derive(Debug)]
pub enum Frame<'a> {
    Message(Message),
    Data(&'a [u8]),
}

/// What the handshake learned about the other end.
#[derive(Debug, Clone)]
pub struct Remote {
    /// The key the other end proved it holds.
    pub public_key: PublicKey,
    /// The Noise handshake hash: unique to this connection and both keys,
    /// which pairing binds its confirmation to.
    pub handshake_hash: Vec<u8>,
}

/// The receiving half of a secure connection.
pub struct SecureReader<R> {
    io: BufReader<R>,
    noise: Arc<StatelessTransportState>,
    nonce: u64,
    cipher: Vec<u8>,
    plain: Vec<u8>,
}

/// The sending half of a secure connection.
pub struct SecureWriter<W> {
    io: BufWriter<W>,
    noise: Arc<StatelessTransportState>,
    nonce: u64,
    plain: Vec<u8>,
    cipher: Vec<u8>,
}

/// A connection after its handshake: both halves and what was learned.
pub type Secured<S> = (
    SecureReader<ReadHalf<S>>,
    SecureWriter<WriteHalf<S>>,
    Remote,
);

/// Runs the handshake as the dialler.
pub async fn initiate<S>(stream: S, identity: &Identity) -> Result<Secured<S>, WireError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(stream, identity, true))
        .await
        .map_err(|_| WireError::Timeout)?
}

/// Runs the handshake as the answerer.
pub async fn respond<S>(stream: S, identity: &Identity) -> Result<Secured<S>, WireError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(stream, identity, false))
        .await
        .map_err(|_| WireError::Timeout)?
}

async fn handshake<S>(
    stream: S,
    identity: &Identity,
    initiator: bool,
) -> Result<Secured<S>, WireError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let params = NOISE_PARAMS.parse().context(NoiseSnafu)?;
    let builder = snow::Builder::new(params)
        .local_private_key(identity.private_key())
        .context(NoiseSnafu)?
        .prologue(&PREAMBLE)
        .context(NoiseSnafu)?;
    let mut state = if initiator {
        builder.build_initiator()
    } else {
        builder.build_responder()
    }
    .context(NoiseSnafu)?;

    let (read, write) = tokio::io::split(stream);
    let mut reader = BufReader::with_capacity(IO_BUFFER, read);
    let mut writer = BufWriter::with_capacity(IO_BUFFER, write);
    if initiator {
        writer.write_all(&PREAMBLE).await.context(IoSnafu)?;
    } else {
        let mut preamble = [0; PREAMBLE.len()];
        read_exact(&mut reader, &mut preamble).await?;
        if preamble != PREAMBLE {
            return Err(WireError::Preamble);
        }
    }

    let mut frame = vec![0; MAX_FRAME];
    let mut scratch = vec![0; MAX_FRAME];
    while !state.is_handshake_finished() {
        if state.is_my_turn() {
            let len = state.write_message(&[], &mut frame).context(NoiseSnafu)?;
            write_frame(&mut writer, frame.get(..len).ok_or(WireError::Malformed)?).await?;
            writer.flush().await.context(IoSnafu)?;
        } else {
            let len = read_frame(&mut reader, &mut frame).await?;
            let message = frame.get(..len).ok_or(WireError::Malformed)?;
            state
                .read_message(message, &mut scratch)
                .context(NoiseSnafu)?;
        }
    }
    let public_key = state
        .get_remote_static()
        .and_then(PublicKey::from_slice)
        .ok_or(WireError::Malformed)?;
    let handshake_hash = state.get_handshake_hash().to_vec();
    let noise = Arc::new(state.into_stateless_transport_mode().context(NoiseSnafu)?);
    Ok((
        SecureReader {
            io: reader,
            noise: noise.clone(),
            nonce: 0,
            cipher: vec![0; MAX_FRAME],
            plain: vec![0; MAX_FRAME],
        },
        SecureWriter {
            io: writer,
            noise,
            nonce: 0,
            plain: Vec::with_capacity(MAX_FRAME),
            cipher: vec![0; MAX_FRAME],
        },
        Remote {
            public_key,
            handshake_hash,
        },
    ))
}

/// `read_exact` with a clean end of stream reported as [`WireError::Closed`].
async fn read_exact<R: AsyncRead + Unpin>(reader: &mut R, buf: &mut [u8]) -> Result<(), WireError> {
    match reader.read_exact(buf).await {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Err(WireError::Closed),
        Err(source) => Err(WireError::Io { source }),
    }
}

/// Reads one length-prefixed frame into `buf`, returning its length.
async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    let mut len = [0; 2];
    read_exact(reader, &mut len).await?;
    let len = usize::from(u16::from_be_bytes(len));
    if len == 0 {
        return Err(WireError::Malformed);
    }
    read_exact(reader, buf.get_mut(..len).ok_or(WireError::Malformed)?).await?;
    Ok(len)
}

/// Writes one length-prefixed frame.
async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, frame: &[u8]) -> Result<(), WireError> {
    let len = u16::try_from(frame.len()).map_err(|_| WireError::Malformed)?;
    writer
        .write_all(&len.to_be_bytes())
        .await
        .context(IoSnafu)?;
    writer.write_all(frame).await.context(IoSnafu)
}

impl<R: AsyncRead + Unpin> SecureReader<R> {
    /// The next frame. Not cancel-safe: a caller that may stop waiting
    /// reads in a task of its own (see the session loop).
    pub async fn recv(&mut self) -> Result<Frame<'_>, WireError> {
        let len = read_frame(&mut self.io, &mut self.cipher).await?;
        let cipher = self.cipher.get(..len).ok_or(WireError::Malformed)?;
        let plain_len = self
            .noise
            .read_message(self.nonce, cipher, &mut self.plain)
            .context(NoiseSnafu)?;
        self.nonce += 1;
        let (kind, payload) = self
            .plain
            .get(..plain_len)
            .and_then(<[u8]>::split_first)
            .ok_or(WireError::Malformed)?;
        match *kind {
            KIND_MESSAGE => serde_json::from_slice(payload)
                .map(Frame::Message)
                .context(DecodeSnafu),
            KIND_DATA => Ok(Frame::Data(payload)),
            _ => Err(WireError::Malformed),
        }
    }

    /// The next frame, which must be a message.
    pub async fn message(&mut self) -> Result<Message, WireError> {
        match self.recv().await? {
            Frame::Message(message) => Ok(message),
            Frame::Data(_) => Err(WireError::Unexpected { got: "data" }),
        }
    }

    /// The next message, or [`WireError::Timeout`] after `limit`.
    pub async fn message_within(&mut self, limit: Duration) -> Result<Message, WireError> {
        tokio::time::timeout(limit, self.message())
            .await
            .map_err(|_| WireError::Timeout)?
    }

    /// A value sent with [`SecureWriter::send_blob`], at most `max` bytes.
    pub async fn blob<T: DeserializeOwned>(&mut self, max: u64) -> Result<T, WireError> {
        let bytes = match self.message().await? {
            Message::Blob { bytes } => bytes,
            other => return Err(WireError::Unexpected { got: other.name() }),
        };
        if bytes > max {
            return Err(WireError::Malformed);
        }
        let len = usize::try_from(bytes).map_err(|_| WireError::Malformed)?;
        let mut json = Vec::with_capacity(len);
        while json.len() < len {
            match self.recv().await? {
                Frame::Data(chunk) if json.len() + chunk.len() <= len => {
                    json.extend_from_slice(chunk);
                }
                Frame::Data(_) => return Err(WireError::Malformed),
                Frame::Message(other) => return Err(WireError::Unexpected { got: other.name() }),
            }
        }
        serde_json::from_slice(&json).context(DecodeSnafu)
    }
}

impl<W: AsyncWrite + Unpin> SecureWriter<W> {
    /// Encrypts and buffers one frame of `kind`.
    async fn frame(&mut self, kind: u8, payload: &[u8]) -> Result<(), WireError> {
        if payload.len() > MAX_PAYLOAD {
            return Err(WireError::Malformed);
        }
        self.plain.clear();
        self.plain.push(kind);
        self.plain.extend_from_slice(payload);
        let len = self
            .noise
            .write_message(self.nonce, &self.plain, &mut self.cipher)
            .context(NoiseSnafu)?;
        self.nonce += 1;
        let frame = self.cipher.get(..len).ok_or(WireError::Malformed)?;
        write_frame(&mut self.io, frame).await
    }

    /// Sends a message and flushes, so it leaves now.
    pub async fn send(&mut self, message: &Message) -> Result<(), WireError> {
        let json = serde_json::to_vec(message).context(DecodeSnafu)?;
        self.frame(KIND_MESSAGE, &json).await?;
        self.flush().await
    }

    /// Buffers file data, split into frames; [`Self::flush`] or the next
    /// [`Self::send`] puts it on the wire.
    pub async fn send_data(&mut self, data: &[u8]) -> Result<(), WireError> {
        for chunk in data.chunks(MAX_PAYLOAD) {
            self.frame(KIND_DATA, chunk).await?;
        }
        Ok(())
    }

    /// Sends `value` as JSON in data frames after a [`Message::Blob`].
    pub async fn send_blob<T: Serialize>(&mut self, value: &T) -> Result<(), WireError> {
        let json = serde_json::to_vec(value).context(DecodeSnafu)?;
        let bytes = u64::try_from(json.len()).map_err(|_| WireError::Malformed)?;
        let header = serde_json::to_vec(&Message::Blob { bytes }).context(DecodeSnafu)?;
        self.frame(KIND_MESSAGE, &header).await?;
        self.send_data(&json).await?;
        self.flush().await
    }

    /// Writes out everything buffered.
    pub async fn flush(&mut self) -> Result<(), WireError> {
        self.io.flush().await.context(IoSnafu)
    }

    /// Flushes and closes the sending direction.
    pub async fn close(&mut self) {
        let _ = self.io.shutdown().await;
    }
}

/// The entries of an offer, as its blob carries them.
pub type OfferEntries = Vec<Entry>;

#[cfg(test)]
mod tests {
    use super::*;

    async fn pair_of_ends() -> (
        Secured<tokio::io::DuplexStream>,
        Secured<tokio::io::DuplexStream>,
        Identity,
        Identity,
    ) {
        let a = Identity::generate().unwrap();
        let b = Identity::generate().unwrap();
        let (left, right) = tokio::io::duplex(MAX_FRAME * 4);
        let (dialled, answered) = tokio::join!(initiate(left, &a), respond(right, &b));
        (dialled.unwrap(), answered.unwrap(), a, b)
    }

    /// Each end learns the other's key and the same handshake hash, and
    /// messages, data and blobs larger than a frame arrive whole.
    #[tokio::test]
    async fn the_handshake_proves_keys_and_frames_round_trip() {
        let ((mut ar, mut aw, a_saw), (mut br, mut bw, b_saw), a, b) = pair_of_ends().await;
        assert_eq!(&a_saw.public_key, b.public_key());
        assert_eq!(&b_saw.public_key, a.public_key());
        assert_eq!(a_saw.handshake_hash, b_saw.handshake_hash);

        aw.send(&Message::Ping).await.unwrap();
        assert_eq!(br.message().await.unwrap(), Message::Ping);

        let data = vec![7u8; MAX_PAYLOAD * 2 + 10];
        let entries: Vec<String> = (0..5000).map(|i| format!("folder/file-{i}.txt")).collect();
        let sender = async {
            bw.send_data(&data).await.unwrap();
            bw.send_blob(&entries).await.unwrap();
        };
        let receiver = async {
            let mut got = Vec::new();
            while got.len() < data.len() {
                match ar.recv().await.unwrap() {
                    Frame::Data(chunk) => got.extend_from_slice(chunk),
                    Frame::Message(other) => panic!("unexpected {other:?}"),
                }
            }
            assert_eq!(got, data);
            let back: Vec<String> = ar.blob(MAX_BLOB).await.unwrap();
            assert_eq!(back, entries);
        };
        tokio::join!(sender, receiver);
    }

    /// A peer that does not open with the preamble is refused before any
    /// key material is exchanged.
    #[tokio::test]
    async fn a_wrong_preamble_is_refused() {
        let b = Identity::generate().unwrap();
        let (mut left, right) = tokio::io::duplex(1024);
        left.write_all(b"GET / HTTP/1.1\r\n").await.unwrap();
        assert!(matches!(respond(right, &b).await, Err(WireError::Preamble)));
    }

    /// A message type from a later version reads as `Unknown` rather than
    /// failing the connection; a known one with an extra field still reads.
    #[test]
    fn later_messages_and_fields_are_tolerated() {
        let unknown: Message =
            serde_json::from_str(r#"{"type": "displayLayout", "screens": 2}"#).unwrap();
        assert_eq!(unknown, Message::Unknown);
        let ping: Message = serde_json::from_str(r#"{"type": "ping", "extra": 1}"#).unwrap();
        assert_eq!(ping, Message::Ping);
        let hello: Message = serde_json::from_str(
            r#"{"type": "hello", "purpose": "teleport", "name": "n", "os": "macos", "version": 2, "port": 1}"#,
        )
        .unwrap();
        assert!(matches!(
            hello,
            Message::Hello {
                purpose: Purpose::Unknown,
                ..
            }
        ));
    }
}
