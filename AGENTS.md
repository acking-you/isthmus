# Repository Guidelines

Rules for changing this repository. `CLAUDE.md` is a symlink to this file.
[docs/architecture.md](docs/architecture.md) maps the code;
[docs/README.md](docs/README.md) indexes the design docs.

## Subagents
- Run subagents, workflow agents and delegated sessions on a cheap, practical
  model (Claude Opus 5.5 rather than Claude Fable 5.2, for example) unless the
  user asks for another.
- Name the model when spawning; left unset, an agent inherits the session's.

## Architecture
- One `isthmus` binary (`crates/isthmus-cli`) with the commands `devices`,
  `status`, `settings`, `preset`, `run`, `restore` and `inspect`, and one
  app (`ui/`, for macOS and Windows); both drive `isthmus-input`, and the
  app also drives `isthmus-link` (discovery, pairing, file sharing) and
  `isthmus-awake` (keeping the computer awake while its screen sleeps).
- Dependencies point one way: `isthmus-cli` → `isthmus-input`, and
  `isthmus-ffi` (`ui/native/isthmus_ffi`) → `isthmus-input`,
  `isthmus-link` and `isthmus-awake`, all → `isthmus-core`. A new feature
  (display alignment, keyboard and mouse across devices) is a sibling crate
  on `isthmus-core`, never a module inside another
  ([docs/roadmap.md](docs/roadmap.md)).
- A feature crate names an operating system's API only under
  `src/platform/<os>`, chosen at compile time: macOS for `isthmus-input`
  (every other OS gets `Unsupported`), macOS and Windows for
  `isthmus-link` and `isthmus-awake`. Everything above it (settings, device
  grouping, the mapping tables, the engines, the journal's decisions, path
  ranking, the wire protocol) is plain data or portable async code, tested
  on any host.
- The Windows app is file sharing, keep-awake and updates: the keyboard
  and mouse pages, the restore and System Settings cards, and the tray's
  Windows-habits switch exist on a Mac alone (`hasInputPages`).
- The event tap's decisions live in `isthmus-input/src/engine/` with no OS
  types: the platform turns a `CGEvent` into the engine's values and carries
  out its answer. A new shortcut is a rule there, with a test, and a row in
  [docs/keyboard-and-mouse.md](docs/keyboard-and-mouse.md).
- The app calls Rust through `ui/native/isthmus_ffi`, the Flutter C ABI. A
  call changes on every side at once: the Rust op, the Dart client
  interface, its implementation and the test fake. A new operation is an
  `op` of `isthmus_call_json`, not a new export; `isthmus_shutdown` exists
  for the Swift quit hook alone.
- Library, binary and bundle names are hardcoded by the Dart loader, the
  xcconfigs (`-listhmus_ffi`, `ISTHMUS_LOCAL_SUFFIX`), the Xcode build phase
  that embeds the library, `ui/windows/CMakeLists.txt` (`Isthmus.exe`,
  `isthmus_ffi.dll` from `ui/native/windows/x64`), `scripts/build/`, the
  Makefile and `.gitignore`; do not rename them. The bundle id is
  `dev.acking.isthmus`, `-local` for the local build; Accessibility grants
  hang on it.
- A private macOS symbol is looked up with `dlsym`, used read-only or with no
  side effect that outlives the process, and has a public fallback.

## Compatibility invariants
- Every HID property Isthmus writes is journalled in `journal.json` before it
  is written, and put back on quit, on pause, on a switch to another user
  and, after a crash, at the next start; never over a value someone else set
  meanwhile (`journal::recover`). A journal from another boot is discarded.
- A keyboard's `UserKeyMapping` holds Isthmus's entries only for the
  modifier sources; entries for any other source belong to someone else and
  are kept on every rewrite (`hidmap::merge`).
- The Windows HID profile is applied only while the keyboard tap runs
  (`hidmap::profile_for`): without the tap nothing stops Ctrl+Q quitting.
- Trackpads and the Magic Mouse are never changed: only notched-wheel events
  are rewritten, and only mouse services get pointer properties.
- Settings files (`settings.json`, and each future feature's own) are read
  by every later version: fields are added with defaults, unknown keys are
  kept when the file is rewritten, and an unknown enum value reads as the
  variant that changes nothing. A struct keeps unknown keys through a
  `#[serde(flatten)] unknown` map; in `settings.json` that is the top level,
  the keyboard and mouse sections, each device entry and each app rule. A
  new field goes only into a struct with that map, never into the `Copy`
  `ShortcutGroups` or `MouseProfile`. A file that cannot be read is an error
  the caller reports, never silently the defaults. State files are replaced
  atomically (`isthmus_core::durable_file`).
- The FFI's `ErrorCode` strings and change kinds never change meaning; Dart
  reads an unknown code as `INTERNAL` and an unknown kind as "something
  changed".
- Two versions of Isthmus on two computers keep talking: a message, field
  or enum value of the link protocol a version does not know reads as
  `unknown` or as the value that changes nothing, and a change older
  versions cannot read bumps the version byte of the preamble
  (`isthmus-link/src/wire.rs`). The pinned keys in `peers.json` and the key
  in `link-identity.json` are never regenerated silently: an unreadable
  identity file is an error, since a new key unpairs every device.
- Nothing from the network names a place on disk unchecked: offered paths
  go through `files::safe_relative`, and files land in a staging folder
  until all of them are complete.
- One instance per Mac: the `instance.lock` `flock` decides which process
  changes the system. Every other one (another build, or `isthmus run`) runs
  but changes nothing. Release and `-local` builds share the lock; likewise
  `link.lock` decides which process announces the device and listens.
- The tap callback never blocks, never takes a lock beyond cloning the
  snapshot `Arc`, allocates nothing on pass-through, and calls no
  Objective-C, AX or file API. Slow side effects go to the
  `isthmus-actions` worker. Every event Isthmus posts carries
  `ISTHMUS_TAG`, so its own tap passes it through.

## Rust
- Read [docs/rust-engineering-guide.md](docs/rust-engineering-guide.md) before
  writing or changing Rust; where it differs from this file or the
  repository's lints, they win.
- Lints: `unwrap_used` and `expect_used` are denied workspace-wide, and a new
  crate takes `[lints] workspace = true`. `#[cfg(test)]` code is exempt through
  `clippy.toml` and needs no attribute; each `tests/` or `examples/` target
  that unwraps carries a file-level
  `#![allow(clippy::unwrap_used, clippy::expect_used)]`. A production
  `unwrap` or `expect` records its reason at the site. Silence any other
  lint with `#[expect(lint, reason = "...")]` on the narrowest item.
- Errors: snafu in `isthmus-core` and `isthmus-input`, wrapping the layer
  below as `source`; the FFI keeps its hand-written `ErrorCode`. No
  `thiserror` or `anyhow`. Callers branch on variants and codes, never on
  message text.
- Every item has a doc comment saying why, in full sentences; a module's
  `//!` states its design decision. Every `unsafe` block has a `// SAFETY:`
  line, and every `unsafe fn` a `# Safety` section.
- A closed set is an enum, matched without a `_` arm where each variant
  needs a decision.
- Use what exists: `isthmus_core::paths` for where files live,
  `isthmus_core::durable_file` for state files, `isthmus_core::instance` for
  the lock.
- Edition 2024 on Rust 1.100.0: `rust-toolchain.toml`, `rust-version` and
  any CI workflow change together.

## Performance
- The tap callback runs for every key, button and wheel event on the Mac,
  in the path to every app. Pass-through allocates nothing and holds a
  lock only to clone the snapshot `Arc`; the snapshot is replaced whole,
  never edited in place.
- macOS disables a tap whose callback is too slow. Keep the callback
  bounded, re-enable on `kCGEventTapDisabledByTimeout`, and release every
  synthetic key Isthmus holds when the tap is disabled.
- Measure before and after a change to the tap path, and report the
  environment with any number. Never put an unmeasured figure in docs.

## Dependencies
- Declare each dependency once in `[workspace.dependencies]`; member crates
  take it with `.workspace = true`. Turn on only the features in use.
- macOS APIs come from the objc2 family (`objc2` 0.6, the `objc2-*` crates
  0.3.2), as pb-mapper pins them. Never `core-foundation`, `core-graphics`
  or `io-kit-sys`: their types do not mix with objc2's.

## Verification
Run what your change touches and say what you could not run.
- Rust: `cargo fmt --all`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo test --workspace --all-features` and
  `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`.
- App, from `ui/`: `flutter gen-l10n` leaving `l10n_untranslated.json` as
  `{}`, `dart format --output=none --set-exit-if-changed lib test`,
  `flutter analyze --no-fatal-infos` and `flutter test`. Icons come from
  `AppIcons`, never Material `Icons.*` (`grep -rnE '\bIcons\.' lib` finds
  none).
- Platform code (`platform/macos`, the Runner) runs only on a Mac, and the
  tap only with Accessibility granted to the process that runs it; unit
  tests cannot reach it. Say which of it you could not exercise.
- Windows: `make check-windows` lints every crate and its tests for
  `x86_64-pc-windows-msvc` from any host, and CI's `windows-test` runs the
  workspace's tests on a Windows runner, where file and power APIs behave
  differently from a Mac's. The Windows runner and the app on Windows run
  only on a PC; say so when a change touches them.
- File sharing between two machines needs Local Network access: a process
  started from a sandboxed tool or an agent's shell may get `No route to
  host` for every LAN packet. The loopback tests cover the protocol; try
  discovery from the app or from Terminal.

## Testing
- Cover core capabilities and real failure, state and security boundaries;
  avoid wording, glue and repetitive input matrices.
- Name a test for the behaviour it pins, as a sentence, with a `///` line
  stating the contract.
- Never change the live system in a test: no HID property writes, no active
  event taps, no posted events, no `defaults write`, `tccutil` or login
  items, no power assertions or requests, no installers run. The decisions
  are pure functions tested on plain values; what needs the system is
  checked by hand, read-only where possible.
- Link tests run services on loopback with discovery off and scratch data
  and receive folders; no test announces on the real network or writes to
  Downloads.
- Tests share a process with parallel threads, so unit tests never write the
  environment. Wait on readiness, not sleeps.
- Widget tests drive a fake `IsthmusApiClient` and never load the library.

## Commits, PRs and releases
- Commits: short and imperative. PRs: summary, rationale, verification, and doc
  updates when behavior changes.
- A user-visible change adds a bullet under `## [Unreleased]` at the top of
  `CHANGELOG.md`; a release renames that heading. The English and Chinese
  pages of a pair change together (`README.md` and `README.zh-CN.md`,
  `docs/keyboard-and-mouse.md` and its `.zh-CN.md`), and a new, renamed or
  retired page updates `docs/README.md`.
- A `vX.Y.Z` tag, the workspace version and `ui/pubspec.yaml` must match, and
  the pubspec's build number only rises. Never reuse a tag or republish a
  version.
- Pushing the tag runs `.github/workflows/release.yml`: a Developer
  ID-signed, notarized DMG per macOS architecture
  (`scripts/release/macos_package.py`), the Windows installer (fastforge
  and Inno Setup, `ui/windows/packaging`, its update contract checked by
  installing it) and portable zip, and `release-manifest.json` signed with
  the update key, all published at once with the notes from that version's
  CHANGELOG section. Run it by hand with `verify_only` to check signing
  without publishing. The signing secrets and variables it names live on
  the repository; never in the tree.
- The update key (`UPDATE_SIGNING_KEY`; its public half in
  `scripts/release/update_signing_key.pub.pem` and `updatePublicKey` in
  `ui/lib/src/updates/release.dart`) and the installer's AppId never
  change: a new key strands every installed copy on its version, and a new
  AppId installs beside the old installation. The asset names are the
  updater's (`packageFor`) and `ui_release_manifest.py`'s; they change
  together, and never for a published version ([docs/updates.md](docs/updates.md)).
- `version` stays the first key of `[workspace.package]`.
- Never commit secrets or signing identities.
