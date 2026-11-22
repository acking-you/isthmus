# File sharing

Isthmus finds the other computers running it, pairs with each one once
using a six-digit code, connects to it by itself from then on, and sends it
whatever you drop on the window. A Mac and a PC work alike: the PC runs the
Windows build of the same app. The code is in `crates/isthmus-link`; the
rules every change keeps are in [AGENTS.md](../AGENTS.md).

## Using it

1. Open Isthmus on both computers. Each one lists the other under
   **Devices › Nearby** within a few seconds.
2. Click **Pair** beside the other computer. Its window comes up showing a
   six-digit code; type that code on the computer you clicked on. With a
   KVM switch: click Pair, switch over, read the code, switch back, type it.
3. From then on the two connect by themselves whenever both are running;
   a paired device shows **Connected** and the path it uses.
4. Drag files or folders anywhere onto the window. They wait under
   **Files to send**; click **Send** beside the device that should get them.
5. Files arrive in the receiver's Downloads folder, under their own names
   (`name (1).ext` when the name is taken). **Show in Finder** (**Show in
   Explorer** on a PC) opens them; the folder button at the top opens the
   folder.

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
- **Losing a path mid-transfer.** A transfer whose connection drops is
  tried again over the best path left, up to six times, and carries on
  from the bytes the receiver already has: pull the cable mid-file and the
  rest comes over Wi-Fi.

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
- **The code.** Pairing runs SPAKE2 on the six digits shown on the device
  being added, then confirms the key with an HMAC over the Noise handshake
  hash, which covers both keys. Someone listening learns nothing to test
  codes against; someone in the middle gets one guess per attempt, and each
  attempt shows a new code and pops the window up on the device asked. A
  wrong code pins nothing on either side.
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
| `link.json` | the device name (`name`, the computer's name when unset) and the receive folder (`receiveDir`, Downloads when unset) |
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
- Each frame is authenticated in transit; there is no hash of a whole file.
  A file whose length changes while it is sent fails the transfer.
- The Windows app has the Devices and Settings pages: the keyboard and
  mouse habits are a Mac's.

## Trying it on one computer

A second device can run in a terminal:

```bash
cargo run -p isthmus-link --example peer -- "Test peer"
```

It announces itself like the app, prints the code when asked to pair, and
takes `list`, `pair <n>`, `code <digits>`, `send <n> <path>…` on stdin. Run
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
  long-lived connection between paired devices: ping, rename, unpair),
  `pair`, or `transfer`. The answerer replies `welcome` or `refused`.
- A transfer: `offer` (id, counts) and the entries as a blob; `accept` and
  the bytes held per entry as a blob; then per file `fileStart` and its data
  from that offset; `done`; `finished`. Either side may `cancel`.
- Messages and enum values from a later version read as `unknown` or the
  value that changes nothing, so two versions keep talking.
