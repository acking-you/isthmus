# Roadmap

**Nothing on this page is built.** Isthmus today is the keyboard and mouse
([keyboard-and-mouse.md](keyboard-and-mouse.md)). This page is the plan for
what comes after it, in the order it is meant to come: what each step does,
the decisions already made, and the questions still open. A step gets its
own page when work on it starts, and reaches the
[changelog](../CHANGELOG.md) only when it ships. [AGENTS.md](../AGENTS.md)
has the rules every step keeps; [architecture.md](architecture.md) maps the
code they extend.

| Step | What it gives | Builds on | Status |
| --- | --- | --- | --- |
| [1. Finding devices](#1-finding-devices-on-the-local-network) | Other Isthmus computers on the local network, listed | – | planned |
| [2. Pairing](#2-pairing-with-a-short-code) | A trusted link between two of them, made once with a short code | 1 | planned |
| [3. File sharing](#3-file-sharing-between-paired-devices) | Files sent from one paired device to another | 2 | planned |
| [4. Display alignment](#4-display-alignment) | A display that looks and moves the same from both systems | – | planned |
| [5. Keyboard and mouse across devices](#5-keyboard-and-mouse-across-devices) | One keyboard and mouse for two computers side by side | 2, 4 | planned |

## How new features fit

Each step is a sibling crate of `isthmus-input` on `isthmus-core`, with its
own `platform/<os>/` and its own ops of the one FFI export,
`isthmus_call_json`. Crate names below are placeholders.

```text
isthmus-cli ─┐   ┌─► isthmus-input    built: keyboard and mouse ───┐
             ├───┼─► isthmus-link     planned: discovery, pairing  ├─► isthmus-core
isthmus-ffi ─┘   ├─► isthmus-share    planned: files, on link      │
                 └─► isthmus-display  planned: display alignment ──┘
```

What carries over from the keyboard and mouse:

- **Dependencies still point one way.** A later crate may use an earlier
  one (file sharing uses pairing), never the reverse, and nothing below
  `isthmus-input` learns about the network.
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

**Plan.** Each running Isthmus announces itself with Bonjour (DNS-SD) as
`_isthmus._tcp` on the local network and browses for the others. The app
lists what it finds: a name the user chose for the computer, its system and
whether it is paired. The TXT record carries a protocol version and a
random device id, never the user's account name.

**Decided.** Discovery only finds; it grants nothing. Every connection after
it is made through pairing (step 2), so a device on the same network that
answers to `_isthmus._tcp` learns no more than that Isthmus runs here.

**Open.**

- Since macOS 15, macOS asks for Local Network access before an app
  browses or advertises; the Info.plist needs the usage text and the
  service type, and the app should explain the prompt before it appears.
- Networks that block multicast (guest Wi-Fi, some offices) find nothing. A
  device can still be added by address, which needs its own design.
- A Windows peer needs a Windows program; the protocol is not Mac-only, but
  the first peer is another Mac.

## 2. Pairing with a short code

**Plan.** Two devices pair once. One shows a short code, the user types it
on the other, and the code runs a password-authenticated key exchange
(PAKE). The exchange swaps each device's long-term certificate, and every
later connection is TLS 1.3 with both certificates pinned, without a code.
Unpairing deletes the pin on both sides; a device that was unpaired while
away is refused when it returns.

**Why a PAKE.** A short code is too weak to protect a key on its own: an
eavesdropper could try every code offline. With a PAKE, someone who watches
the exchange learns nothing to test codes against, and someone who joins it
gets one guess per attempt. A wrong code ends the attempt.

**Decided.** The code is shown, never sent. A pairing proves only that the
two devices are the ones the user looked at; it says nothing about the
network, which is why every later connection is encrypted and pinned. Each
device's private key stays on that device.

**Open.** Which PAKE (SPAKE2 and CPace are the candidates) and which crate,
chosen for review history over features; where the private key lives (the
login Keychain is the first choice); how long a code stays valid.

## 3. File sharing between paired devices

**Plan.** Send a file or folder to a paired device from the app or by
dropping it on the device's entry. It arrives in the receiver's Downloads
folder, after the receiver accepts it or straight away for a device the user
trusts that way. Transfers go over the pinned connection from step 2, are
checked by hash at the end, and resume after a dropped connection.

**Decided.** Only paired devices take part, and nothing passes through a
server outside the local network. It is sending, not syncing: no folder is
kept in step.

**Open.** Whether the clipboard is a later part of this step (copy on one
computer, paste on the other), and how a transfer shows its progress in the
menu bar.

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
