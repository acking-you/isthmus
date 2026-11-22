# Updates and releases

Isthmus updates itself from this repository's GitHub releases, after
pb-mapper's updater. This page covers what the app does, why it can trust
what it installs, how each platform puts the new version in place, and how
a release is made. The code is in `ui/lib/src/updates/`,
`ui/native/isthmus_ffi/src/update.rs`, `scripts/release/` and
`.github/workflows/release.yml`.

## What the user sees

Settings › About has **Check for updates**, which becomes **Update to X**
once a check has found a newer release, and **Check for updates
automatically**, on by default. The automatic check runs at launch and then
about every twelve hours while the app keeps running; it announces each new
version once, as a toast, and downloads nothing. The dialog says which file
it will fetch, how big it is, and what will happen, before the user presses
**Update**.

After an update that replaces the app, the next launch says whether it went
in ("Updated to Isthmus X", or "Isthmus X was not installed" with what to do
next): the app quit for the update, so the person never saw it finish.

## Trust

A release is an update only if every link of this chain holds:

1. **The newest release.** `GET api.github.com/repos/acking-you/isthmus/releases`;
   the highest `vX.Y.Z` tag that is neither a draft nor a prerelease, and
   newer than the running app's own version (`package_info_plus`, which
   reads the pubspec's version from the bundle).
2. **Its manifest is signed by the release key.** The release carries
   `release-manifest.json` and `release-manifest.json.sig`, the base64 of an
   Ed25519 signature over the manifest's exact bytes. The app checks it in
   Rust (`update_verify`, ed25519-dalek, strict verification) against the
   public key it carries (`updatePublicKey` in `release.dart`). A release
   without the two files is "unsigned" and is pointed at, never installed.
3. **The manifest is this release's.** Its `version` must equal the
   release's tag, so a genuinely signed manifest of an older release cannot
   stand in for a newer one.
4. **The file is the one the manifest lists.** The download is streamed to
   disk and hashed as it goes; one byte more than the listed size, or a
   SHA-256 that differs, and nothing is installed. A file name from the
   manifest is a plain name (`[A-Za-z0-9][A-Za-z0-9._-]*`), never a path.
5. **On a Mac, the new app is this app.** Before the swap, `codesign
   --verify --deep --strict` must pass on the copy taken out of the disk
   image, and its identifier and Developer ID team must equal the running
   app's. That signature is also what Accessibility's grant hangs on, so the
   grant carries over.

## Putting it in place

| This copy | Release file | What happens |
| --- | --- | --- |
| Mac, Developer ID-signed, in a folder the user can write to | `Isthmus-macos-<arch>.dmg` | The image is mounted out of sight, its `Isthmus.app` copied with `ditto` into a hidden folder beside this one and checked (5. above), and a detached helper waits for the app to quit, swaps the two with renames (putting the old one back if the second rename fails), and opens the app. |
| Mac, otherwise (a `-local` build, a read-only folder) | `Isthmus-macos-<arch>.dmg` | Saved to Downloads and opened, for the user to drag into Applications. |
| Windows, installed with Setup (`unins000.exe` beside the app) | `Isthmus-windows-x86_64-setup.exe` | Setup runs with `/SILENT /SUPPRESSMSGBOXES /NORESTART /SP- /LOG= /UPDATEPID=<pid>` while the app is still in front, so Windows' permission prompt shows in front of it; the app quits once Setup's log appears (Setup starts it only after that permission), Setup waits for the app to be gone, installs, and opens the installed app again as the user. |
| Windows, portable | `Isthmus-windows-x86_64.zip` | Saved to Downloads and shown in Explorer, to unpack over this copy. |

A debug build downloads and verifies, then stops ("a development build does
not replace itself"). Before the app quits for an update it writes where it
was going (`pendingUpdate` in its preferences); the next launch reads that
once, compares it with its own version, and removes the Windows installer
that could not remove itself.

## The Windows installer

`ui/windows/packaging/exe/inno_setup.iss` is fastforge's Inno Setup script,
owned here. Three things in it are contracts:

- **`AppId`** (`make_config.yaml`) identifies every earlier installation to
  an upgrade; it never changes, or an upgrade installs beside the old copy.
- **`PrivilegesRequired=none`**: an administrator's Setup asks Windows for
  permission and installs for all users in Program Files, anyone else's for
  that user alone, and an upgrade finds the installation of its mode.
- **`/UPDATEPID`**: `PrepareToInstall` waits up to a minute for that process
  to end before replacing a file, and `DeinitializeSetup` opens
  `{app}\Isthmus.exe` again afterwards.

The release workflow installs the installer it just built the way the app
runs it, with a stand-in process for the app, and fails unless the log shows
it waiting, closing, installing and reopening in that order, the installed
files match the bundle, `unins000.exe` exists and every shortcut opens
`Isthmus.exe`.

## The update key

The manifest is signed with an Ed25519 key made for Isthmus:

- the private half is the repository secret `UPDATE_SIGNING_KEY` (PEM) and
  the maintainer's offline backup, never in the tree;
- the public half is `scripts/release/update_signing_key.pub.pem` (for the
  workflow's check) and `updatePublicKey` in
  `ui/lib/src/updates/release.dart` (its 32 raw bytes in base64, for the
  app).

The key never changes: every installed copy trusts only it, so a release
signed with another key cannot update them, and they stay on their version
until someone installs a new one by hand. If the private key is lost, the
next release must say so and ship the new public key, and every user
updates once by hand.

## Making a release

1. Rename `## [Unreleased]` in `CHANGELOG.md` to the version, and set the
   same version in `Cargo.toml` (`[workspace.package]`) and `ui/pubspec.yaml`
   (raising its build number).
2. Optionally run the Release workflow by hand with `verify_only`: it
   builds, signs, notarizes, installs the Windows installer, and builds and
   signs the manifest, without publishing.
3. Push the `vX.Y.Z` tag. The workflow builds the two DMGs and the Windows
   packages, writes the manifest (`ui_release_manifest.py build`, which
   refuses a missing package or a tag that differs from the pubspec), signs
   it and checks the signature against `update_signing_key.pub.pem`, then
   creates the release with every file at once, so no client ever sees a
   release without its manifest, and finally checks that each published
   file's size and SHA-256 are the manifest's (`release_assets.py verify`).
