# Roadmap

Isthmus today is the keyboard and mouse
([keyboard-and-mouse.md](keyboard-and-mouse.md)) and file sharing between
computers ([file-sharing.md](file-sharing.md)). This page is the plan for
the rest, in the order it is meant to come: what each step does, the
decisions already made, and the questions still open. A step gets its own
page when work on it starts, and reaches the [changelog](../CHANGELOG.md)
only when it ships. [AGENTS.md](../AGENTS.md) has the rules every step
keeps; [architecture.md](architecture.md) maps the code they extend.

| Step | What it gives | Builds on | Status |
| --- | --- | --- | --- |
| [1. Finding devices](#1-finding-devices-on-the-local-network) | Other Isthmus computers on the local network, listed | – | built |
| [2. Pairing](#2-pairing-with-a-short-code) | A trusted link between two of them, made once with a short code | 1 | built |
| [3. File sharing](#3-file-sharing-between-paired-devices) | Files sent from one paired device to another | 2 | built |
| [4. Display alignment](#4-display-alignment) | A display that looks and moves the same from both systems | – | planned |
| [5. Keyboard and mouse across devices](#5-keyboard-and-mouse-across-devices) | One keyboard and mouse for two computers side by side | 2, 4 | planned |

## How new features fit

Each step is a sibling crate of `isthmus-input` on `isthmus-core`, with its
own `platform/<os>/` and its own ops of the one FFI export,
`isthmus_call_json`. Discovery, pairing and file sharing turned out to be
one crate, `isthmus-link`: a transfer is a connection of the same protocol.
The display crate's name is a placeholder.

```text
isthmus-cli ─┐   ┌─► isthmus-input    built: keyboard and mouse ─────────────┐
             ├───┼─► isthmus-link     built: discovery, pairing, files ──────├─► isthmus-core
isthmus-ffi ─┘   └─► isthmus-display  planned: display alignment ────────────┘
```

What carries over from the keyboard and mouse:

- **Dependencies still point one way.** A later crate may use an earlier
  one (keyboard and mouse across devices will use `isthmus-link`), never
  the reverse, and nothing below them learns about the network.
- **Every system change is journalled.** A display setting or a default
  Isthmus writes is recorded before it is written and put back on quit,
  pause and after a crash, never over a value someone else set, as HID
  properties are today.
- **State files follow the settings rules.** Each step keeps its own file
  under `~/Library/Application Support/Isthmus/`, written atomically, with
  fields added with defaults and unknown keys kept.
- **The app adds ops, not exports.** Each call changes the Rust op, the Dart
  client interface, its implementation and the test fake together.

## 1. Finding devices on the local network

**Built** in `isthmus-link` ([file-sharing.md](file-sharing.md)). Each
running Isthmus announces itself with mDNS as `_isthmus._tcp` on every
interface, a bare cable included, and browses for the others; the TXT
record carries the device id, the name the user chose, the system and the
protocol version, never the account name. Discovery only finds: whoever
answers must still prove the id's key, and only a pinned key is trusted.

**Decided along the way.** The browse restarts and the device announces
again every 30 seconds, since mdns-sd backs its queries off towards an hour
and a query lost before macOS granted Local Network access would otherwise
hide a device for long. A paired device is also dialled at the address it
was last reached at, so it reconnects on networks that block multicast. The
first Windows peer came with this step rather than later: the Windows build
of the app has the Devices and Settings pages.

**Open.** Adding a device by address by hand, for a network that blocks
multicast before the first pairing.

## 2. Pairing with a short code

**Built.** The device being added shows a six-digit code and the user types
it on the device that asked. The code runs SPAKE2 (the RustCrypto `spake2`
crate), confirmed with an HMAC over the Noise handshake hash, so an
eavesdropper learns nothing to test codes against and a man in the middle
gets one guess per attempt. Unpairing deletes the pin on both sides; a
device unpaired while away is refused when it returns and forgets its side.

**Changed from the plan.** Connections use the Noise protocol (XX, X25519,
ChaCha20-Poly1305, BLAKE2s) instead of TLS 1.3 with pinned certificates:
the static key is the identity with no certificate around it, and every
crate is pure Rust, so the Windows build type-checks from a Mac. The private
key lives in `link-identity.json` in the data directory, readable by the
user alone, rather than in the Keychain, the same on both systems. A code
lasts two minutes and serves one attempt.

## 3. File sharing between paired devices

**Built.** Files and folders dropped on the window are sent to a paired
device with one click and land in its Downloads folder. Paired devices are
trusted to send without a prompt. Each transfer runs over the best path at
the moment (a direct cable before the shared network) and resumes from the
bytes the receiver holds when its connection drops, over the next path.

**Changed from the plan.** Integrity in transit comes from the
authenticated encryption of every frame; there is no hash of the whole file
at the end. A file whose length changes while it is sent fails the
transfer instead.

**Open.** The clipboard (copy on one computer, paste on the other); a
progress line in the menu bar; choosing the receive folder in the app
(today it is `receiveDir` in `link.json`).

## 4. Display alignment

**Plan.** A monitor shared through a KVM switch should look and feel the
same from both systems. Candidates, each to be checked before it is
offered:

- **Scaling.** macOS offers no HiDPI modes on many external non-Retina
  monitors, so text is either small or blurry where Windows would scale it
  to 125 % or 150 %.
- **Text rendering.** macOS has drawn text without subpixel antialiasing
  since 10.14, so on a low-density monitor it looks lighter and softer than
  Windows' ClearType.
- **Cursor size.** The pointer's size to match what Windows shows at the
  same scale.
- **Pointer distance.** Windows pointer speed, which is built, moves the
  pointer by points; under a scaled mode one point covers more than one
  pixel, so the gain should follow the display's scale to cover the same
  distance as Windows does.

**Decided.** Every setting Isthmus changes here is journalled and put back
like a HID property. Nothing that needs an administrator password or writes
outside the user's own preferences is done silently.

**Open.** Which of these can be changed without an administrator password,
which take effect without logging out, and how a per-display setting
follows a monitor across a KVM switch.

## 5. Keyboard and mouse across devices

**Plan.** One keyboard and mouse for a Mac and a PC side by side: the
pointer crosses to the other computer at a screen edge, and keys follow it.
The screens are arranged edge to edge in the app, as displays are in System
Settings. It runs over the pinned connection from step 2.

**Decided.** The Windows translation is applied once, on the computer where
the keys are typed: input that arrives from a peer is posted as Isthmus's
own events, and the local event tap must tell it from hardware. Input from a
peer stops at once when the connection drops, with every held key released.

**Open.** The PC side needs its own program; the Rust core above
`platform/` is portable, so that program is a `platform/windows/`, built and
tested only on Windows. How the two ends agree on screen edges when the
displays have different scales (step 4).

## Not planned

- **The Mac App Store.** The App Sandbox forbids an event tap that changes
  events, which the keyboard and mouse need.
- **Remapping letters and digits.** macOS restricts HID mappings of those
  keys to processes with Input Monitoring, and the shortcut table covers
  what Windows habits need without them.
