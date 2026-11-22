# Isthmus app

The Flutter app: a menu bar agent with one window of four pages (Overview,
Keyboard, Mouse, Settings). It calls Rust through the C ABI in
`native/isthmus_ffi`, one JSON `op` at a time on a worker isolate; the ops,
their JSON and the error codes are in
[docs/architecture.md](../docs/architecture.md).

```text
lib/
├── main.dart              # font licences, the hidden window, change hints, runApp
├── l10n/                  # app_en.arb and app_zh.arb (every key in both)
└── src/
    ├── app.dart           # the store, the tray, showing and hiding the window, quitting
    ├── ffi/               # loader, isolate service, typed client, JSON models
    ├── state/             # AppStore (reads, hints, serialized optimistic writes), shortcut examples
    ├── shell/             # rail, page frame, window chrome, preferences
    ├── pages/             # overview, keyboard, mouse, settings
    ├── theme/             # colours, type, radii, motion, glass, AppIcons (Lucide only)
    ├── widgets/           # surfaces, status chips, keycaps, setting rows, brand mark
    └── common/            # l10n, locale, toasts, tray
```

Rules the code keeps:

- The settings model keeps the raw JSON and edits a copy of it, so keys a
  newer library writes survive this app.
- The window starts hidden (`MainFlutterWindow.swift`); the app shows it on
  first run, while Accessibility is missing, or when the service did not
  start. Closing hides it. Quitting goes through
  `ServicesBinding.instance.exitApplication(AppExitType.required)`, never
  `dart:io` `exit()`, which would skip `applicationWillTerminate` and leave
  the keyboard remapped.
- Icons come from `AppIcons`, never Material `Icons.*`.
- `tool/gen_tray_icon.dart` draws `assets/tray/tray_icon*.png` from the same
  geometry as the brand mark.

## Checks

From `ui/`:

```bash
flutter pub get
flutter gen-l10n                 # l10n_untranslated.json must stay {}
dart format --output=none --set-exit-if-changed lib test tool
flutter analyze --no-fatal-infos
flutter test
grep -rnE '\bIcons\.' lib        # must find nothing
```

`ISTHMUS_UI_PREVIEW_DIR=/tmp/isthmus flutter test test/preview_test.dart`
renders every page to PNG with the real faces; add
`ISTHMUS_UI_PREVIEW_CJK=<a .ttf with Chinese>` for the Chinese ones.

## Building

```bash
PROFILE=debug bash ../scripts/build/isthmus-ffi-macos.sh   # stage libisthmus_ffi.dylib
flutter build macos --debug
```

`flutter run -d macos` makes the terminal the app's responsible process, so
macOS grants Accessibility to the terminal, not to Isthmus; test permissions
with the app opened from Finder or `open`.
