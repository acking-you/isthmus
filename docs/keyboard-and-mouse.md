# Keyboard and mouse

Isthmus makes a Mac's keyboard and mouse behave the way Windows taught your
hands. This page owns that feature: what each setting does, how it is done,
its limits, and what a change must keep. [AGENTS.md](../AGENTS.md) has the
rules; [architecture.md](architecture.md) maps the code.

## Three layers

| Layer | What it changes | Where | Needs |
| --- | --- | --- | --- |
| **HID key mapping** | Which modifier each physical key is, per keyboard | `UserKeyMapping` on the keyboard's HID services ([`hidmap.rs`](../crates/isthmus-input/src/hidmap.rs)) | Nothing; the keyboard tap must run for the Windows profile |
| **Event tap** | Shortcuts one key cannot express, wheel and side buttons | One active `kCGHIDEventTap` on its own thread ([`engine/`](../crates/isthmus-input/src/engine)) | Accessibility |
| **Pointer properties** | Pointer speed and acceleration, per mouse | HID event-system properties ([`pointer.rs`](../crates/isthmus-input/src/pointer.rs)) | Nothing |

Ctrl acting as Command is done by the HID mapping, not by the tap: a mapped
key is a real Command to every app, including password fields, where
macOS's Secure Event Input stops every event tap. The tap then translates
the exceptions (Home/End, Ctrl+arrows, Alt+Tab, Win+D, ...) and guards the
Ctrl chords that must not become Command chords (Ctrl+Q, Ctrl+Tab, ...).

## Keyboard

### Styles and layouts

Each keyboard has a **style**: Windows or Mac. A Mac-style keyboard is never
touched. A Windows-style keyboard gets a mapping chosen by its **layout**
(detected, or set by the user) and by the frontmost app's **class**:

| Profile (app class) | Windows Ctrl becomes | Win becomes | Alt becomes |
| --- | --- | --- | --- |
| Windows (standard apps) | Right Command | Right Control | Right Option |
| NativeCtrl (terminals, JetBrains, Emacs, Vim) | Right Control | Right Command | Left Option |
| Positions (VMs, remote desktops) | Left Control | Left Command | Left Option |

"Windows Ctrl" is the key in Ctrl's place: Control on both layouts. "Win" is
the key in the Windows key's place: Command on a PC keyboard, Option on a
Mac keyboard. "Alt" is next to the space bar: Option on a PC keyboard,
Command on a Mac keyboard. The exact arrays:

| Layout | Profile | `UserKeyMapping` (src → dst, HID page 7) |
| --- | --- | --- |
| PC | Windows | E0→E7, E4→E7, E3→E4, E7→E4, E2→E6 |
| PC | NativeCtrl | E0→E4, E3→E7 |
| PC | Positions | (none) |
| Mac | Windows | E0→E7, E4→E7, E2→E4, E6→E4, E3→E6, E7→E6 |
| Mac | NativeCtrl | E0→E4, E2→E7, E6→E7, E3→E2, E7→E6 |
| Mac | Positions | E2→E3, E3→E2, E6→E7, E7→E6 |

(E0 LeftControl, E1 LeftShift, E2 LeftAlt/Option, E3 LeftGUI/Command, E4
RightControl, E6 RightAlt, E7 RightGUI; each written as `0x7000000E0` and so
on, a 64-bit CFNumber.)

**Why the right-hand side.** macOS does not tell an event tap which keyboard
a key came from, but every event carries device-dependent side bits. A
Windows-style keyboard's modifiers arrive on the right-hand side; a
Mac-style keyboard's arrive on the left. So with a MacBook's own keyboard in
Mac style and a PC keyboard in Windows style, the tap translates only the
PC keyboard's chords, and the MacBook keeps Command+Tab, Command+Left and
Control+A. Apps see ordinary modifiers: macOS shortcuts do not care about
sides. Positions keeps the left side because a remote Windows reads Right
Alt as AltGr. A Mac-style keyboard's own right-hand keys carry the same
marks: in standard apps its right Command reads as Windows Ctrl and its
right Option as Windows Alt. That is the known collision.

**Profiles follow the app.** When the frontmost app's class changes, the
HID thread rewrites every Windows-style keyboard's mapping. macOS releases
keys held across a mapping change, so nothing sticks. Without a running
keyboard tap the Windows profile is not applied (Positions is), because
nothing would then stop Ctrl+Q quitting the app.

**Layout detection.** No HID property says where the modifiers are.
Built-in and Apple keyboards are Mac; Apple's vendor id on another name
(Keychron and other clones in Mac mode use it) is Mac with medium
confidence, a name containing "Mac" is Mac; a KVM switch's emulated
keyboard (Manufacturer "TESmart", vendor 0x1A86 bridge chips) is PC with low
confidence; anything else is PC. The keyboard page shows the guess and lets
the user correct it.

**Keyboards with a Mac/Windows switch.** Keychron and other keyboards that
borrow Apple's vendor id, and brands known for the switch (Lofree, NuPhy,
Akko, Logitech's multi-device keyboards, ...), swap the keys beside the
space bar in firmware: Command in Mac mode, Alt in Windows mode. Flipping
the switch tells macOS nothing (checked on a Keychron K8: same ids, same
service, no reconnect), so for these keyboards the layout is the mode,
and the keyboard page shows it as "Keyboard mode: Mac / Windows". Isthmus
finds it with one key press: it lifts that keyboard's mapping for a moment
(`layout_probe_begin`, at most 20 s), asks for the key just left of the
space bar, and reads it (Command: Mac mode, Option/Alt: Windows mode).
After flipping the switch, the menu bar has a "Windows mode" checkbox per
such keyboard.

**Function keys.** Keyboards that report Apple's vendor id send media keys
on F1–F12 unless Fn is held. Windows style sets `HIDFKeyMode` on their
services so F2, F5 and F11 are F-keys, as on Windows.

### App classes

| Class | Who (built-in lists in [`context.rs`](../crates/isthmus-input/src/context.rs)) | Ctrl | Groups |
| --- | --- | --- | --- |
| Standard | everything else | acts as Command | all |
| Terminal | Terminal, iTerm2, Warp, Ghostty, kitty, WezTerm, Alacritty, Hyper, Tabby, ... | stays Ctrl (Ctrl+C interrupts) | windows, start menu, system, input source, terminal |
| NativeCtrl | JetBrains IDEs, Android Studio, Emacs, Vim, X11 | stays Ctrl | windows, start menu, system, input source |
| Passthrough | Parallels, VMware, UTM, VirtualBox, Windows App, Jump, RustDesk, TeamViewer, AnyDesk, Parsec, Moonlight, ToDesk, Sunlogin, CrossOver, ... | positions only | none |

The user's own rules (bundle id or `prefix*`, and a class) win over the
built-in lists. Apple's Screen Sharing is standard: the other side is a Mac.

### Shortcut table

Read "Ctrl", "Win" and "Alt" as the Windows keys, decoded from the side bits
under the current profile ([`hidmap::decode`](../crates/isthmus-input/src/hidmap.rs)).
An event with a modifier from a Mac-style keyboard is never translated. The
first matching row wins, top to bottom. Every rewrite keeps the event's
non-modifier bits; a changed keycode is sent as a new key event; a key's
autorepeats and its key-up get the translation its key-down got.

**Guards** (always on in standard apps; Ctrl arrives as Command, so these
keep it from meaning Command):

| Windows | Becomes | Why |
| --- | --- | --- |
| Ctrl+Tab, Ctrl+Shift+Tab | Control+Tab (+Shift) | Command+Tab is the app switcher |
| Ctrl+Space | the "previous input source" hotkey (Control+Space) | Command+Space is Spotlight; Windows Ctrl+Space toggles the IME |
| Ctrl+Esc | Spotlight with the start menu group, else Control+Esc | Command+Esc is Game Overlay on macOS 26+ |
| Ctrl+H | see editing; Control+H when editing is off | Command+H hides the app |
| Ctrl+M, Ctrl+Q, Ctrl+Shift+Q, Ctrl+\`, Ctrl+Shift+\` | Control+same | minimize, quit, log out, window cycling |
| Ctrl+Alt+any key | Control+Option+key | AltGr chords; Command+Option+D/H/W/Space are system shortcuts |
| Ctrl+F1 … Ctrl+F20 | Control+F-key | Command+F5 toggles VoiceOver |
| Ctrl+Home / Ctrl+End (+Shift) | Command+Up / Command+Down (+Shift) | document start and end |
| Ctrl+Left / Ctrl+Right (+Shift) | Option+Left / Option+Right (+Shift) | by word; Command+Left is Back in browsers |
| Ctrl+Up / Ctrl+Down (+Shift) | Option+Up / Option+Down (+Shift) | by paragraph |
| Ctrl+Backspace / Ctrl+Delete | Option+Backspace / Option+Forward Delete | by word |

Every other Ctrl chord stays Command: Ctrl+C/V/X/Z/A/S/F/N/O/P/W/T/R/L/D/K/B/I/U,
digits, `-` `=` `0` zoom, `/`, Ctrl+Shift+T/N/P/Z, Ctrl+Return, Ctrl+click
(multi-select).

**Editing** (group `editing`):

| Windows | Becomes | Notes |
| --- | --- | --- |
| Ctrl+Y | Command+Shift+Z | Office: Command+Y |
| Ctrl+H | Command+Option+F (Find and Replace) | browsers: Command+Y (History); Office: Control+H |
| Ctrl+F4 | Command+W | |
| F3 / Shift+F3 | Command+G / Command+Shift+G | VS Code: unchanged; Finder: Command+F |
| Ctrl+PageDown / Ctrl+PageUp | Control+Tab / Control+Shift+Tab | VS Code: Command+Option+Right/Left |
| Ctrl+Insert / Shift+Insert | Command+C / Command+V | the Insert key arrives as Help |

**Navigation** (group `navigation`):

| Windows | Becomes | Notes |
| --- | --- | --- |
| Home / End (+Shift) | Command+Left / Command+Right (+Shift) | not in browsers, where Command+Left is Back; macOS Shift+Home would select to the document start |

**Windows** (group `windows`; also in terminal and NativeCtrl apps):

| Windows | Becomes |
| --- | --- |
| Alt+Tab, Alt+Shift+Tab | the app switcher, held open while Alt is held (below) |
| Alt+F4 | Command+W; browsers and VS Code: Command+Shift+W |
| Win+D | Show Desktop (hotkey 36) |
| Win+E | a new Finder window at home |
| Win+L | Lock Screen (Control+Command+Q) |
| Win+Tab | Mission Control (hotkey 32) |
| Win+Up / Win+Left / Win+Right | Fill / Tile Left / Tile Right (macOS 15+ tiling hotkeys) |
| Win+Down | Command+M (minimize) |
| Win+M | Command+Option+M (minimize the app's windows) |
| Win+Ctrl+Left / Win+Ctrl+Right | the previous / next Space |
| Win+I | System Settings |
| Win+R | Spotlight |
| Win+. , Win+; | the emoji viewer (Control+Command+Space) |

**Start menu** (group `startMenu`): Win pressed and released alone within
300 ms, with no key, click or wheel in between, opens Spotlight (hotkey 64).

**System** (group `system`; also in terminal and NativeCtrl apps):

| Windows | Becomes |
| --- | --- |
| Ctrl+Shift+Esc | Activity Monitor |
| Ctrl+Alt+Delete (or Backspace) | Force Quit (Command+Option+Esc) |
| PrintScreen, Win+Shift+S, Alt+PrintScreen | an area screenshot to the clipboard (Control+Shift+Command+4) |
| Win+PrintScreen | a full-screen screenshot to a file (Shift+Command+3) |

PrintScreen arrives as F13 from a PC keyboard.

**Input source** (group `inputSource`): Win+Space selects the next input
source (hotkey 61). Ctrl+Space is a guard (above). Shift pressed and
released alone within 500 ms selects the previous input source (hotkey 60),
so it goes between the input method and ABC the way Shift switches Chinese
and English in a Windows IME; macOS's own way is Caps Lock. It has its own
setting (`keyboard.shiftSwitchesInput`, on by default), works in terminals
and NativeCtrl apps too, and is used only when an input method (Pinyin,
WeType, Rime …) is among the user's input sources, read from
`com.apple.HIToolbox`: with keyboard layouts alone a Shift tap would swap
layouts, which Windows never does. A key, a click, the wheel or another
modifier in between cancels it, and the Shift events themselves always pass,
so Shift+click, capitals and an IDE's double-Shift are untouched (a double
Shift switches twice, as on Windows). Virtual machines keep Shift for the
IME inside them. An input method with its own Shift switch would switch
twice; turn one of the two off.

**App fixes** (group `appFixes`, standard apps only):

| App | Windows | Becomes |
| --- | --- | --- |
| Browsers | F5 / Shift+F5 / Ctrl+F5 | Command+R / Command+Shift+R / Command+Shift+R |
| | Alt+Left / Alt+Right | Command+[ / Command+] |
| | Alt+D, F6 | Command+L |
| | F11 | Control+Command+F (full screen) |
| | F12 | Command+Option+I |
| | Alt+Home | Command+Shift+H |
| VS Code family | Ctrl+G, Ctrl+R, Ctrl+Shift+G, Ctrl+Q | Control+same (go to line, recent, source control, view picker) |
| | Ctrl+Up / Ctrl+Down | Control+PageUp / Control+PageDown (scroll by line) |
| | Ctrl+Alt+Up / Ctrl+Alt+Down | Command+Option+Up / Down (add cursor) |
| | Ctrl+Shift+[ / ] | Command+Option+[ / ] (fold) |
| | Alt+Left / Alt+Right | Control+- / Control+Shift+- (navigate back / forward) |
| Office | F12 | Command+Shift+S (Save As) |
| Finder | F2 | Return (rename) |
| | Alt+Up | Command+Up (enclosing folder) |
| | Alt+Left / Alt+Right | Command+[ / Command+] |
| | Alt+Return | Command+I (Get Info) |

**Terminal** (group `terminal`, terminal apps only; Ctrl arrives as Control):

| Windows Terminal | Becomes |
| --- | --- |
| Ctrl+Shift+C, Ctrl+Insert | Command+C |
| Ctrl+Shift+V, Shift+Insert | Command+V |
| Ctrl+Shift+T / W / N / F | Command+T / W / N / F |
| Ctrl+= / Ctrl+- / Ctrl+0 | Command+= / - / 0 |
| Ctrl+Left / Ctrl+Right | Option+Left / Option+Right |
| Ctrl+Backspace | Control+W |

In terminal and NativeCtrl apps Ctrl+Left/Right/Up/Down would otherwise hit
macOS's Space and Mission Control hotkeys, so they become Option+arrows
whatever the groups say.

### Alt+Tab

Command+Tab's switcher stays open only while Command is held, so turning
Alt+Tab into Command+Tab once is not enough. On the first Tab with Alt held,
the tap posts Command down (`flagsChanged`) and sends the Tab as
Command+Tab, with Option taken off it. While the session lasts, Tab, Shift,
the arrows, Esc, Q, H and the backtick key are sent with Command. Alt's own
release passes through and is followed by a posted Command up, and the
switcher commits. Isthmus never posts an Alt release of its own, so the
physical key's state stays true: a watchdog reads it every 150 ms and ends
a session whose release was missed (Secure Input, a disabled tap).

### Hotkeys

Show Desktop, Mission Control, Spotlight, the input-source switches,
Spaces and tiling are macOS symbolic hotkeys. Isthmus posts the chord the
user has bound (read from `com.apple.symbolichotkeys`, else the macOS
default) and does nothing for one the user switched off.

## Mouse

Trackpads and the Magic Mouse are never changed: only notched-wheel events
(`kCGScrollWheelEventIsContinuous` 0, no scroll or momentum phase) are.
Each event names the HID service that sent it (field 87), so each mouse
gets its own profile; an unknown sender gets the defaults.

| Setting | Windows preset | What it does |
| --- | --- | --- |
| Wheel direction | Windows | Rolling toward you scrolls down, even with natural scrolling on for the trackpad. The tap flips both axes when field 137 says macOS inverted the event |
| Linear wheel, lines per notch | on, 3 | Each notch scrolls N lines, without acceleration: line delta ±1, fixed-point delta ±N, pixel delta ±10·N (Chrome and Safari scroll 40·N px, AppKit 10·N pt, VS Code ~50 px per notch) |
| Pointer | Windows, speed 10 | macOS 14+ linear tracking with Windows' gain for the 1–20 speed: Windows with "Enhance pointer precision" off. Without linear support, acceleration off |
| Side buttons | on | Buttons 4 and 5 send a back / forward swipe. Chrome, Edge, Brave, Arc, Vivaldi, Chromium, VS Code, VSCodium, Cursor, Antigravity, JetBrains IDEs and Android Studio handle them natively and get them untouched, as do Passthrough apps; every other app, Safari, Firefox and other Electron apps included, gets the swipe. To give another app the raw buttons, turn side buttons off for that mouse, or give the app a Passthrough rule (which also stops its keyboard translations) |
| Ctrl+wheel zoom | on | One Command+= / Command+- per notch; Office keeps Command+wheel |

Deltas are written in the order line, fixed, pixel: setting the line delta
makes macOS recompute the other two.

The Windows pointer gain is divided by the main display's panel pixels per
point, because Windows moves the pointer in pixels and macOS in points: a
2560-pixel panel shown as 2048 points gives 1.25, so speed 10 becomes a gain
of 0.8, and a mouse shared through a KVM crosses the same share of the same
monitor as it does on Windows.

## Settings and one click

What Isthmus does is set in one file,
`~/Library/Application Support/Isthmus/settings.json`
([`settings.rs`](../crates/isthmus-input/src/settings.rs)):

| Part | What it holds |
| --- | --- |
| **The switch** | Off puts back every system change and removes the event tap (pause); on applies the two sections below. A new install starts off. |
| **Keyboard** | Its own switch; the style of a keyboard with no entry of its own; per keyboard its style, layout (auto, Mac or PC) and function keys (standard, or as System Settings has them); the eight shortcut groups; whether a lone Shift switches input sources; the user's app rules |
| **Mouse** | Its own switch; the profile of a mouse with no entry of its own; per mouse its own profile: the settings in the mouse table, with 1–12 lines per notch and speed 1–20 |

A device's entry follows it across replugs, KVM switches and restarts: its
id is the transport, vendor, product, kind and serial number, or the USB
port when there is no serial ([`device.rs`](../crates/isthmus-input/src/device.rs)).
A device with an entry of its own is listed while disconnected too.

Presets are values, not modes. Each applies to the keyboard, the mouse or
both in one click:

- **Windows** puts every keyboard in Windows style with standard function
  keys, every group on and the Shift switch on, makes the Windows profile
  every mouse's, and turns the switch on.
- **Mac** puts every keyboard in Mac style with the function keys System
  Settings sets and the Shift switch off, and gives every mouse the Mac profile, which leaves the
  wheel, the pointer and the side buttons as macOS has them and turns zoom
  off. It leaves the switch alone; with both sections Mac, Isthmus changes
  nothing.

Layout corrections and app rules are facts about the user's hardware and
apps, not style, so presets keep them. A mouse preset drops the per-mouse
profiles so every mouse follows it. A page shows **Custom** when its values
match neither preset.

Every later version reads this file: fields are added with defaults,
unknown keys are kept when the file is rewritten (at the top level, in the
keyboard and mouse sections, and in each device entry and app rule), and a
value this version does not know reads as the one that changes nothing. The
shortcut groups and a mouse profile keep no unknown keys, so a later version
adds its settings beside them, never inside. A file that cannot be read is
reported, never replaced by the defaults.

## System changes and putting them back

Every HID property Isthmus writes is journalled first in
`~/Library/Application Support/Isthmus/journal.json`: the service, the key,
the original value (the first original wins) and what Isthmus wrote. On
quit (`applicationWillTerminate`, a routed SIGTERM), on pause, on switching
to another user, and at the next start after a crash, every entry still as
Isthmus left it is put back; an entry someone else changed is left alone. A
journal from a previous boot is discarded: a restart already undid it.
Event taps need no journal: the window server drops a dead process's tap.
`isthmus restore` puts back a journal without starting anything else, and
refuses while a running Isthmus holds the lock.

If Isthmus cannot run at all, unplugging and replugging a device clears
everything it set on that device, and restarting the Mac clears all of it.
Key mappings alone can be cleared from a terminal:

```bash
hidutil property --set '{"UserKeyMapping":[]}'   # every keyboard's mapping, yours included
```

## Worth knowing

- **Without Accessibility there is no tap.** macOS refuses an event tap
  that changes events to an app without it. Keyboards then get only the
  Positions mapping (Win and Alt in their Windows places, Ctrl as Control),
  and mice only pointer speed.
- **Secure Input pauses the tap.** While a password field or a terminal's
  "Secure Keyboard Entry" holds it, the tap receives no key events. In a
  password field the HID mapping still works, so Ctrl+V pastes, but
  Home/End, Alt+Tab and every other tap translation stop, the guards
  included: Ctrl+Q there is Command+Q.
- **Secure Keyboard Entry stops terminal copy and paste.** A terminal keeps
  Ctrl as Control, so with it on, Ctrl+Shift+C reaches the terminal as
  Control+Shift+C, which it sends as ^C, interrupting the running command;
  every other terminal translation stops too. Win+C and Win+V (Command)
  still copy and paste.
- **A KVM switch is a replug.** Every switch drops the HID mappings and
  pointer properties with the old services; Isthmus re-applies them when
  the new services appear.
- **Keycodes are positions.** Rules match US positions, so Ctrl+Z on a
  German keyboard is the key labelled Y.
- **System Settings' Modifier Keys come first.** macOS applies them before
  Isthmus's mapping, so a keyboard changed in both places gets both; the
  overview warns about it.
- **Karabiner-Elements takes keyboards whole.** A keyboard it has seized
  never sees Isthmus's mapping; the overview says so.
- **macOS 13 or later.** Linear pointer speed needs macOS 14 (before it,
  acceleration is switched off instead), and Win+arrow tiling needs macOS 15.
- **Granting is a drag and a switch.** Grant opens System Settings at the
  Accessibility pane (without the system's own prompt) and puts a small
  card under the Settings window holding Isthmus's icon: drag it into the
  list, turn it on. The card follows the window and goes away once the
  grant is seen.
- **A grant takes effect without a restart.** While the keyboard tap waits
  for Accessibility, Isthmus watches tccd's change notification and reads
  the permission twice a second, and tries the tap again as soon as a
  check that never prompts says it may be allowed. If macOS still refuses
  the tap with the grant in place, the app restarts itself once; a second
  refusal within three minutes is shown on the overview instead.
- **Accessibility follows the code signature.** An ad-hoc signed
  development build loses the grant on every rebuild of the Rust library;
  `tccutil reset Accessibility dev.acking.isthmus-local` clears the stale
  entry.
- **One Isthmus per Mac.** HID state is machine-wide; a second copy (another
  build, or `isthmus run`) runs but changes nothing.
