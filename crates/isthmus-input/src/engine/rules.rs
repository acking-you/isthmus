//! The Windows shortcut table (docs/keyboard-and-mouse.md, "Shortcut table").
//!
//! The table is data, not code: one [`Rule`] is one Windows chord in one
//! scope, and [`decide`] walks [`RULES`] and takes the first row that
//! matches. Keeping it as a flat, ordered list makes every row testable on
//! its own and keeps the per-key cost to a scan of a static slice, with no
//! allocation on the tap thread.
//!
//! Rows match the **exact** set of Windows modifiers (Ctrl, Win, Alt,
//! Shift) that [`crate::hidmap::decode`] read from the side bits, never a
//! subset, so Ctrl+Shift+T is not Ctrl+T. A row marked `carry_shift`
//! matches with or without Shift and adds Shift to its output when held,
//! which is how the "(+Shift)" rows of the documentation read.
//!
//! The documentation lists groups in reading order; the table lists them by
//! precedence, most specific first, because the documented rows overlap and
//! the specific one must win:
//!
//! 1. app fixes (an app's own binding beats the general rule: VS Code's
//!    Ctrl+Up, a browser's Ctrl+F5);
//! 2. system (Ctrl+Alt+Delete beats the Ctrl+Alt guard);
//! 3. windows, start menu, input source;
//! 4. terminal (terminal apps only);
//! 5. editing and navigation, each app variant before its general row
//!    (Ctrl+H in a browser before Ctrl+H elsewhere);
//! 6. the guards last: they are always on in their scope and stand in for a
//!    group that is switched off (Ctrl+H, Ctrl+F4, Ctrl+Esc), so a group
//!    that is on must come first.

use crate::context::{AppClass, AppProfile, AppTraits};
use crate::hidmap::WinMods;
use crate::keys::{KeyCode, Mods, kc};
use crate::settings::ShortcutGroups;

use super::{Action, Chord, SystemHotkeys};

/// What switches a row on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Group {
    /// Always on in its scope: the rows that keep a Windows Ctrl chord, which
    /// the HID layer made a Command chord, from meaning a macOS Command
    /// shortcut (Command+Q quits, Command+Tab switches apps).
    Guard,
    Editing,
    Navigation,
    Windows,
    StartMenu,
    System,
    InputSource,
    AppFixes,
    Terminal,
}

impl Group {
    /// Whether the user has this group on. Every variant is decided here,
    /// so a new group cannot be silently always-on.
    fn is_on(self, groups: &ShortcutGroups) -> bool {
        match self {
            Group::Guard => true,
            Group::Editing => groups.editing,
            Group::Navigation => groups.navigation,
            Group::Windows => groups.windows,
            Group::StartMenu => groups.start_menu,
            Group::System => groups.system,
            Group::InputSource => groups.input_source,
            Group::AppFixes => groups.app_fixes,
            Group::Terminal => groups.terminal,
        }
    }
}

/// The app classes a row applies in. Which keys arrive as what depends on
/// the class (Ctrl is Command in standard apps, Control in the others), so
/// a row is written for the classes whose arrival it expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    /// Standard apps, where Ctrl arrives as Command.
    OnlyStandard,
    /// Terminals only (the terminal group).
    OnlyTerminal,
    /// Terminals and NativeCtrl apps, where Ctrl arrives as Control.
    CtrlNative,
    /// Every class that is translated at all: all but passthrough.
    Translated,
}

impl Scope {
    /// Whether a row of this scope applies while an app of `class` is in
    /// front. Passthrough apps are never translated.
    fn includes(self, class: AppClass) -> bool {
        let (standard, terminal, native_ctrl) = match self {
            Scope::OnlyStandard => (true, false, false),
            Scope::OnlyTerminal => (false, true, false),
            Scope::CtrlNative => (false, true, true),
            Scope::Translated => (true, true, true),
        };
        match class {
            AppClass::Standard => standard,
            AppClass::Terminal => terminal,
            AppClass::NativeCtrl => native_ctrl,
            AppClass::Passthrough => false,
        }
    }
}

/// The app a row is limited to: the per-app variants of a row (Ctrl+H in a
/// browser) sit before the general one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AppIs {
    AnyApp,
    Browser,
    VsCode,
    Office,
    Finder,
}

impl AppIs {
    fn matches(self, traits: &AppTraits) -> bool {
        match self {
            AppIs::AnyApp => true,
            AppIs::Browser => traits.browser,
            AppIs::VsCode => traits.vscode,
            AppIs::Office => traits.office,
            AppIs::Finder => traits.finder,
        }
    }
}

/// The keys a row matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Keys {
    One(KeyCode),
    List(&'static [KeyCode]),
    /// F1 to F20.
    FunctionRow,
    /// Any key that is not a modifier (modifiers arrive as `flagsChanged`
    /// and never reach the table).
    Any,
}

impl Keys {
    /// Whether `keycode` is one of these keys.
    fn contains(self, keycode: KeyCode) -> bool {
        match self {
            Keys::One(one) => one == keycode,
            Keys::List(list) => list.contains(&keycode),
            Keys::FunctionRow => kc::is_function_key(keycode),
            Keys::Any => !kc::is_modifier(keycode),
        }
    }
}

/// A set of Windows modifiers, side-agnostic: what a row requires held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(super) struct Held {
    pub ctrl: bool,
    pub win: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Held {
    /// Nothing held: a key pressed alone.
    const NONE: Held = Held {
        ctrl: false,
        win: false,
        alt: false,
        shift: false,
    };

    /// Both sets together, to name the combinations the table uses.
    const fn and(self, other: Held) -> Held {
        Held {
            ctrl: self.ctrl || other.ctrl,
            win: self.win || other.win,
            alt: self.alt || other.alt,
            shift: self.shift || other.shift,
        }
    }

    /// The modifiers a decoded event holds. `foreign` is the caller's
    /// business: a foreign event never reaches the table.
    pub(super) fn of(mods: WinMods) -> Held {
        let WinMods {
            ctrl,
            win,
            alt,
            shift,
            foreign: _,
        } = mods;
        Held {
            ctrl,
            win,
            alt,
            shift,
        }
    }

    /// The same set with Shift ignored, for rows that carry it.
    const fn without_shift(self) -> Held {
        Held {
            shift: false,
            ..self
        }
    }
}

/// The Windows modifier sets the table uses, named as Windows writes them.
mod win {
    use super::Held;

    pub(super) const NONE: Held = Held::NONE;
    pub(super) const CTRL: Held = Held {
        ctrl: true,
        ..Held::NONE
    };
    pub(super) const WIN: Held = Held {
        win: true,
        ..Held::NONE
    };
    pub(super) const ALT: Held = Held {
        alt: true,
        ..Held::NONE
    };
    pub(super) const SHIFT: Held = Held {
        shift: true,
        ..Held::NONE
    };
    pub(super) const CTRL_SHIFT: Held = CTRL.and(SHIFT);
    pub(super) const CTRL_ALT: Held = CTRL.and(ALT);
    pub(super) const WIN_SHIFT: Held = WIN.and(SHIFT);
    pub(super) const WIN_CTRL: Held = WIN.and(CTRL);
}

/// The macOS modifier sets the table sends, named as macOS writes them.
mod mac {
    use crate::keys::Mods;

    pub(super) const NONE: Mods = Mods::NONE;
    pub(super) const COMMAND: Mods = Mods::COMMAND;
    pub(super) const CONTROL: Mods = Mods::CONTROL;
    pub(super) const OPTION: Mods = Mods::OPTION;
    pub(super) const COMMAND_SHIFT: Mods = Mods::COMMAND.with(Mods::SHIFT);
    pub(super) const COMMAND_OPTION: Mods = Mods::COMMAND.with(Mods::OPTION);
    pub(super) const CONTROL_COMMAND: Mods = Mods::CONTROL.with(Mods::COMMAND);
    pub(super) const CONTROL_SHIFT: Mods = Mods::CONTROL.with(Mods::SHIFT);
    pub(super) const CONTROL_OPTION: Mods = Mods::CONTROL.with(Mods::OPTION);
    pub(super) const CONTROL_SHIFT_COMMAND: Mods = CONTROL_SHIFT.with(Mods::COMMAND);
}

/// A macOS symbolic hotkey a row triggers. The chord comes from the user's
/// bindings at the time of the key press ([`SystemHotkeys`]), because the
/// user may have rebound or switched off any of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hotkey {
    MissionControl,
    ShowDesktop,
    Spotlight,
    PreviousInputSource,
    NextInputSource,
    SpaceLeft,
    SpaceRight,
    TileFill,
    TileLeft,
    TileRight,
}

impl Hotkey {
    /// The chord to post, or `None` when the user switched the hotkey off,
    /// in which case the Windows shortcut does nothing.
    pub(super) fn chord(self, hotkeys: &SystemHotkeys) -> Option<Chord> {
        match self {
            Hotkey::MissionControl => hotkeys.mission_control,
            Hotkey::ShowDesktop => hotkeys.show_desktop,
            Hotkey::Spotlight => hotkeys.spotlight,
            Hotkey::PreviousInputSource => hotkeys.previous_input_source,
            Hotkey::NextInputSource => hotkeys.next_input_source,
            Hotkey::SpaceLeft => hotkeys.space_left,
            Hotkey::SpaceRight => hotkeys.space_right,
            Hotkey::TileFill => hotkeys.tile_fill,
            Hotkey::TileLeft => hotkeys.tile_left,
            Hotkey::TileRight => hotkeys.tile_right,
        }
    }
}

/// The key a chord output sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum To {
    /// The key that was pressed (only the modifiers change).
    Same,
    Key(KeyCode),
}

/// What a row does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Out {
    /// Nothing: the app's own binding already is the Windows one (VS Code's
    /// F3), or the translation would do harm there (Home in a browser).
    Pass,
    /// Send this chord in place of the key.
    Chord { to: To, mods: Mods },
    /// Post a system hotkey and swallow the key.
    Hotkey(Hotkey),
    /// Run an action off the tap thread and swallow the key.
    Action(Action),
}

/// One row of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Rule {
    pub group: Group,
    pub scope: Scope,
    pub app: AppIs,
    pub keys: Keys,
    /// The Windows modifiers held, exactly.
    pub held: Held,
    /// Shift may also be held, and is then added to the output.
    pub carry_shift: bool,
    pub out: Out,
}

impl Rule {
    /// The same row, also matching with Shift held and passing it on: the
    /// documentation's "(+Shift)".
    const fn shifted(self) -> Rule {
        Rule {
            carry_shift: true,
            ..self
        }
    }

    /// Whether this row applies to `keycode` pressed with exactly `held`
    /// (Shift aside when carried), in `app`, with `groups` switched on.
    fn matches(
        &self,
        keycode: KeyCode,
        held: Held,
        app: &AppProfile,
        groups: &ShortcutGroups,
    ) -> bool {
        let held_matches = if self.carry_shift {
            held.without_shift() == self.held
        } else {
            held == self.held
        };
        held_matches
            && self.keys.contains(keycode)
            && self.scope.includes(app.class)
            && self.app.matches(&app.traits)
            && self.group.is_on(groups)
    }

    /// This row's output for `keycode` pressed with `held`.
    fn decision(&self, keycode: KeyCode, held: Held) -> Decision {
        match self.out {
            Out::Pass => Decision::Pass,
            Out::Chord { to, mods } => {
                let key = match to {
                    To::Same => keycode,
                    To::Key(key) => key,
                };
                let mods = if self.carry_shift && held.shift {
                    mods.with(Mods::SHIFT)
                } else {
                    mods
                };
                Decision::Chord(Chord::new(key, mods))
            }
            Out::Hotkey(hotkey) => Decision::Hotkey(hotkey),
            Out::Action(action) => Decision::Action(action),
        }
    }
}

/// What the table decided for one key-down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Decision {
    /// A row matched and says to leave the key alone.
    Pass,
    /// Send this chord in place of the key.
    Chord(Chord),
    /// Post this hotkey and swallow the key.
    Hotkey(Hotkey),
    /// Run this action and swallow the key.
    Action(Action),
}

/// The first row matching `keycode` with `mods` held while `app` is in
/// front, if any.
pub(super) fn find(
    keycode: KeyCode,
    mods: WinMods,
    app: &AppProfile,
    groups: &ShortcutGroups,
) -> Option<&'static Rule> {
    let held = Held::of(mods);
    RULES
        .iter()
        .find(|rule| rule.matches(keycode, held, app, groups))
}

/// What to do with `keycode` pressed with `mods` (not foreign) while `app`
/// is in front; `None` when no row matches and the key passes.
pub(super) fn decide(
    keycode: KeyCode,
    mods: WinMods,
    app: &AppProfile,
    groups: &ShortcutGroups,
) -> Option<Decision> {
    find(keycode, mods, app, groups).map(|rule| rule.decision(keycode, Held::of(mods)))
}

/// Activity Monitor, Windows' Task Manager.
const ACTIVITY_MONITOR: Action = Action::OpenApp {
    bundle_id: "com.apple.ActivityMonitor",
};
/// System Settings, Windows' Settings.
const SYSTEM_SETTINGS: Action = Action::OpenApp {
    bundle_id: "com.apple.systempreferences",
};

/// The four arrows.
const ARROWS: &[KeyCode] = &[kc::LEFT, kc::RIGHT, kc::UP, kc::DOWN];

// The variants the table names, so that each row reads as one line.
use AppIs::{AnyApp, Browser, Finder, Office, VsCode};
use Group::{
    AppFixes, Editing, Guard, InputSource, Navigation, StartMenu, System, Terminal, Windows,
};
use Hotkey::{
    MissionControl, NextInputSource, PreviousInputSource, ShowDesktop, SpaceLeft, SpaceRight,
    Spotlight, TileFill, TileLeft, TileRight,
};
use Keys::{List, One};
use Scope::{CtrlNative, OnlyStandard, OnlyTerminal, Translated};

/// A row's chord output with a new key.
const fn to(key: KeyCode, mods: Mods) -> Out {
    Out::Chord {
        to: To::Key(key),
        mods,
    }
}

/// A row's chord output on the same key.
const fn same(mods: Mods) -> Out {
    Out::Chord { to: To::Same, mods }
}

/// A row's hotkey output.
const fn hotkey(hotkey: Hotkey) -> Out {
    Out::Hotkey(hotkey)
}

/// A row's action output.
const fn act(action: Action) -> Out {
    Out::Action(action)
}

/// A row's "leave it alone" output.
const PASS: Out = Out::Pass;

/// A plain row: exact modifiers, Shift not carried (see [`Rule::shifted`]).
const fn row(group: Group, scope: Scope, app: AppIs, keys: Keys, held: Held, out: Out) -> Rule {
    Rule {
        group,
        scope,
        app,
        keys,
        held,
        carry_shift: false,
        out,
    }
}

/// Every row, in precedence order (see the module documentation). A static,
/// not a const, so a row has one address that tests can find again. The
/// first match wins.
#[rustfmt::skip]
pub(super) static RULES: &[Rule] = &[
    // App fixes: browsers.
    row(AppFixes, OnlyStandard, Browser, One(kc::F5), win::NONE, to(kc::R, mac::COMMAND)),
    row(AppFixes, OnlyStandard, Browser, One(kc::F5), win::SHIFT, to(kc::R, mac::COMMAND_SHIFT)),
    row(AppFixes, OnlyStandard, Browser, One(kc::F5), win::CTRL, to(kc::R, mac::COMMAND_SHIFT)),
    row(AppFixes, OnlyStandard, Browser, One(kc::LEFT), win::ALT, to(kc::LEFT_BRACKET, mac::COMMAND)),
    row(AppFixes, OnlyStandard, Browser, One(kc::RIGHT), win::ALT, to(kc::RIGHT_BRACKET, mac::COMMAND)),
    row(AppFixes, OnlyStandard, Browser, One(kc::D), win::ALT, to(kc::L, mac::COMMAND)),
    row(AppFixes, OnlyStandard, Browser, One(kc::F6), win::NONE, to(kc::L, mac::COMMAND)),
    row(AppFixes, OnlyStandard, Browser, One(kc::F11), win::NONE, to(kc::F, mac::CONTROL_COMMAND)),
    row(AppFixes, OnlyStandard, Browser, One(kc::F12), win::NONE, to(kc::I, mac::COMMAND_OPTION)),
    row(AppFixes, OnlyStandard, Browser, One(kc::HOME), win::ALT, to(kc::H, mac::COMMAND_SHIFT)),
    // App fixes: the VS Code family.
    row(AppFixes, OnlyStandard, VsCode, List(&[kc::G, kc::R, kc::Q]), win::CTRL, same(mac::CONTROL)),
    row(AppFixes, OnlyStandard, VsCode, One(kc::G), win::CTRL_SHIFT, same(mac::CONTROL_SHIFT)),
    row(AppFixes, OnlyStandard, VsCode, One(kc::UP), win::CTRL, to(kc::PAGE_UP, mac::CONTROL)),
    row(AppFixes, OnlyStandard, VsCode, One(kc::DOWN), win::CTRL, to(kc::PAGE_DOWN, mac::CONTROL)),
    row(AppFixes, OnlyStandard, VsCode, List(&[kc::UP, kc::DOWN]), win::CTRL_ALT, same(mac::COMMAND_OPTION)),
    row(AppFixes, OnlyStandard, VsCode, List(&[kc::LEFT_BRACKET, kc::RIGHT_BRACKET]), win::CTRL_SHIFT, same(mac::COMMAND_OPTION)),
    row(AppFixes, OnlyStandard, VsCode, One(kc::LEFT), win::ALT, to(kc::MINUS, mac::CONTROL)),
    row(AppFixes, OnlyStandard, VsCode, One(kc::RIGHT), win::ALT, to(kc::MINUS, mac::CONTROL_SHIFT)),
    // App fixes: Office.
    row(AppFixes, OnlyStandard, Office, One(kc::F12), win::NONE, to(kc::S, mac::COMMAND_SHIFT)),
    // App fixes: Finder.
    row(AppFixes, OnlyStandard, Finder, One(kc::F2), win::NONE, to(kc::RETURN, mac::NONE)),
    row(AppFixes, OnlyStandard, Finder, One(kc::UP), win::ALT, same(mac::COMMAND)),
    row(AppFixes, OnlyStandard, Finder, One(kc::LEFT), win::ALT, to(kc::LEFT_BRACKET, mac::COMMAND)),
    row(AppFixes, OnlyStandard, Finder, One(kc::RIGHT), win::ALT, to(kc::RIGHT_BRACKET, mac::COMMAND)),
    row(AppFixes, OnlyStandard, Finder, One(kc::RETURN), win::ALT, to(kc::I, mac::COMMAND)),

    // System.
    row(System, Translated, AnyApp, One(kc::ESCAPE), win::CTRL_SHIFT, act(ACTIVITY_MONITOR)),
    row(System, Translated, AnyApp, List(&[kc::FORWARD_DELETE, kc::DELETE]), win::CTRL_ALT, to(kc::ESCAPE, mac::COMMAND_OPTION)),
    row(System, Translated, AnyApp, One(kc::PRINT_SCREEN), win::NONE, to(kc::N4, mac::CONTROL_SHIFT_COMMAND)),
    row(System, Translated, AnyApp, One(kc::S), win::WIN_SHIFT, to(kc::N4, mac::CONTROL_SHIFT_COMMAND)),
    row(System, Translated, AnyApp, One(kc::PRINT_SCREEN), win::ALT, to(kc::N4, mac::CONTROL_SHIFT_COMMAND)),
    row(System, Translated, AnyApp, One(kc::PRINT_SCREEN), win::WIN, to(kc::N3, mac::COMMAND_SHIFT)),

    // Windows. Alt+Tab is the switcher session, decided before the table.
    row(Windows, Translated, Browser, One(kc::F4), win::ALT, to(kc::W, mac::COMMAND_SHIFT)),
    row(Windows, Translated, VsCode, One(kc::F4), win::ALT, to(kc::W, mac::COMMAND_SHIFT)),
    row(Windows, Translated, AnyApp, One(kc::F4), win::ALT, to(kc::W, mac::COMMAND)),
    row(Windows, Translated, AnyApp, One(kc::D), win::WIN, hotkey(ShowDesktop)),
    row(Windows, Translated, AnyApp, One(kc::E), win::WIN, act(Action::OpenHomeInFinder)),
    row(Windows, Translated, AnyApp, One(kc::L), win::WIN, to(kc::Q, mac::CONTROL_COMMAND)),
    row(Windows, Translated, AnyApp, One(kc::TAB), win::WIN, hotkey(MissionControl)),
    row(Windows, Translated, AnyApp, One(kc::UP), win::WIN, hotkey(TileFill)),
    row(Windows, Translated, AnyApp, One(kc::LEFT), win::WIN, hotkey(TileLeft)),
    row(Windows, Translated, AnyApp, One(kc::RIGHT), win::WIN, hotkey(TileRight)),
    row(Windows, Translated, AnyApp, One(kc::DOWN), win::WIN, to(kc::M, mac::COMMAND)),
    row(Windows, Translated, AnyApp, One(kc::M), win::WIN, same(mac::COMMAND_OPTION)),
    row(Windows, Translated, AnyApp, One(kc::LEFT), win::WIN_CTRL, hotkey(SpaceLeft)),
    row(Windows, Translated, AnyApp, One(kc::RIGHT), win::WIN_CTRL, hotkey(SpaceRight)),
    row(Windows, Translated, AnyApp, One(kc::I), win::WIN, act(SYSTEM_SETTINGS)),
    row(Windows, Translated, AnyApp, One(kc::R), win::WIN, hotkey(Spotlight)),
    row(Windows, Translated, AnyApp, List(&[kc::PERIOD, kc::SEMICOLON]), win::WIN, to(kc::SPACE, mac::CONTROL_COMMAND)),

    // Start menu. Tapping Win alone is a detector in the engine, not a row.
    row(StartMenu, Translated, AnyApp, One(kc::ESCAPE), win::CTRL, hotkey(Spotlight)),

    // Input source. Ctrl+Space is a guard (below).
    row(InputSource, Translated, AnyApp, One(kc::SPACE), win::WIN, hotkey(NextInputSource)),

    // Terminal: terminal apps only, where Ctrl arrives as Control.
    row(Terminal, OnlyTerminal, AnyApp, One(kc::C), win::CTRL_SHIFT, same(mac::COMMAND)),
    row(Terminal, OnlyTerminal, AnyApp, One(kc::INSERT), win::CTRL, to(kc::C, mac::COMMAND)),
    row(Terminal, OnlyTerminal, AnyApp, One(kc::V), win::CTRL_SHIFT, same(mac::COMMAND)),
    row(Terminal, OnlyTerminal, AnyApp, One(kc::INSERT), win::SHIFT, to(kc::V, mac::COMMAND)),
    row(Terminal, OnlyTerminal, AnyApp, List(&[kc::T, kc::W, kc::N, kc::F]), win::CTRL_SHIFT, same(mac::COMMAND)),
    row(Terminal, OnlyTerminal, AnyApp, List(&[kc::EQUAL, kc::MINUS, kc::N0]), win::CTRL, same(mac::COMMAND)),
    row(Terminal, OnlyTerminal, AnyApp, List(&[kc::LEFT, kc::RIGHT]), win::CTRL, same(mac::OPTION)),
    row(Terminal, OnlyTerminal, AnyApp, One(kc::DELETE), win::CTRL, to(kc::W, mac::CONTROL)),

    // Editing: standard apps.
    row(Editing, OnlyStandard, Office, One(kc::Y), win::CTRL, same(mac::COMMAND)),
    row(Editing, OnlyStandard, AnyApp, One(kc::Y), win::CTRL, to(kc::Z, mac::COMMAND_SHIFT)),
    row(Editing, OnlyStandard, Browser, One(kc::H), win::CTRL, to(kc::Y, mac::COMMAND)),
    row(Editing, OnlyStandard, Office, One(kc::H), win::CTRL, same(mac::CONTROL)),
    row(Editing, OnlyStandard, AnyApp, One(kc::H), win::CTRL, to(kc::F, mac::COMMAND_OPTION)),
    row(Editing, OnlyStandard, AnyApp, One(kc::F4), win::CTRL, to(kc::W, mac::COMMAND)),
    row(Editing, OnlyStandard, VsCode, One(kc::F3), win::NONE, PASS).shifted(),
    row(Editing, OnlyStandard, Finder, One(kc::F3), win::NONE, to(kc::F, mac::COMMAND)),
    // Finder's Command+Shift+G is Go to Folder: Shift+F3 must not become it.
    row(Editing, OnlyStandard, Finder, One(kc::F3), win::SHIFT, PASS),
    row(Editing, OnlyStandard, AnyApp, One(kc::F3), win::NONE, to(kc::G, mac::COMMAND)).shifted(),
    row(Editing, OnlyStandard, VsCode, One(kc::PAGE_DOWN), win::CTRL, to(kc::RIGHT, mac::COMMAND_OPTION)),
    row(Editing, OnlyStandard, VsCode, One(kc::PAGE_UP), win::CTRL, to(kc::LEFT, mac::COMMAND_OPTION)),
    row(Editing, OnlyStandard, AnyApp, One(kc::PAGE_DOWN), win::CTRL, to(kc::TAB, mac::CONTROL)),
    row(Editing, OnlyStandard, AnyApp, One(kc::PAGE_UP), win::CTRL, to(kc::TAB, mac::CONTROL_SHIFT)),
    row(Editing, OnlyStandard, AnyApp, One(kc::INSERT), win::CTRL, to(kc::C, mac::COMMAND)),
    row(Editing, OnlyStandard, AnyApp, One(kc::INSERT), win::SHIFT, to(kc::V, mac::COMMAND)),

    // Navigation: standard apps. In a browser Command+Left is Back.
    row(Navigation, OnlyStandard, Browser, List(&[kc::HOME, kc::END]), win::NONE, PASS).shifted(),
    row(Navigation, OnlyStandard, AnyApp, One(kc::HOME), win::NONE, to(kc::LEFT, mac::COMMAND)).shifted(),
    row(Navigation, OnlyStandard, AnyApp, One(kc::END), win::NONE, to(kc::RIGHT, mac::COMMAND)).shifted(),

    // Guards in terminals and NativeCtrl apps: Control+arrows are macOS's
    // Spaces and Mission Control hotkeys (with Shift, their slow variants).
    row(Guard, CtrlNative, AnyApp, List(ARROWS), win::CTRL, same(mac::OPTION)).shifted(),

    // Guards in standard apps, where Ctrl arrives as Command.
    row(Guard, OnlyStandard, AnyApp, One(kc::TAB), win::CTRL, same(mac::CONTROL)).shifted(),
    row(Guard, OnlyStandard, AnyApp, One(kc::SPACE), win::CTRL, hotkey(PreviousInputSource)),
    row(Guard, OnlyStandard, AnyApp, One(kc::ESCAPE), win::CTRL, same(mac::CONTROL)),
    row(Guard, OnlyStandard, AnyApp, One(kc::H), win::CTRL, same(mac::CONTROL)),
    row(Guard, OnlyStandard, AnyApp, List(&[kc::M, kc::Q, kc::GRAVE]), win::CTRL, same(mac::CONTROL)),
    row(Guard, OnlyStandard, AnyApp, List(&[kc::Q, kc::GRAVE]), win::CTRL_SHIFT, same(mac::CONTROL_SHIFT)),
    row(Guard, OnlyStandard, AnyApp, Keys::Any, win::CTRL_ALT, same(mac::CONTROL_OPTION)).shifted(),
    row(Guard, OnlyStandard, AnyApp, Keys::FunctionRow, win::CTRL, same(mac::CONTROL)),
    row(Guard, OnlyStandard, AnyApp, One(kc::HOME), win::CTRL, to(kc::UP, mac::COMMAND)).shifted(),
    row(Guard, OnlyStandard, AnyApp, One(kc::END), win::CTRL, to(kc::DOWN, mac::COMMAND)).shifted(),
    row(Guard, OnlyStandard, AnyApp, List(&[kc::LEFT, kc::RIGHT]), win::CTRL, same(mac::OPTION)).shifted(),
    row(Guard, OnlyStandard, AnyApp, List(&[kc::UP, kc::DOWN]), win::CTRL, same(mac::OPTION)).shifted(),
    row(Guard, OnlyStandard, AnyApp, List(&[kc::DELETE, kc::FORWARD_DELETE]), win::CTRL, same(mac::OPTION)),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// The app in front for a case.
    #[derive(Clone, Copy, Debug)]
    enum In {
        Standard,
        Browser,
        VsCode,
        Office,
        Finder,
        Terminal,
        NativeCtrl,
    }

    impl In {
        fn profile(self) -> AppProfile {
            let mut traits = AppTraits::default();
            let class = match self {
                In::Standard => AppClass::Standard,
                In::Browser => {
                    traits.browser = true;
                    AppClass::Standard
                }
                In::VsCode => {
                    traits.vscode = true;
                    AppClass::Standard
                }
                In::Office => {
                    traits.office = true;
                    AppClass::Standard
                }
                In::Finder => {
                    traits.finder = true;
                    AppClass::Standard
                }
                In::Terminal => AppClass::Terminal,
                In::NativeCtrl => AppClass::NativeCtrl,
            };
            AppProfile { class, traits }
        }
    }

    /// What a case expects.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Want {
        /// No row matches.
        Nothing,
        Pass,
        Chord(KeyCode, Mods),
        Hotkey(Hotkey),
        Action(Action),
    }

    fn held(ctrl: bool, win: bool, alt: bool, shift: bool) -> WinMods {
        WinMods {
            ctrl,
            win,
            alt,
            shift,
            foreign: false,
        }
    }

    const NO: WinMods = WinMods {
        ctrl: false,
        win: false,
        alt: false,
        shift: false,
        foreign: false,
    };

    fn c() -> WinMods {
        held(true, false, false, false)
    }
    fn cs() -> WinMods {
        held(true, false, false, true)
    }
    fn ca() -> WinMods {
        held(true, false, true, false)
    }
    fn cas() -> WinMods {
        held(true, false, true, true)
    }
    fn w() -> WinMods {
        held(false, true, false, false)
    }
    fn ws() -> WinMods {
        held(false, true, false, true)
    }
    fn wc() -> WinMods {
        held(true, true, false, false)
    }
    fn a() -> WinMods {
        held(false, false, true, false)
    }
    fn s() -> WinMods {
        held(false, false, false, true)
    }

    const CMD: Mods = Mods::COMMAND;
    const CTL: Mods = Mods::CONTROL;
    const OPT: Mods = Mods::OPTION;
    const SH: Mods = Mods::SHIFT;

    /// One case per documented row (and per app variant), all groups on.
    #[rustfmt::skip]
    fn cases() -> Vec<(&'static str, In, KeyCode, WinMods, Want)> {
        use Want::{Action as Act, Chord as Ch, Hotkey as Hk, Nothing, Pass};
        vec![
            // Guards.
            ("ctrl+tab", In::Standard, kc::TAB, c(), Ch(kc::TAB, CTL)),
            ("ctrl+shift+tab", In::Standard, kc::TAB, cs(), Ch(kc::TAB, CTL.with(SH))),
            ("ctrl+space", In::Standard, kc::SPACE, c(), Hk(Hotkey::PreviousInputSource)),
            ("ctrl+esc start menu", In::Standard, kc::ESCAPE, c(), Hk(Hotkey::Spotlight)),
            ("ctrl+h", In::Standard, kc::H, c(), Ch(kc::F, CMD.with(OPT))),
            ("ctrl+m", In::Standard, kc::M, c(), Ch(kc::M, CTL)),
            ("ctrl+q", In::Standard, kc::Q, c(), Ch(kc::Q, CTL)),
            ("ctrl+shift+q", In::Standard, kc::Q, cs(), Ch(kc::Q, CTL.with(SH))),
            ("ctrl+grave", In::Standard, kc::GRAVE, c(), Ch(kc::GRAVE, CTL)),
            ("ctrl+shift+grave", In::Standard, kc::GRAVE, cs(), Ch(kc::GRAVE, CTL.with(SH))),
            ("ctrl+alt+d", In::Standard, kc::D, ca(), Ch(kc::D, CTL.with(OPT))),
            ("ctrl+alt+shift+2", In::Standard, kc::N2, cas(), Ch(kc::N2, CTL.with(OPT).with(SH))),
            ("ctrl+f5", In::Standard, kc::F5, c(), Ch(kc::F5, CTL)),
            ("ctrl+f20", In::Standard, kc::F20, c(), Ch(kc::F20, CTL)),
            ("ctrl+home", In::Standard, kc::HOME, c(), Ch(kc::UP, CMD)),
            ("ctrl+shift+end", In::Standard, kc::END, cs(), Ch(kc::DOWN, CMD.with(SH))),
            ("ctrl+left", In::Standard, kc::LEFT, c(), Ch(kc::LEFT, OPT)),
            ("ctrl+shift+right", In::Standard, kc::RIGHT, cs(), Ch(kc::RIGHT, OPT.with(SH))),
            ("ctrl+up", In::Standard, kc::UP, c(), Ch(kc::UP, OPT)),
            ("ctrl+shift+down", In::Standard, kc::DOWN, cs(), Ch(kc::DOWN, OPT.with(SH))),
            ("ctrl+backspace", In::Standard, kc::DELETE, c(), Ch(kc::DELETE, OPT)),
            ("ctrl+delete", In::Standard, kc::FORWARD_DELETE, c(), Ch(kc::FORWARD_DELETE, OPT)),
            // Every other Ctrl chord stays Command: no row.
            ("ctrl+c", In::Standard, kc::C, c(), Nothing),
            ("ctrl+shift+t", In::Standard, kc::T, cs(), Nothing),
            ("ctrl+return", In::Standard, kc::RETURN, c(), Nothing),
            ("ctrl+shift+f5 stays command", In::Standard, kc::F5, cs(), Nothing),
            ("plain letter", In::Standard, kc::A, NO, Nothing),
            // Editing.
            ("ctrl+y", In::Standard, kc::Y, c(), Ch(kc::Z, CMD.with(SH))),
            ("ctrl+y office", In::Office, kc::Y, c(), Ch(kc::Y, CMD)),
            ("ctrl+h browser", In::Browser, kc::H, c(), Ch(kc::Y, CMD)),
            ("ctrl+h office", In::Office, kc::H, c(), Ch(kc::H, CTL)),
            ("ctrl+f4", In::Standard, kc::F4, c(), Ch(kc::W, CMD)),
            ("f3", In::Standard, kc::F3, NO, Ch(kc::G, CMD)),
            ("shift+f3", In::Standard, kc::F3, s(), Ch(kc::G, CMD.with(SH))),
            ("f3 vscode", In::VsCode, kc::F3, NO, Pass),
            ("shift+f3 vscode", In::VsCode, kc::F3, s(), Pass),
            ("f3 finder", In::Finder, kc::F3, NO, Ch(kc::F, CMD)),
            ("shift+f3 finder", In::Finder, kc::F3, s(), Pass),
            ("ctrl+pagedown", In::Standard, kc::PAGE_DOWN, c(), Ch(kc::TAB, CTL)),
            ("ctrl+pageup", In::Standard, kc::PAGE_UP, c(), Ch(kc::TAB, CTL.with(SH))),
            ("ctrl+pagedown vscode", In::VsCode, kc::PAGE_DOWN, c(), Ch(kc::RIGHT, CMD.with(OPT))),
            ("ctrl+pageup vscode", In::VsCode, kc::PAGE_UP, c(), Ch(kc::LEFT, CMD.with(OPT))),
            ("ctrl+insert", In::Standard, kc::INSERT, c(), Ch(kc::C, CMD)),
            ("shift+insert", In::Standard, kc::INSERT, s(), Ch(kc::V, CMD)),
            // Navigation.
            ("home", In::Standard, kc::HOME, NO, Ch(kc::LEFT, CMD)),
            ("shift+home", In::Standard, kc::HOME, s(), Ch(kc::LEFT, CMD.with(SH))),
            ("end", In::Standard, kc::END, NO, Ch(kc::RIGHT, CMD)),
            ("shift+end", In::Standard, kc::END, s(), Ch(kc::RIGHT, CMD.with(SH))),
            ("home browser", In::Browser, kc::HOME, NO, Pass),
            ("shift+end browser", In::Browser, kc::END, s(), Pass),
            // Windows.
            ("alt+f4", In::Standard, kc::F4, a(), Ch(kc::W, CMD)),
            ("alt+f4 browser", In::Browser, kc::F4, a(), Ch(kc::W, CMD.with(SH))),
            ("alt+f4 vscode", In::VsCode, kc::F4, a(), Ch(kc::W, CMD.with(SH))),
            ("alt+f4 terminal", In::Terminal, kc::F4, a(), Ch(kc::W, CMD)),
            ("win+d", In::Standard, kc::D, w(), Hk(Hotkey::ShowDesktop)),
            ("win+e", In::Standard, kc::E, w(), Act(Action::OpenHomeInFinder)),
            ("win+l", In::Standard, kc::L, w(), Ch(kc::Q, CTL.with(CMD))),
            ("win+tab", In::Standard, kc::TAB, w(), Hk(Hotkey::MissionControl)),
            ("win+up", In::Standard, kc::UP, w(), Hk(Hotkey::TileFill)),
            ("win+left", In::Standard, kc::LEFT, w(), Hk(Hotkey::TileLeft)),
            ("win+right", In::Standard, kc::RIGHT, w(), Hk(Hotkey::TileRight)),
            ("win+down", In::Standard, kc::DOWN, w(), Ch(kc::M, CMD)),
            ("win+m", In::Standard, kc::M, w(), Ch(kc::M, CMD.with(OPT))),
            ("win+ctrl+left", In::Standard, kc::LEFT, wc(), Hk(Hotkey::SpaceLeft)),
            ("win+ctrl+right", In::NativeCtrl, kc::RIGHT, wc(), Hk(Hotkey::SpaceRight)),
            ("win+i", In::Standard, kc::I, w(), Act(SYSTEM_SETTINGS)),
            ("win+r", In::Standard, kc::R, w(), Hk(Hotkey::Spotlight)),
            ("win+period", In::Standard, kc::PERIOD, w(), Ch(kc::SPACE, CTL.with(CMD))),
            ("win+semicolon", In::Terminal, kc::SEMICOLON, w(), Ch(kc::SPACE, CTL.with(CMD))),
            // System.
            ("ctrl+shift+esc", In::Standard, kc::ESCAPE, cs(), Act(ACTIVITY_MONITOR)),
            ("ctrl+alt+delete", In::Standard, kc::FORWARD_DELETE, ca(), Ch(kc::ESCAPE, CMD.with(OPT))),
            ("ctrl+alt+backspace", In::Terminal, kc::DELETE, ca(), Ch(kc::ESCAPE, CMD.with(OPT))),
            ("printscreen", In::Standard, kc::PRINT_SCREEN, NO, Ch(kc::N4, CTL.with(SH).with(CMD))),
            ("win+shift+s", In::Standard, kc::S, ws(), Ch(kc::N4, CTL.with(SH).with(CMD))),
            ("alt+printscreen", In::Standard, kc::PRINT_SCREEN, a(), Ch(kc::N4, CTL.with(SH).with(CMD))),
            ("win+printscreen", In::Standard, kc::PRINT_SCREEN, w(), Ch(kc::N3, CMD.with(SH))),
            // Input source.
            ("win+space", In::Standard, kc::SPACE, w(), Hk(Hotkey::NextInputSource)),
            // App fixes: browsers.
            ("f5 browser", In::Browser, kc::F5, NO, Ch(kc::R, CMD)),
            ("shift+f5 browser", In::Browser, kc::F5, s(), Ch(kc::R, CMD.with(SH))),
            ("ctrl+f5 browser", In::Browser, kc::F5, c(), Ch(kc::R, CMD.with(SH))),
            ("alt+left browser", In::Browser, kc::LEFT, a(), Ch(kc::LEFT_BRACKET, CMD)),
            ("alt+right browser", In::Browser, kc::RIGHT, a(), Ch(kc::RIGHT_BRACKET, CMD)),
            ("alt+d browser", In::Browser, kc::D, a(), Ch(kc::L, CMD)),
            ("f6 browser", In::Browser, kc::F6, NO, Ch(kc::L, CMD)),
            ("f11 browser", In::Browser, kc::F11, NO, Ch(kc::F, CTL.with(CMD))),
            ("f12 browser", In::Browser, kc::F12, NO, Ch(kc::I, CMD.with(OPT))),
            ("alt+home browser", In::Browser, kc::HOME, a(), Ch(kc::H, CMD.with(SH))),
            // App fixes: VS Code.
            ("ctrl+g vscode", In::VsCode, kc::G, c(), Ch(kc::G, CTL)),
            ("ctrl+r vscode", In::VsCode, kc::R, c(), Ch(kc::R, CTL)),
            ("ctrl+q vscode", In::VsCode, kc::Q, c(), Ch(kc::Q, CTL)),
            ("ctrl+shift+g vscode", In::VsCode, kc::G, cs(), Ch(kc::G, CTL.with(SH))),
            ("ctrl+up vscode", In::VsCode, kc::UP, c(), Ch(kc::PAGE_UP, CTL)),
            ("ctrl+down vscode", In::VsCode, kc::DOWN, c(), Ch(kc::PAGE_DOWN, CTL)),
            ("ctrl+alt+up vscode", In::VsCode, kc::UP, ca(), Ch(kc::UP, CMD.with(OPT))),
            ("ctrl+shift+[ vscode", In::VsCode, kc::LEFT_BRACKET, cs(), Ch(kc::LEFT_BRACKET, CMD.with(OPT))),
            ("alt+left vscode", In::VsCode, kc::LEFT, a(), Ch(kc::MINUS, CTL)),
            ("alt+right vscode", In::VsCode, kc::RIGHT, a(), Ch(kc::MINUS, CTL.with(SH))),
            // App fixes: Office and Finder.
            ("f12 office", In::Office, kc::F12, NO, Ch(kc::S, CMD.with(SH))),
            ("f2 finder", In::Finder, kc::F2, NO, Ch(kc::RETURN, Mods::NONE)),
            ("alt+up finder", In::Finder, kc::UP, a(), Ch(kc::UP, CMD)),
            ("alt+left finder", In::Finder, kc::LEFT, a(), Ch(kc::LEFT_BRACKET, CMD)),
            ("alt+right finder", In::Finder, kc::RIGHT, a(), Ch(kc::RIGHT_BRACKET, CMD)),
            ("alt+return finder", In::Finder, kc::RETURN, a(), Ch(kc::I, CMD)),
            // Terminal.
            ("ctrl+shift+c terminal", In::Terminal, kc::C, cs(), Ch(kc::C, CMD)),
            ("ctrl+insert terminal", In::Terminal, kc::INSERT, c(), Ch(kc::C, CMD)),
            ("ctrl+shift+v terminal", In::Terminal, kc::V, cs(), Ch(kc::V, CMD)),
            ("shift+insert terminal", In::Terminal, kc::INSERT, s(), Ch(kc::V, CMD)),
            ("ctrl+shift+t terminal", In::Terminal, kc::T, cs(), Ch(kc::T, CMD)),
            ("ctrl+shift+f terminal", In::Terminal, kc::F, cs(), Ch(kc::F, CMD)),
            ("ctrl+= terminal", In::Terminal, kc::EQUAL, c(), Ch(kc::EQUAL, CMD)),
            ("ctrl+0 terminal", In::Terminal, kc::N0, c(), Ch(kc::N0, CMD)),
            ("ctrl+left terminal", In::Terminal, kc::LEFT, c(), Ch(kc::LEFT, OPT)),
            ("ctrl+backspace terminal", In::Terminal, kc::DELETE, c(), Ch(kc::W, CTL)),
            ("ctrl+c terminal interrupts", In::Terminal, kc::C, c(), Nothing),
            // Terminal and NativeCtrl guards.
            ("ctrl+up terminal", In::Terminal, kc::UP, c(), Ch(kc::UP, OPT)),
            ("ctrl+shift+left jetbrains", In::NativeCtrl, kc::LEFT, cs(), Ch(kc::LEFT, OPT.with(SH))),
            ("ctrl+c jetbrains", In::NativeCtrl, kc::C, c(), Nothing),
            ("ctrl+shift+c jetbrains", In::NativeCtrl, kc::C, cs(), Nothing),
            ("home terminal", In::Terminal, kc::HOME, NO, Nothing),
            ("ctrl+tab terminal", In::Terminal, kc::TAB, c(), Nothing),
        ]
    }

    /// Cases for a group switched off, each naming that group: its rows
    /// are skipped, and the guard that stands in for it, if any, decides.
    #[rustfmt::skip]
    fn fallback_cases() -> Vec<(&'static str, Group, In, KeyCode, WinMods, Want)> {
        use Want::{Chord as Ch, Nothing};
        vec![
            ("ctrl+h", Group::Editing, In::Standard, kc::H, c(), Ch(kc::H, CTL)),
            ("ctrl+h browser", Group::Editing, In::Browser, kc::H, c(), Ch(kc::H, CTL)),
            ("ctrl+f4", Group::Editing, In::Standard, kc::F4, c(), Ch(kc::F4, CTL)),
            ("ctrl+y", Group::Editing, In::Standard, kc::Y, c(), Nothing),
            ("f3", Group::Editing, In::Standard, kc::F3, NO, Nothing),
            ("ctrl+esc", Group::StartMenu, In::Standard, kc::ESCAPE, c(), Ch(kc::ESCAPE, CTL)),
            ("ctrl+esc terminal", Group::StartMenu, In::Terminal, kc::ESCAPE, c(), Nothing),
            ("home", Group::Navigation, In::Standard, kc::HOME, NO, Nothing),
            ("ctrl+home", Group::Navigation, In::Standard, kc::HOME, c(), Ch(kc::UP, CMD)),
            ("ctrl+up vscode", Group::AppFixes, In::VsCode, kc::UP, c(), Ch(kc::UP, OPT)),
            ("ctrl+alt+up vscode", Group::AppFixes, In::VsCode, kc::UP, ca(), Ch(kc::UP, CTL.with(OPT))),
            ("ctrl+f5 browser", Group::AppFixes, In::Browser, kc::F5, c(), Ch(kc::F5, CTL)),
            ("ctrl+alt+delete", Group::System, In::Standard, kc::FORWARD_DELETE, ca(), Ch(kc::FORWARD_DELETE, CTL.with(OPT))),
            ("ctrl+alt+delete terminal", Group::System, In::Terminal, kc::FORWARD_DELETE, ca(), Nothing),
            ("win+d", Group::Windows, In::Standard, kc::D, w(), Nothing),
            ("win+space", Group::InputSource, In::Standard, kc::SPACE, w(), Nothing),
            ("ctrl+left terminal", Group::Terminal, In::Terminal, kc::LEFT, c(), Ch(kc::LEFT, OPT)),
            ("ctrl+backspace terminal", Group::Terminal, In::Terminal, kc::DELETE, c(), Nothing),
            ("ctrl+tab guard", Group::Guard, In::Standard, kc::TAB, c(), Ch(kc::TAB, CTL)),
        ]
    }

    /// Every group on but `group`.
    fn groups_without(group: Group) -> ShortcutGroups {
        let mut groups = ShortcutGroups::all(true);
        match group {
            Group::Guard => {}
            Group::Editing => groups.editing = false,
            Group::Navigation => groups.navigation = false,
            Group::Windows => groups.windows = false,
            Group::StartMenu => groups.start_menu = false,
            Group::System => groups.system = false,
            Group::InputSource => groups.input_source = false,
            Group::AppFixes => groups.app_fixes = false,
            Group::Terminal => groups.terminal = false,
        }
        groups
    }

    fn want_of(decision: Option<Decision>) -> Want {
        match decision {
            None => Want::Nothing,
            Some(Decision::Pass) => Want::Pass,
            Some(Decision::Chord(chord)) => {
                assert!(!chord.fn_key, "table chords never ask for Fn: {chord:?}");
                Want::Chord(chord.keycode, chord.mods)
            }
            Some(Decision::Hotkey(hotkey)) => Want::Hotkey(hotkey),
            Some(Decision::Action(action)) => Want::Action(action),
        }
    }

    /// Every documented row gives its documented output, with every group
    /// on, including each app's variant of a row.
    #[test]
    fn every_documented_row_gives_its_documented_output() {
        let groups = ShortcutGroups::all(true);
        for (name, app, keycode, mods, want) in cases() {
            let got = want_of(decide(keycode, mods, &app.profile(), &groups));
            assert_eq!(got, want, "{name}");
        }
    }

    /// The cases reach every row of the table, so no row is untested and
    /// no row is shadowed by an earlier one.
    #[test]
    fn every_row_of_the_table_is_reached_by_a_case() {
        let all_on = cases()
            .into_iter()
            .map(|(_, app, keycode, mods, _)| (Group::Guard, app, keycode, mods));
        let one_off = fallback_cases()
            .into_iter()
            .map(|(_, off, app, keycode, mods, _)| (off, app, keycode, mods));
        let mut reached = BTreeSet::new();
        for (off, app, keycode, mods) in all_on.chain(one_off) {
            let groups = groups_without(off);
            if let Some(rule) = find(keycode, mods, &app.profile(), &groups) {
                let index = RULES
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate, rule))
                    .unwrap();
                reached.insert(index);
            }
        }
        let missing: Vec<_> = (0..RULES.len())
            .filter(|index| !reached.contains(index))
            .map(|index| RULES[index])
            .collect();
        assert!(missing.is_empty(), "rows without a case: {missing:#?}");
    }

    /// A carried Shift is only ever added, so a row that carries it must
    /// not also require it.
    #[test]
    fn rows_that_carry_shift_do_not_also_require_it() {
        for rule in RULES {
            if rule.carry_shift {
                assert!(!rule.held.shift, "{rule:?}");
            }
        }
    }

    /// Passthrough apps are never translated by any row, whatever is held.
    #[test]
    fn passthrough_apps_match_no_row() {
        let groups = ShortcutGroups::all(true);
        let app = AppProfile {
            class: AppClass::Passthrough,
            traits: AppTraits::default(),
        };
        for (_, _, keycode, mods, _) in cases() {
            assert_eq!(decide(keycode, mods, &app, &groups), None);
        }
    }

    /// With a group off its rows are skipped and the guard that stands in
    /// for it, if any, decides: Ctrl+H stays off Command+H, Ctrl+Esc stays
    /// off Command+Esc, Ctrl+F4 stays a Control chord.
    #[test]
    fn a_group_switched_off_falls_back_to_its_guard() {
        for (name, off, app, keycode, mods, want) in fallback_cases() {
            let groups = groups_without(off);
            let got = want_of(decide(keycode, mods, &app.profile(), &groups));
            assert_eq!(got, want, "{name} without {off:?}");
        }
    }

    /// Each group switch removes exactly its own rows; the guards stay.
    #[test]
    fn every_group_switch_gates_its_rows() {
        for (name, app, keycode, mods, _) in cases() {
            let all = ShortcutGroups::all(true);
            let Some(rule) = find(keycode, mods, &app.profile(), &all) else {
                continue;
            };
            let groups = groups_without(rule.group);
            let after = find(keycode, mods, &app.profile(), &groups);
            if rule.group == Group::Guard {
                assert_eq!(after, Some(rule), "{name}");
            } else {
                assert!(
                    after.is_none_or(|other| other.group != rule.group),
                    "{name}: {after:?}"
                );
            }
        }
    }

    /// Hotkeys resolve to the user's bindings; one switched off is `None`.
    #[test]
    fn hotkeys_resolve_to_the_users_bindings() {
        let defaults = SystemHotkeys::default();
        assert_eq!(Hotkey::Spotlight.chord(&defaults), defaults.spotlight);
        assert_eq!(Hotkey::TileLeft.chord(&defaults), defaults.tile_left);
        let off = SystemHotkeys {
            show_desktop: None,
            ..defaults
        };
        assert_eq!(Hotkey::ShowDesktop.chord(&off), None);
    }
}
