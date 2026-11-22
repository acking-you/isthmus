# File sharing

Isthmus finds the other computers running it, pairs with each one once
with a click on the other computer, connects to it by itself from then on,
sends it whatever you drop on the window, and shares the clipboard with it. A Mac and a PC work alike: the PC runs the
Windows build of the same app. The code is in `crates/isthmus-link`; the
rules every change keeps are in [AGENTS.md](../AGENTS.md).

## Using it

1. Open Isthmus on both computers. Each one lists the other under
   **Devices › Nearby** within a few seconds.
2. Click **Pair** beside the other computer. Its window comes up asking
   whether to pair, and both computers show the same six digits; click
   **Accept** there. With a KVM switch: click Pair, switch over, check the
   digits match, accept. (A computer still on Isthmus 0.1 shows a code to
   type instead.)
3. From then on the two connect by themselves whenever both are running;
   a paired device shows **Connected** and the path it uses.
4. Drag files or folders anywhere onto the window. They wait under
   **Files to send**; click **Send** beside the device that should get them.
5. Files arrive in the receiver's Downloads folder, under their own names
   (`name (1).ext` when the name is taken). **Show in Finder** (**Show in
   Explorer** on a PC) opens them; the folder button at the top opens the
   folder.

Copy text on one computer and paste it on the other: the clipboard is
shared with every paired device that is connected, both ways, and each copy
that crosses says so (a toast with the window in front, a system
notification while Isthmus waits in the menu bar or the notification area).
**Share the clipboard** on the Devices page turns it off. Text only, up to
4 MB; what a password manager copies (marked concealed or transient) stays
on its computer. A Mac running macOS 15.4 or later asks before an app reads
the clipboard by itself: click **Allow** on the Devices page, allow
pasting, then set Isthmus to **Always Allow** in System Settings › Privacy
& Security › Paste from Other Apps. Until then copies from that Mac stay
there, and copies from other devices still arrive.

Unpair from a device's menu. Both sides forget the pairing; a device that
was off when you unpaired it is refused when it comes back and forgets its
side then.

## Paths: the cable first, then the network

A KVM switch moves one keyboard, mouse and monitor between computers; it
carries no data from one computer to the other. What can carry data is a
**cable straight between the two computers**: a USB Ethernet adapter on the
KVM's USB hub (or on the Mac) cabled to the PC's Ethernet port, or a
Thunderbolt cable between two Macs. Nothing on such a cable hands out
addresses, so both ends give themselves one in 169.254/16 (macOS and Windows
do this by themselves within a minute); Isthmus calls any interface whose
IPv4 addresses are all self-assigned a **direct cable**, and everything else
the **local network**.

| Path | What it is | Rank |
| --- | --- | --- |
| Direct cable | an interface with only 169.254 addresses: a bare cable between the two | first |
| Local network | the Wi-Fi or wired network both are on | second |

- **Connecting.** Every connection tries the device's addresses best first,
  a quarter second apart, and takes the first that answers. A working cable
  answers in about a millisecond, so it wins whenever it works; when it is
  unplugged, the network takes over after the quarter second.
- **Moving to the cable.** A session on the network checks for the cable
  every 15 seconds and moves to it as soon as it answers.
- **Losing a path mid-transfer.** A path that dies says nothing: Wi-Fi
  drops or the cable is pulled and TCP retransmits for minutes before the
  system calls the connection broken. So the sender does not wait for that:
  a write that moves nothing for 10 seconds ends the attempt, and the next
  one tries the other paths first. Retries start a quarter second later,
  back off to at most 3 seconds, and go on for 2 minutes after bytes last
  moved; each carries on from the bytes the receiver already has, so pull
  the cable mid-file and the rest comes over Wi-Fi within seconds. The
  receiver waits for the sender to come back meanwhile.
- **Saying why.** A retrying transfer shows which retry it is, what went
  wrong and where ("Local network en1 went quiet", "the connection on
  Direct cable en8 broke"), with the system's own words beneath; a failed
  one keeps its last reason.

The Devices page shows each path of this computer (cable or network,
interface, address), the path each connected device uses, and the path each
transfer took.

## Security

- **One handshake.** Every connection is TCP with a Noise XX handshake
  (X25519, ChaCha20-Poly1305, BLAKE2s). Each device's long-term key is its
  identity; its id is the first 8 bytes of the key's SHA-256.
- **Discovery finds, pairing trusts.** mDNS (`_isthmus._tcp`) says where an
  id answers and what the device calls itself; that grants nothing. Only a
  key pinned by pairing makes a device a peer, and every later connection
  must prove that key, both ways.
- **Pairing.** Between two current versions, the device being added
  commits to a random nonce (sends its SHA-256), the other sends its own
  nonce, the first reveals its nonce, and both show six digits derived from
  the Noise handshake hash (which covers both keys) and the two nonces. A
  machine in the middle runs two handshakes and must fix its nonce on each
  side before it learns the honest side's, so the digits match by chance
  alone, one in a million: comparing them before accepting is what catches
  it, as with Bluetooth's numeric comparison. Accepting without looking
  trusts the network once. With a version 1 device, pairing runs SPAKE2 on
  a six-digit code typed by the user, which a man in the middle gets one
  guess at per attempt. A declined or failed pairing pins nothing on either
  side.
- **The clipboard** travels only to paired devices, over the same
  encrypted sessions, and only while sharing is on at both ends.
- **What arrives.** Only paired devices can send; files from them are taken
  without asking. Every path in an offer is rebuilt name by name under a
  staging folder: `..`, absolute paths and empty names are refused, and
  characters Windows cannot store are replaced, so nothing lands outside
  the receive folder. Files move into place only when all of them are
  complete.
- **Nothing leaves the local network.** There is no server and no relay.

## Permissions and firewalls

- **macOS** asks once for **Local Network** access (finding and reaching
  the other computer) and once for the **Downloads folder** (where files
  land), the latter at the first start so the question comes while you are
  at the Mac. Allow both. If the macOS firewall is on, allow Isthmus to
  accept incoming connections.
- **Windows** Defender Firewall asks on the first start. Allow both
  **private and public** networks: a bare cable is an "Unidentified
  network", which Windows files as public. Without it the PC can still
  connect to the Mac, but the Mac may not find the PC.

Isthmus listens on TCP port 47821 (any free port when that one is taken)
and uses mDNS on UDP port 5353.

## Files

All in the data directory (`~/Library/Application Support/Isthmus` on a Mac,
`%APPDATA%\Isthmus` on Windows), following the settings rules in
[AGENTS.md](../AGENTS.md): read by every later version, unknown keys kept,
written atomically.

| File | Holds |
| --- | --- |
| `link.json` | the device name (`name`, the computer's name when unset), the receive folder (`receiveDir`, Downloads when unset), and whether the clipboard is shared (`clipboard`, on when unset) |
| `peers.json` | each paired device: its id, pinned key, name, system and the addresses it was last reached at |
| `link-identity.json` | this device's key pair, readable by the user alone; deleting it unpairs every device |
| `link.lock` | one process per computer announces this device; a second copy of the app says another Isthmus is sharing |

A transfer cut off for good leaves a hidden `.isthmus-<id>` folder in the
receive folder; one untouched for three days is removed at the next start.

## Limits

- The two computers must share a network or a cable. A network that blocks
  multicast (some guest Wi-Fi) finds nothing, though a paired device still
  reconnects from the address it was last reached at.
- Paired devices send without asking; there is no accept prompt per
  transfer.
- The clipboard shares text, not images or files (send files with Send).
- Each frame is authenticated in transit; there is no hash of a whole file.
  A file whose length changes while it is sent fails the transfer.
- The Windows app has the Devices and Settings pages: the keyboard and
  mouse habits are a Mac's.

## Trying it on one computer

A second device can run in a terminal:

```bash
cargo run -p isthmus-link --example peer -- "Test peer"
```

It announces itself like the app, prints the digits both devices show when
asked to pair, and takes `list`, `pair <n>`, `accept`, `code <digits>` (a
version 1 device), `send <n> <path>…` on stdin; clipboard text from a
paired device is printed. Run
it from Terminal or iTerm, which have their own Local Network permission.

## The protocol

For contributors; `crates/isthmus-link/src/wire.rs` is the reference.

- A connection opens with the 8-byte preamble `ISTHMUS\x01`, which is also
  the Noise prologue, then the Noise XX handshake in length-prefixed frames.
- After it, every frame is a big-endian `u16` length and a Noise transport
  message whose plaintext is a kind byte (1 JSON message, 2 file data) and
  its payload. A value larger than a frame (an offer's file list) is sent as
  a `blob` message and data frames.
- The dialler's first message is `hello` with a purpose: `session` (the
  long-lived connection between paired devices: ping, rename, unpair, the
  clipboard), `pair`, or `transfer`, and its protocol version. The answerer
  replies `welcome`, with its own, or `refused`.
- A pairing between two version 2 devices: `pairCommit` (the answerer's
  SHA-256 of its nonce), `pairNonce` (the dialler's), `pairReveal` (the
  answerer's nonce); both show the digits; the answerer's acceptance is
  `pairResult` true, acknowledged by the dialler's `pairResult` true. With a
  version 1 dialler: `pairCodeShown`, `pairSpake` each way, `pairConfirm`
  each way, `pairResult`.
- The clipboard, on a session with a version 2 device: `clipboard` and the
  text as a blob (`{"text": …}`); a version 1 device is never sent one.
- A transfer: `offer` (id, counts) and the entries as a blob; `accept` and
  the bytes held per entry as a blob; then per file `fileStart` and its data
  from that offset; `done`; `finished`. Either side may `cancel`.
- Messages and enum values from a later version read as `unknown` or the
  value that changes nothing, so two versions keep talking.
