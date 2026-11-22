# Architecture

This page maps the code. The rules for changing it are in
[AGENTS.md](../AGENTS.md) and apply to every task; what the keyboard and
mouse features do is in [keyboard-and-mouse.md](keyboard-and-mouse.md), and
file sharing in [file-sharing.md](file-sharing.md).

## Project structure

```text
isthmus/
├── AGENTS.md                  # the rules; CLAUDE.md is a symlink to it
├── README.md, README.zh-CN.md
├── CHANGELOG.md
├── Makefile                   # build the FFI library, the app and the CLI
├── crates/
│   ├── isthmus-core/          # data directory, atomic state files, instance lock, boot session
│   ├── isthmus-input/         # keyboard and mouse habits
│   │   └── src/
│   │       ├── keys.rs        # keycodes, flag bits, HID usages
│   │       ├── settings.rs    # the settings file, presets, normalization
│   │       ├── context.rs     # app classes and traits, built-in bundle lists
│   │       ├── device.rs      # HID services → devices, stable ids, layout detection
│   │       ├── hidmap.rs      # UserKeyMapping per layout and profile, side-bit decoding
│   │       ├── pointer.rs     # Windows pointer speed → HID gain
│   │       ├── journal.rs     # the journal of system changes and recovery decisions
│   │       ├── engine/        # the event tap's decisions: keyboard, rules, mouse
│   │       ├── status.rs      # what the UI shows
│   │       ├── error.rs       # what the service reports failing
│   │       ├── service.rs     # InputService: settings in, system changes out
│   │       └── platform/macos # event tap, HID event system, front app, permissions, login item
│   ├── isthmus-link/          # other computers: discovery, pairing, file sharing
│   │   ├── src/
│   │   │   ├── identity.rs    # the Noise static key and the device id derived from it
│   │   │   ├── store.rs       # link.json and peers.json
│   │   │   ├── path.rs        # interfaces, direct cable vs LAN, candidate ranking
│   │   │   ├── wire.rs        # preamble, Noise XX handshake, frames, messages
│   │   │   ├── pairing.rs     # six-digit codes, SPAKE2, key confirmation
│   │   │   ├── files.rs       # collecting a send, safe names on receive
│   │   │   ├── discovery.rs   # mDNS announce and browse
│   │   │   ├── status.rs      # what the Devices page shows
│   │   │   ├── service/       # LinkService: connect race, sessions, pair, send, receive
│   │   │   └── platform/      # computer name, Downloads, reveal, socket pinning (macOS, Windows)
│   │   └── examples/peer.rs   # a second device in a terminal
│   ├── isthmus-awake/         # keeping the computer awake while the screen sleeps (awake.json, one power request)
│   ├── isthmus-clipboard/     # the system clipboard: a change-counter watcher, reading and writing text
│   └── isthmus-cli/           # the `isthmus` command: devices, status, settings, preset, run, restore, inspect
├── ui/                        # the Flutter app
│   ├── native/isthmus_ffi/    # the C ABI the app calls
│   ├── macos/                 # the Runner: menu-bar agent, quit hook, single instance
│   └── windows/               # the Windows runner: Isthmus.exe beside isthmus_ffi.dll; packaging/ for the installer
├── docs/                      # these pages; docs/README.md indexes them
└── scripts/
    ├── build/                 # stage the FFI library for Xcode and for Windows
    └── release/               # the DMG, the update manifest and its key (docs/updates.md)
```

Dependencies point one way:

```text
isthmus-cli ───► isthmus-input ─┐
                                ├─► isthmus-core
isthmus-ffi ─┬─► isthmus-input ─┤
             ├─► isthmus-link ──┤
             ├─► isthmus-awake ─┘
             └─► isthmus-clipboard   (the FFI joins it to the link)
```

Later features (display alignment, keyboard and mouse across devices)
become sibling crates on top of `isthmus-core`, and further `op`s of the
same FFI export ([roadmap.md](roadmap.md)).

## Threads

| Thread | Owns | Talks to others by |
| --- | --- | --- |
| Tap (`isthmus-tap`) | the active `CGEventTap`, its `CFRunLoop`, the keyboard and mouse engines | reads an `Arc` snapshot (settings, app profile, HID profile, mouse profiles by sender, hotkeys) under a `RwLock` held only to clone the `Arc`; posts actions to the worker |
| HID (`isthmus-hid`) | the HID event-system client, device matching and removal, power notifications, every property write and the journal | commands queued with `CFRunLoopPerformBlock` |
| Main (AppKit) | the `NSWorkspace` observers: app activation, sleep and wake, session switches | publishes the front app and wakes the HID thread |
| Worker (`isthmus-actions`) | blocking side effects: opening apps and Finder windows | a bounded channel |
| Link runtime (`isthmus-link`, two tokio workers) | the listeners, the connector, sessions, pairings, transfers | `LinkService`'s state under one mutex, never held across an `.await`; a change sink |
| Discovery (`isthmus-discovery`, plus mdns-sd's own) | the mDNS browse and its 30-second refresh | an unbounded channel into the link runtime |
| Clipboard (`isthmus-clipboard`) | four looks a second at the clipboard's change counter, and reading what changed | `LinkService::share_clipboard`, which queues the text on each session; text from a session comes back through the watcher's `put` |

The tap callback does no Objective-C, AX, file or lock-holding work; it
reads the snapshot, runs the engine and posts what the engine asks for.

## How the UI talks to Rust

1. `ui/native/isthmus_ffi` exports six C symbols: `isthmus_create`,
   `isthmus_destroy`, `isthmus_call_json`, `isthmus_free_string` and
   `isthmus_set_change_port` for Dart, and `isthmus_shutdown` for the
   Swift quit hook.
2. `isthmus_call_json(handle, request)` takes `{"op": "...", ...args}` and
   returns an envelope: `{"success": true, "data": ...}`,
   `{"success": true, "message": "..."}` or
   `{"success": false, "message": "...", "code": "SCREAMING_SNAKE"}`. Every
   returned string is freed with `isthmus_free_string`.
3. Dart calls it on a worker isolate (`compute`), so the UI never waits on
   Rust. Change hints are
   `{"kind": "status" | "devices" | "settings" | "frontApp" | "link", "seq": n}`
   strings that Rust posts to a Dart `ReceivePort`: Dart hands
   `isthmus_set_change_port` the VM's `NativeApi.postCObject` and the
   port's `nativePort`, and the app re-reads through the normal ops. Rust
   never calls into an isolate: a hot restart kills the old isolate
   without `dispose`, and a post to its port is refused, where a call to
   its callback would abort the app.
4. The service is one per process (a global inside the FFI library). A
   handle is a view of it; Dart's hot restart makes a new handle and finds
   the same service.

**When adding a call**, change all four in step: the Rust op, the Dart
client interface, its implementation and the test fake.

### Ops

| op | args | data |
| --- | --- | --- |
| `start` | – | `Status` (starts the service once; later calls return the status) |
| `status` | – | `Status` |
| `settings_get` | – | `Settings` |
| `settings_set` | `settings` | `Settings` as saved |
| `preset_apply` | `preset`: `windows`/`mac`, `target`: `all`/`keyboard`/`mouse` | `Settings` |
| `devices` | – | `[DeviceEntry]` |
| `running_apps` | – | `[FrontApp]` |
| `permission_request` | `kind`: `accessibility` | `Status` |
| `open_settings_pane` | `pane`: `accessibility`/`loginItems`/`keyboard`/`mouse`/`trackpad` | message |
| `restore` | – | `Settings` (switched off, everything put back) |
| `layout_probe_begin` | `deviceId` | message (that keyboard unmapped for at most 20 s, to read its Mac/Windows mode) |
| `layout_probe_end` | – | message (mapped again) |
| `login_item_get` | – | `{"state": LoginItem}` |
| `login_item_set` | `enabled` | `{"state": LoginItem}` |
| `shutdown` | – | message |
| `link_status` | – | `LinkStatus`, with `clipboardAccess` (`supported`, `access`: `allowed`/`ask`/`denied`, `blocked`) |
| `link_pair` | `peerId` | message (both devices show a code; the other accepts) |
| `link_pair_code` | `code` | message (pairing with a version 1 device) |
| `link_pair_accept` | – | message (accepts the pairing another device asked for) |
| `link_pair_cancel` | – | message (cancels, or closes a finished pairing) |
| `link_unpair` | `peerId` | message |
| `link_send` | `peerId`, `paths` | `{"id": transfer id}` |
| `link_transfer_cancel` | `id` | message |
| `link_transfers_clear` | – | message (finished transfers dropped) |
| `link_rename` | `name` | `LinkStatus`, as `link_status` |
| `link_reveal` | `id`? | message (the transfer's files, or the receive folder, in Finder or Explorer) |
| `link_clipboard_set` | `enabled` | `LinkStatus` (sharing the clipboard with paired devices on or off) |
| `clipboard_request_access` | – | message (raises macOS's "Paste from Other Apps" prompt once) |
| `awake_status` | – | `AwakeStatus` (`enabled`, `active`, `supported`, `error`?) |
| `awake_set` | `enabled` | `AwakeStatus` |
| `update_verify` | `message`, `signature`, `publicKey` (base64) | `{"valid": bool}` (Ed25519, strict; needs no running service) |

The JSON shapes are the serde forms of the Rust types in
[`settings.rs`](../crates/isthmus-input/src/settings.rs),
[`status.rs`](../crates/isthmus-input/src/status.rs),
[`context.rs`](../crates/isthmus-input/src/context.rs) and
[`device.rs`](../crates/isthmus-input/src/device.rs) and
[`isthmus-link/src/status.rs`](../crates/isthmus-link/src/status.rs), all
camelCase. File sharing and keep-awake start with the service; if one
cannot (another copy holds `link.lock`, an unreadable `awake.json`), the
keyboard and mouse run on and its ops answer with that failure.

Error codes: `INVALID_ARGUMENT`, `NOT_RUNNING`, `ANOTHER_INSTANCE`,
`PERMISSION_MISSING`, `UNSUPPORTED`, `IO`, `BUSY` (another pairing is in
progress), `UNAVAILABLE` (the network or the other device cannot be
reached), `INTERNAL`. Dart treats an unknown code as `INTERNAL`.

## The app

`ui/lib/src`: `ffi/` (raw symbols and loader, the isolate service, the typed
API and its models), `state/` (`AppStore`, one `ChangeNotifier` every page
reads), `shell/` (navigation rail, page frame, window chrome), `pages/`
(overview, keyboard, mouse, devices, settings), `theme/` (colours, type,
metrics, motion, `AppIcons`), `widgets/`, `common/` (l10n, tray, the
window-wide file drop target), `updates/` (the update check, its dialog
and each platform's installer: [updates.md](updates.md)).
`state/link_store.dart` holds file sharing's snapshot and the files waiting
to be sent. The Runner (`ui/macos/Runner`)
makes the app a menu-bar agent (`LSUIElement`), keeps a single instance,
routes SIGTERM to AppKit, and calls `isthmus_shutdown` from
`applicationWillTerminate`; its Info.plist declares the Local Network use
and the `_isthmus._tcp` service. On Windows (`ui/windows`) the app shows
the Devices and Settings pages only, keeps the native title bar, and lives
in the notification area; `ui/windows/packaging` and
`ui/distribute_options.yaml` make its installer with fastforge and Inno
Setup.

## Development workflow

```bash
make build-isthmus-ffi-macos        # build and stage libisthmus_ffi.dylib
cd ui && flutter run -d macos       # permissions then belong to the terminal
make build-ui-macos-local           # Isthmus-local.app, its own TCC identity
make build-isthmus-cli              # target/release/isthmus
cargo test --workspace --all-features
make check-windows                  # lint every crate for Windows, tests included
cargo run -p isthmus-link --example peer -- "Test peer"   # a second device to pair with
```
