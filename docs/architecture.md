# Architecture

This page maps the code. The rules for changing it are in
[AGENTS.md](../AGENTS.md) and apply to every task; what the keyboard and
mouse features do is in [keyboard-and-mouse.md](keyboard-and-mouse.md).

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
│   └── isthmus-cli/           # the `isthmus` command: devices, status, settings, preset, run, restore, inspect
├── ui/                        # the Flutter app
│   ├── native/isthmus_ffi/    # the C ABI the app calls
│   └── macos/                 # the Runner: menu-bar agent, quit hook, single instance
├── docs/                      # these pages; docs/README.md indexes them
└── scripts/build/             # stage the FFI library for Xcode
```

Dependencies point one way:

```text
isthmus-cli ─┐
             ├─► isthmus-input ─► isthmus-core
isthmus-ffi ─┘
```

Future features (LAN discovery and pairing, file sharing, display
alignment) become sibling crates of `isthmus-input` on top of
`isthmus-core`, and further `op`s of the same FFI export; none is built yet
([roadmap.md](roadmap.md)).

## Threads

| Thread | Owns | Talks to others by |
| --- | --- | --- |
| Tap (`isthmus-tap`) | the active `CGEventTap`, its `CFRunLoop`, the keyboard and mouse engines | reads an `Arc` snapshot (settings, app profile, HID profile, mouse profiles by sender, hotkeys) under a `RwLock` held only to clone the `Arc`; posts actions to the worker |
| HID (`isthmus-hid`) | the HID event-system client, device matching and removal, power notifications, every property write and the journal | commands queued with `CFRunLoopPerformBlock` |
| Main (AppKit) | the `NSWorkspace` observers: app activation, sleep and wake, session switches | publishes the front app and wakes the HID thread |
| Worker (`isthmus-actions`) | blocking side effects: opening apps and Finder windows | a bounded channel |

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
   `{"kind": "status" | "devices" | "settings" | "frontApp", "seq": n}`
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
| `login_item_get` | – | `{"state": LoginItem}` |
| `login_item_set` | `enabled` | `{"state": LoginItem}` |
| `shutdown` | – | message |

The JSON shapes are the serde forms of the Rust types in
[`settings.rs`](../crates/isthmus-input/src/settings.rs),
[`status.rs`](../crates/isthmus-input/src/status.rs),
[`context.rs`](../crates/isthmus-input/src/context.rs) and
[`device.rs`](../crates/isthmus-input/src/device.rs), all camelCase.

Error codes: `INVALID_ARGUMENT`, `NOT_RUNNING`, `ANOTHER_INSTANCE`,
`PERMISSION_MISSING`, `UNSUPPORTED`, `IO`, `INTERNAL`. Dart treats an unknown
code as `INTERNAL`.

## The app

`ui/lib/src`: `ffi/` (raw symbols and loader, the isolate service, the typed
API and its models), `state/` (`AppStore`, one `ChangeNotifier` every page
reads), `shell/` (navigation rail, page frame, window chrome), `pages/`
(overview, keyboard, mouse, settings), `theme/` (colours, type, metrics,
motion, `AppIcons`), `widgets/`, `common/` (l10n, tray). The Runner
(`ui/macos/Runner`) makes the app a menu-bar agent (`LSUIElement`), keeps a
single instance, routes SIGTERM to AppKit, and calls `isthmus_shutdown` from
`applicationWillTerminate`.

## Development workflow

```bash
make build-isthmus-ffi-macos        # build and stage libisthmus_ffi.dylib
cd ui && flutter run -d macos       # permissions then belong to the terminal
make build-ui-macos-local           # Isthmus-local.app, its own TCC identity
make build-isthmus-cli              # target/release/isthmus
cargo test --workspace --all-features
```
