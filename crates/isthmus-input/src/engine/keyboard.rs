//! The keyboard engine: one decision per key event, on the tap thread.
//!
//! Three pieces of state make the decisions more than a table lookup:
//!
//! - **The pairing table.** A key's translation is decided once, on its
//!   key-down, and replayed for its autorepeats and its key-up, whatever the
//!   modifiers do meanwhile. Releasing Ctrl before the arrow of a Ctrl+Left
//!   must not let the arrow's key-up arrive as a plain Left, and a swallowed
//!   Win+D must swallow its key-up too. One slot per keycode, no allocation.
//! - **The switcher session.** macOS's app switcher stays open only while
//!   Command is held, so Alt+Tab holds a synthetic Command down from the
//!   first Tab until the physical Alt goes up, and sends the switcher's
//!   own keys with Command meanwhile; a key already held under a
//!   translation keeps it, so the other key it went down as comes up.
//!   The physical Alt release passes through and is followed by the
//!   Command release; Isthmus never posts an Option release of its own, so
//!   the physical Option state the watchdog reads stays the truth.
//! - **The Win tap.** Win pressed and released alone, quickly, opens
//!   Spotlight like the Start menu. Any key, other modifier, click or wheel
//!   in between cancels it.
//!
//! The pass path (no translation) returns [`KeyOutcome::PASS`] and allocates
//! nothing; only an event that posts something allocates its list.

use super::rules::{self, Decision};
use super::{Chord, Fate, KeyContext, KeyEvent, KeyOutcome, KeyPhase, Synth};
use crate::context::AppClass;
use crate::hidmap::{self, HidProfile, WinKey};
use crate::keys::{Flags, KeyCode, kc};

/// How long Win may be held and still count as a tap of the Start key: a
/// longer press is a chord the user changed their mind about.
pub const WIN_TAP_WINDOW_NS: u64 = 300_000_000;

/// Keycodes below this have a pairing slot: every `kVK_*` code does.
const PAIRED_KEYS: usize = 128;

/// The flag bits that describe a key rather than the modifier state (an
/// arrow's Fn and NumericPad, Insert's Help). A translation to another key
/// drops them and takes the new key's own from [`Chord::flags`].
const KEY_BITS: Flags = Flags::FN.union(Flags::NUMERIC_PAD).union(Flags::HELP);

/// Option and both its side bits: what the switcher takes off its keys.
const OPTION_BITS: Flags = Flags::OPTION.union(Flags::DEV_LALT).union(Flags::DEV_RALT);

/// The synthetic Command the switcher holds: left-hand, as
/// [`crate::keys::Mods::to_flags`] writes it.
const SYNTHETIC_COMMAND: Flags = Flags::COMMAND.union(Flags::DEV_LCMD);

/// What a key-down decided, replayed for that key's autorepeats and key-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Paired {
    /// No key-down seen, or its key-up already came.
    Untracked,
    Pass,
    /// Sent as this chord.
    Rewrite(Chord),
    /// Swallowed (a hotkey or action trigger).
    Drop,
    /// Sent with Command by the switcher session while it lasts, unchanged
    /// after it.
    Switcher,
}

/// The open Alt+Tab session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Switcher {
    /// The side bit of the Alt key that opened it (`DEV_LALT` or
    /// `DEV_RALT`), or none when the event carried no side bit.
    alt_side: Flags,
    /// The keycode that Alt key arrives as, for the watchdog's physical
    /// check.
    alt_keycode: KeyCode,
}

impl Switcher {
    /// Whether a `flagsChanged` event with these flags shows the session's
    /// Alt key released: Option gone, or the session key's own side bit
    /// gone while another Option key is still held.
    fn released_by(self, flags: Flags) -> bool {
        !flags.contains(Flags::OPTION)
            || (self.alt_side != Flags::NONE && !flags.intersects(self.alt_side))
    }
}

/// Keyboard state that outlives one event: which keys were translated on
/// the way down (so their autorepeats and key-up are translated the same,
/// whatever the modifiers do meanwhile), the Alt+Tab switcher session, the
/// pending Win tap, and the synthetic modifiers Isthmus holds down.
#[derive(Debug)]
pub struct KeyboardEngine {
    paired: [Paired; PAIRED_KEYS],
    switcher: Option<Switcher>,
    /// When Win went down alone, while it may still be a tap.
    win_tap_since: Option<u64>,
}

impl KeyboardEngine {
    pub fn new() -> Self {
        KeyboardEngine {
            paired: [Paired::Untracked; PAIRED_KEYS],
            switcher: None,
            win_tap_since: None,
        }
    }

    /// Decides one key event.
    pub fn handle(&mut self, event: &KeyEvent, cx: &KeyContext<'_>) -> KeyOutcome {
        match event.phase {
            KeyPhase::FlagsChanged => self.flags_changed(event, cx),
            KeyPhase::Down => {
                // A key pressed while Win is down makes it a Win chord.
                self.win_tap_since = None;
                self.key(event, cx)
            }
            KeyPhase::Up => self.key(event, cx),
        }
    }

    /// A mouse button went down or the wheel turned: a Win press is then no
    /// longer a tap.
    pub fn note_pointer_activity(&mut self) {
        self.win_tap_since = None;
    }

    /// Called every ~150 ms while [`Self::needs_ticks`]. `alt_down` is
    /// whether the physical Alt key is still down (the key
    /// [`Self::switcher_alt_keycode`] names); when it is not, a switcher
    /// session whose release was missed (Secure Input, a disabled tap)
    /// ends, and the synthetic Command is released.
    pub fn tick(&mut self, now_ns: u64, alt_down: bool) -> Vec<Synth> {
        // The decision needs only the physical key: the session has no
        // deadline of its own, since holding Alt with the switcher open is
        // legitimate for as long as the user likes.
        let _ = now_ns;
        if alt_down || self.switcher.take().is_none() {
            return Vec::new();
        }
        vec![command_up(Flags::NON_COALESCED)]
    }

    /// Whether a session is open that [`Self::tick`] must watch.
    pub fn needs_ticks(&self) -> bool {
        self.switcher.is_some()
    }

    /// The keycode of the physical Alt key holding the switcher open, for
    /// the watchdog to read (`CGEventSourceKeyState`). Under NativeCtrl
    /// either Option key is Alt, so the profile alone cannot name it.
    pub fn switcher_alt_keycode(&self) -> Option<KeyCode> {
        self.switcher.map(|switcher| switcher.alt_keycode)
    }

    /// Ends every session and forgets every pairing, returning the events
    /// that release whatever Isthmus still holds down. Called when the tap
    /// is disabled, re-enabled, torn down, or Secure Input toggles.
    ///
    /// Besides the switcher's Command, that is the key-up of every key
    /// held under a translation to another key (Home sent as Left): its
    /// physical key-up may never reach the tap, and an app that tracks keys
    /// would otherwise see the other key stuck down.
    pub fn release_all(&mut self) -> Vec<Synth> {
        let mut released = Vec::new();
        for (code, paired) in self.paired.iter_mut().enumerate() {
            if let Paired::Rewrite(chord) = *paired
                && usize::from(chord.keycode) != code
            {
                released.push(Synth::Key {
                    keycode: chord.keycode,
                    // Without modifiers, as every synthetic key-up (see
                    // `Chord::stroke`).
                    flags: chord.flags().non_modifier_bits(),
                    down: false,
                });
            }
            *paired = Paired::Untracked;
        }
        if self.switcher.take().is_some() {
            released.push(command_up(Flags::NON_COALESCED));
        }
        self.win_tap_since = None;
        released
    }

    /// A key-down, autorepeat or key-up.
    fn key(&mut self, event: &KeyEvent, cx: &KeyContext<'_>) -> KeyOutcome {
        let down = event.phase == KeyPhase::Down;
        if self.switcher.is_some()
            && is_switcher_key(event.keycode)
            && !self.held_under_translation(event)
        {
            let paired = if down {
                Paired::Switcher
            } else {
                Paired::Untracked
            };
            self.pair(event.keycode, paired);
            return rewrite(event.keycode, switcher_flags(event.flags));
        }
        if !down || event.autorepeat {
            let paired = self.paired_of(event.keycode);
            if !down {
                self.pair(event.keycode, Paired::Untracked);
            }
            match paired {
                Paired::Pass | Paired::Switcher => return KeyOutcome::PASS,
                Paired::Rewrite(chord) if !down && chord.keycode != event.keycode => {
                    // The key-up of a key sent as another key becomes a new
                    // event posted from the HID state source: without
                    // modifiers, so it does not leave them held (see
                    // `Chord::stroke`). An in-place rewrite keeps its flags.
                    return rewrite(chord.keycode, chord_flags(event, chord).non_modifier_bits());
                }
                Paired::Rewrite(chord) => return rewritten(event, chord),
                Paired::Drop => return dropped(),
                // A key-up whose key-down was never seen passes; an
                // autorepeat without one (after `release_all`, say) is
                // decided as if it were the key-down.
                Paired::Untracked if !down => return KeyOutcome::PASS,
                Paired::Untracked => {}
            }
        }
        let (paired, outcome) = self.decide(event, cx);
        self.pair(event.keycode, paired);
        outcome
    }

    /// Decides a key-down afresh.
    fn decide(&mut self, event: &KeyEvent, cx: &KeyContext<'_>) -> (Paired, KeyOutcome) {
        if cx.app.class == AppClass::Passthrough {
            return (Paired::Pass, KeyOutcome::PASS);
        }
        let mods = hidmap::decode(event.flags, cx.hid_profile);
        if mods.foreign {
            return (Paired::Pass, KeyOutcome::PASS);
        }
        let first = !event.autorepeat;
        if first
            && event.keycode == kc::TAB
            && mods.alt
            && !mods.ctrl
            && !mods.win
            && cx.groups.windows
            && self.switcher.is_none()
        {
            return (Paired::Switcher, self.open_switcher(event, cx.hid_profile));
        }
        match rules::decide(event.keycode, mods, &cx.app, cx.groups) {
            None | Some(Decision::Pass) => (Paired::Pass, KeyOutcome::PASS),
            Some(Decision::Chord(chord)) => (Paired::Rewrite(chord), rewritten(event, chord)),
            Some(Decision::Hotkey(hotkey)) => {
                let mut outcome = dropped();
                if first && let Some(chord) = hotkey.chord(cx.hotkeys) {
                    outcome.before = chord.stroke().to_vec();
                }
                (Paired::Drop, outcome)
            }
            Some(Decision::Action(action)) => {
                let mut outcome = dropped();
                if first {
                    outcome.action = Some(action);
                }
                (Paired::Drop, outcome)
            }
        }
    }

    /// The first Tab with Alt held: hold Command down and send this Tab as
    /// Command+Tab, which opens the switcher.
    fn open_switcher(&mut self, event: &KeyEvent, profile: HidProfile) -> KeyOutcome {
        let alt_side = Flags(event.flags.0 & (Flags::DEV_LALT.0 | Flags::DEV_RALT.0));
        let alt_keycode = if alt_side == Flags::DEV_RALT {
            kc::RIGHT_OPTION
        } else if alt_side == Flags::DEV_LALT {
            kc::OPTION
        } else {
            hidmap::arrival_keycode(WinKey::Alt, profile).unwrap_or(kc::OPTION)
        };
        self.switcher = Some(Switcher {
            alt_side,
            alt_keycode,
        });
        let flags = switcher_flags(event.flags);
        KeyOutcome {
            before: vec![Synth::Modifier {
                keycode: kc::COMMAND,
                flags: flags.without(KEY_BITS),
            }],
            ..rewrite(event.keycode, flags)
        }
    }

    /// A modifier went down or up.
    fn flags_changed(&mut self, event: &KeyEvent, cx: &KeyContext<'_>) -> KeyOutcome {
        if let Some(switcher) = self.switcher {
            if switcher.released_by(event.flags) {
                self.switcher = None;
                self.win_tap_since = None;
                return KeyOutcome {
                    after: vec![command_up(after_switcher(event.flags))],
                    ..KeyOutcome::PASS
                };
            }
            if matches!(event.keycode, kc::SHIFT | kc::RIGHT_SHIFT) {
                // The switcher reads Command+Shift as "backwards".
                self.win_tap_since = None;
                return rewrite(event.keycode, switcher_flags(event.flags));
            }
        }
        self.track_win_tap(event, cx)
    }

    /// Arms, fires or cancels the Win tap on a modifier change.
    fn track_win_tap(&mut self, event: &KeyEvent, cx: &KeyContext<'_>) -> KeyOutcome {
        let armed_at = self.win_tap_since.take();
        let applies = cx.groups.start_menu && cx.app.class != AppClass::Passthrough;
        if !applies || !is_win_key(event.keycode, cx.hid_profile) {
            // Any other modifier makes it a chord.
            return KeyOutcome::PASS;
        }
        let mods = hidmap::decode(event.flags, cx.hid_profile);
        if !mods.win {
            // Win released: a tap if it was alone and quick.
            let quick = armed_at
                .is_some_and(|since| event.timestamp_ns.saturating_sub(since) <= WIN_TAP_WINDOW_NS);
            return match cx.hotkeys.spotlight {
                Some(spotlight) if quick => KeyOutcome {
                    after: spotlight.stroke().to_vec(),
                    ..KeyOutcome::PASS
                },
                Some(_) | None => KeyOutcome::PASS,
            };
        }
        if !mods.ctrl && !mods.alt && !mods.shift && !mods.foreign {
            self.win_tap_since = Some(event.timestamp_ns);
        }
        KeyOutcome::PASS
    }

    /// Whether `event` is the autorepeat or key-up of a key whose key-down
    /// was rewritten or swallowed before a switcher session opened. Such a
    /// key keeps its pairing through the session: sent with Command as
    /// itself, the Command+M that Win+Down went down as would never come up,
    /// and a swallowed trigger would leak a key-up. A fresh key-down is
    /// never held, whatever a stale slot says.
    fn held_under_translation(&self, event: &KeyEvent) -> bool {
        let fresh = event.phase == KeyPhase::Down && !event.autorepeat;
        !fresh
            && matches!(
                self.paired_of(event.keycode),
                Paired::Rewrite(_) | Paired::Drop
            )
    }

    fn paired_of(&self, keycode: KeyCode) -> Paired {
        self.paired
            .get(usize::from(keycode))
            .copied()
            .unwrap_or(Paired::Untracked)
    }

    /// Records what a key-down decided. Keycodes without a slot are decided
    /// afresh on every event instead.
    fn pair(&mut self, keycode: KeyCode, paired: Paired) {
        if let Some(slot) = self.paired.get_mut(usize::from(keycode)) {
            *slot = paired;
        }
    }
}

impl Default for KeyboardEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// The keys the switcher reacts to, which it only sees with Command held:
/// Tab and the arrows move, Esc cancels, Q quits, H hides, the backtick
/// moves backwards.
fn is_switcher_key(keycode: KeyCode) -> bool {
    matches!(
        keycode,
        kc::TAB | kc::LEFT | kc::RIGHT | kc::UP | kc::DOWN | kc::ESCAPE | kc::Q | kc::H | kc::GRAVE
    )
}

/// Whether `keycode` is the key Win arrives as under `profile`. Positions
/// leaves a PC keyboard's second Win key, and a Mac keyboard's right
/// Option, on the right-hand Command.
fn is_win_key(keycode: KeyCode, profile: HidProfile) -> bool {
    let second = match profile {
        HidProfile::Positions => Some(kc::RIGHT_COMMAND),
        HidProfile::Windows | HidProfile::NativeCtrl => None,
    };
    Some(keycode) == hidmap::arrival_keycode(WinKey::Win, profile) || Some(keycode) == second
}

/// A switcher key's flags: Option off, the synthetic Command on, everything
/// else (Shift, Fn on arrows, CapsLock) kept.
fn switcher_flags(flags: Flags) -> Flags {
    flags.without(OPTION_BITS).union(SYNTHETIC_COMMAND)
}

/// The flags after the synthetic Command goes up with the switcher's Alt:
/// what the physical release shows, without Option and without the
/// left-hand Command, which is the synthetic one. A right-hand Command
/// (Windows Ctrl under the Windows profile) is real and stays.
fn after_switcher(flags: Flags) -> Flags {
    let rest = flags.without(OPTION_BITS).without(Flags::DEV_LCMD);
    if rest.intersects(Flags::DEV_RCMD) {
        rest
    } else {
        rest.without(Flags::COMMAND)
    }
}

/// The synthetic Command going up.
fn command_up(flags: Flags) -> Synth {
    Synth::Modifier {
        keycode: kc::COMMAND,
        flags,
    }
}

/// The event's flags for `chord`: the event's non-modifier bits, without
/// the old key's own bits when the key changes, plus the chord's.
fn chord_flags(event: &KeyEvent, chord: Chord) -> Flags {
    let kept = event.flags.non_modifier_bits();
    let kept = if chord.keycode == event.keycode {
        kept
    } else {
        kept.without(KEY_BITS)
    };
    kept.union(chord.flags())
}

/// The event sent as `chord`.
fn rewritten(event: &KeyEvent, chord: Chord) -> KeyOutcome {
    rewrite(chord.keycode, chord_flags(event, chord))
}

fn rewrite(keycode: KeyCode, flags: Flags) -> KeyOutcome {
    KeyOutcome {
        fate: Fate::Rewrite { keycode, flags },
        ..KeyOutcome::PASS
    }
}

fn dropped() -> KeyOutcome {
    KeyOutcome {
        fate: Fate::Drop,
        ..KeyOutcome::PASS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{AppProfile, AppTraits};
    use crate::engine::{Action, SystemHotkeys};
    use crate::hidmap::profile_for;
    use crate::keys::Mods;
    use crate::settings::ShortcutGroups;

    const NC: Flags = Flags::NON_COALESCED;
    /// Windows Ctrl, Win and Alt as they arrive under the Windows profile.
    const CTRL: Flags = Flags::COMMAND.union(Flags::DEV_RCMD);
    const WIN: Flags = Flags::CONTROL.union(Flags::DEV_RCTL);
    const ALT: Flags = Flags::OPTION.union(Flags::DEV_RALT);
    const SHIFT: Flags = Flags::SHIFT.union(Flags::DEV_LSHIFT);
    /// Windows Ctrl and Win as they arrive under NativeCtrl.
    const NATIVE_CTRL: Flags = Flags::CONTROL.union(Flags::DEV_RCTL);
    const NATIVE_WIN: Flags = Flags::COMMAND.union(Flags::DEV_RCMD);
    /// A Mac-style keyboard's Command and Control.
    const MAC_COMMAND: Flags = Flags::COMMAND.union(Flags::DEV_LCMD);
    const MAC_CONTROL: Flags = Flags::CONTROL.union(Flags::DEV_LCTL);
    const MS: u64 = 1_000_000;

    /// Everything a test varies about the context.
    struct Setup {
        app: AppProfile,
        hid: HidProfile,
        groups: ShortcutGroups,
        hotkeys: SystemHotkeys,
    }

    impl Setup {
        fn class(class: AppClass) -> Self {
            Setup {
                app: AppProfile {
                    class,
                    traits: AppTraits::default(),
                },
                hid: profile_for(class, true),
                groups: ShortcutGroups::all(true),
                hotkeys: SystemHotkeys::default(),
            }
        }

        fn standard() -> Self {
            Setup::class(AppClass::Standard)
        }

        fn cx(&self) -> KeyContext<'_> {
            KeyContext {
                app: self.app,
                hid_profile: self.hid,
                groups: &self.groups,
                hotkeys: &self.hotkeys,
            }
        }
    }

    /// A hardware key event: the key's own bits (Fn on arrows and the
    /// navigation block) plus `mods`.
    fn key(phase: KeyPhase, keycode: KeyCode, mods: Flags) -> KeyEvent {
        KeyEvent {
            phase,
            keycode,
            flags: Chord::new(keycode, Mods::NONE).flags().union(mods),
            autorepeat: false,
            timestamp_ns: 0,
        }
    }

    fn down(keycode: KeyCode, mods: Flags) -> KeyEvent {
        key(KeyPhase::Down, keycode, mods)
    }

    fn repeat(keycode: KeyCode, mods: Flags) -> KeyEvent {
        KeyEvent {
            autorepeat: true,
            ..down(keycode, mods)
        }
    }

    fn up(keycode: KeyCode, mods: Flags) -> KeyEvent {
        key(KeyPhase::Up, keycode, mods)
    }

    /// A modifier key event; `flags` is the state after it.
    fn modifier(keycode: KeyCode, flags: Flags, at_ns: u64) -> KeyEvent {
        KeyEvent {
            phase: KeyPhase::FlagsChanged,
            keycode,
            flags: flags.union(NC),
            autorepeat: false,
            timestamp_ns: at_ns,
        }
    }

    fn rewritten_to(keycode: KeyCode, flags: Flags) -> KeyOutcome {
        rewrite(keycode, flags)
    }

    /// Typing passes untouched and allocates nothing: the tap runs this for
    /// every key.
    #[test]
    fn plain_typing_passes_without_allocating() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        for event in [
            down(kc::A, Flags::NONE),
            repeat(kc::A, Flags::NONE),
            up(kc::A, Flags::NONE),
            down(kc::A, SHIFT),
            up(kc::A, SHIFT),
        ] {
            let outcome = engine.handle(&event, &setup.cx());
            assert!(outcome.is_pass(), "{event:?}");
            assert_eq!(outcome.before.capacity(), 0);
            assert_eq!(outcome.after.capacity(), 0);
        }
    }

    /// Ctrl+C in a standard app already is Command+C from the HID layer; the
    /// tap leaves it alone.
    #[test]
    fn a_ctrl_chord_that_means_command_passes() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        assert!(engine.handle(&down(kc::C, CTRL), &setup.cx()).is_pass());
        assert!(engine.handle(&up(kc::C, CTRL), &setup.cx()).is_pass());
    }

    /// In a terminal Ctrl+C stays Control+C and interrupts, and
    /// Ctrl+Shift+C copies; in a standard app Ctrl+Shift+C stays
    /// Command+Shift+C.
    #[test]
    fn terminal_and_standard_apps_treat_ctrl_c_differently() {
        let terminal = Setup::class(AppClass::Terminal);
        let mut engine = KeyboardEngine::new();
        assert!(
            engine
                .handle(&down(kc::C, NATIVE_CTRL), &terminal.cx())
                .is_pass()
        );
        assert!(
            engine
                .handle(&up(kc::C, NATIVE_CTRL), &terminal.cx())
                .is_pass()
        );
        assert_eq!(
            engine.handle(&down(kc::C, NATIVE_CTRL.union(SHIFT)), &terminal.cx()),
            rewritten_to(kc::C, Flags::COMMAND | Flags::DEV_LCMD | NC)
        );

        let standard = Setup::standard();
        let mut engine = KeyboardEngine::new();
        assert!(
            engine
                .handle(&down(kc::C, CTRL.union(SHIFT)), &standard.cx())
                .is_pass()
        );
    }

    /// A rewrite of the same key keeps the event's own bits (Fn and
    /// NumericPad on an arrow, CapsLock) and swaps only the modifiers.
    #[test]
    fn a_same_key_rewrite_keeps_the_non_modifier_bits() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        let event = down(kc::LEFT, CTRL.union(Flags::CAPS_LOCK));
        assert_eq!(
            engine.handle(&event, &setup.cx()),
            rewritten_to(
                kc::LEFT,
                Flags::OPTION
                    | Flags::DEV_LALT
                    | Flags::FN
                    | Flags::NUMERIC_PAD
                    | Flags::CAPS_LOCK
                    | NC
            )
        );
    }

    /// A translation to another key carries the new key's hardware bits,
    /// not the old one's: Home's Fn becomes the arrow's Fn and NumericPad,
    /// and F3's Fn does not leak onto G.
    #[test]
    fn a_new_key_gets_its_own_hardware_bits() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        assert_eq!(
            engine.handle(&down(kc::HOME, Flags::CAPS_LOCK), &setup.cx()),
            rewritten_to(
                kc::LEFT,
                Flags::COMMAND
                    | Flags::DEV_LCMD
                    | Flags::FN
                    | Flags::NUMERIC_PAD
                    | Flags::CAPS_LOCK
                    | NC
            )
        );
        assert_eq!(
            engine.handle(&down(kc::F3, Flags::NONE), &setup.cx()),
            rewritten_to(kc::G, Flags::COMMAND | Flags::DEV_LCMD | NC)
        );
    }

    /// Releasing Ctrl before the arrow of Ctrl+Left does not change what
    /// the arrow's autorepeats and key-up are: the key-down decided.
    #[test]
    fn autorepeats_and_the_key_up_follow_the_key_down_after_ctrl_is_released() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        let word_left = rewritten_to(
            kc::LEFT,
            Flags::OPTION | Flags::DEV_LALT | Flags::FN | Flags::NUMERIC_PAD | NC,
        );
        assert_eq!(engine.handle(&down(kc::LEFT, CTRL), &setup.cx()), word_left);
        assert_eq!(
            engine.handle(&repeat(kc::LEFT, CTRL), &setup.cx()),
            word_left
        );
        assert!(
            engine
                .handle(&modifier(kc::RIGHT_COMMAND, Flags::NONE, 0), &setup.cx())
                .is_pass()
        );
        assert_eq!(
            engine.handle(&repeat(kc::LEFT, Flags::NONE), &setup.cx()),
            word_left
        );
        assert_eq!(
            engine.handle(&up(kc::LEFT, Flags::NONE), &setup.cx()),
            word_left
        );
        // The pairing ends with the key-up.
        assert!(
            engine
                .handle(&down(kc::LEFT, Flags::NONE), &setup.cx())
                .is_pass()
        );
    }

    /// A key that went down untranslated stays untranslated when Ctrl joins
    /// during its autorepeat.
    #[test]
    fn a_key_that_passed_keeps_passing_when_a_modifier_joins() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        assert!(
            engine
                .handle(&down(kc::LEFT, Flags::NONE), &setup.cx())
                .is_pass()
        );
        assert!(
            engine
                .handle(&repeat(kc::LEFT, CTRL), &setup.cx())
                .is_pass()
        );
        assert!(engine.handle(&up(kc::LEFT, CTRL), &setup.cx()).is_pass());
    }

    /// A hotkey is posted once, before the swallowed trigger; its
    /// autorepeats and key-up are swallowed too, even after Win went up.
    #[test]
    fn a_hotkey_is_posted_once_and_its_trigger_is_swallowed_down_and_up() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        let show_desktop = setup.hotkeys.show_desktop.unwrap();
        assert_eq!(
            engine.handle(&down(kc::D, WIN), &setup.cx()),
            KeyOutcome {
                before: show_desktop.stroke().to_vec(),
                ..dropped()
            }
        );
        assert_eq!(engine.handle(&repeat(kc::D, WIN), &setup.cx()), dropped());
        engine.handle(&modifier(kc::RIGHT_CONTROL, Flags::NONE, 0), &setup.cx());
        assert_eq!(
            engine.handle(&up(kc::D, Flags::NONE), &setup.cx()),
            dropped()
        );
        assert!(
            engine
                .handle(&down(kc::D, Flags::NONE), &setup.cx())
                .is_pass()
        );
    }

    /// A hotkey the user switched off does nothing: the trigger is still
    /// swallowed, so Win+D does not type Control+D.
    #[test]
    fn a_switched_off_hotkey_swallows_its_trigger_and_posts_nothing() {
        let mut setup = Setup::standard();
        setup.hotkeys.show_desktop = None;
        let mut engine = KeyboardEngine::new();
        assert_eq!(engine.handle(&down(kc::D, WIN), &setup.cx()), dropped());
        assert_eq!(engine.handle(&up(kc::D, WIN), &setup.cx()), dropped());
    }

    /// Actions run once, off the tap thread, and their trigger is
    /// swallowed.
    #[test]
    fn an_action_runs_once_and_its_trigger_is_swallowed() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        assert_eq!(
            engine.handle(&down(kc::E, WIN), &setup.cx()),
            KeyOutcome {
                action: Some(Action::OpenHomeInFinder),
                ..dropped()
            }
        );
        assert_eq!(engine.handle(&repeat(kc::E, WIN), &setup.cx()), dropped());
        assert_eq!(engine.handle(&up(kc::E, WIN), &setup.cx()), dropped());
        assert_eq!(
            engine.handle(&down(kc::ESCAPE, CTRL.union(SHIFT)), &setup.cx()),
            KeyOutcome {
                action: Some(Action::OpenApp {
                    bundle_id: "com.apple.ActivityMonitor"
                }),
                ..dropped()
            }
        );
    }

    /// A chord with a modifier from a Mac-style keyboard is never
    /// translated: the MacBook keeps Command+Left and Control+D.
    #[test]
    fn chords_with_a_mac_keyboards_modifier_pass() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        assert!(
            engine
                .handle(&down(kc::LEFT, MAC_COMMAND), &setup.cx())
                .is_pass()
        );
        assert!(
            engine
                .handle(&down(kc::D, MAC_CONTROL), &setup.cx())
                .is_pass()
        );
        assert!(
            engine
                .handle(&down(kc::TAB, Flags::OPTION | Flags::DEV_LALT), &setup.cx())
                .is_pass()
        );
        assert!(!engine.needs_ticks());
    }

    /// Virtual machines and remote desktops get the keys as they are: no
    /// guard, no hotkey, no Win tap.
    #[test]
    fn passthrough_apps_get_every_key_unchanged() {
        let setup = Setup::class(AppClass::Passthrough);
        let ctrl = Flags::CONTROL | Flags::DEV_LCTL;
        let win = Flags::COMMAND | Flags::DEV_LCMD;
        let mut engine = KeyboardEngine::new();
        for event in [
            down(kc::TAB, ctrl),
            up(kc::TAB, ctrl),
            down(kc::D, win),
            up(kc::D, win),
            down(kc::HOME, Flags::NONE),
            down(kc::TAB, Flags::OPTION | Flags::DEV_LALT),
            modifier(kc::COMMAND, win, 0),
            modifier(kc::COMMAND, Flags::NONE, MS),
        ] {
            assert!(engine.handle(&event, &setup.cx()).is_pass(), "{event:?}");
        }
        assert!(!engine.needs_ticks());
    }

    /// The whole Alt+Tab gesture: the first Tab holds Command and opens the
    /// switcher, further Tabs, Shift+Tab, arrows and Esc go with Command,
    /// and releasing Alt passes and releases the Command.
    #[test]
    fn alt_tab_holds_the_switcher_open_until_alt_is_released() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        let cmd = SYNTHETIC_COMMAND.union(NC);

        assert!(
            engine
                .handle(&modifier(kc::RIGHT_OPTION, ALT, 0), &setup.cx())
                .is_pass()
        );
        assert_eq!(
            engine.handle(&down(kc::TAB, ALT), &setup.cx()),
            KeyOutcome {
                before: vec![Synth::Modifier {
                    keycode: kc::COMMAND,
                    flags: cmd
                }],
                ..rewritten_to(kc::TAB, cmd)
            }
        );
        assert!(engine.needs_ticks());
        assert_eq!(engine.switcher_alt_keycode(), Some(kc::RIGHT_OPTION));
        assert_eq!(
            engine.handle(&up(kc::TAB, ALT), &setup.cx()),
            rewritten_to(kc::TAB, cmd)
        );
        assert_eq!(
            engine.handle(&down(kc::TAB, ALT), &setup.cx()),
            rewritten_to(kc::TAB, cmd)
        );
        assert_eq!(
            engine.handle(&repeat(kc::TAB, ALT), &setup.cx()),
            rewritten_to(kc::TAB, cmd)
        );
        assert_eq!(
            engine.handle(&up(kc::TAB, ALT), &setup.cx()),
            rewritten_to(kc::TAB, cmd)
        );

        // Shift goes with Command, so the switcher moves backwards.
        let cmd_shift = cmd.union(SHIFT);
        assert_eq!(
            engine.handle(&modifier(kc::SHIFT, ALT.union(SHIFT), 0), &setup.cx()),
            rewritten_to(kc::SHIFT, cmd_shift)
        );
        assert_eq!(
            engine.handle(&down(kc::TAB, ALT.union(SHIFT)), &setup.cx()),
            rewritten_to(kc::TAB, cmd_shift)
        );
        assert_eq!(
            engine.handle(&up(kc::TAB, ALT.union(SHIFT)), &setup.cx()),
            rewritten_to(kc::TAB, cmd_shift)
        );
        assert_eq!(
            engine.handle(&modifier(kc::SHIFT, ALT, 0), &setup.cx()),
            rewritten_to(kc::SHIFT, cmd)
        );

        // The switcher's own keys.
        assert_eq!(
            engine.handle(&down(kc::RIGHT, ALT), &setup.cx()),
            rewritten_to(kc::RIGHT, cmd | Flags::FN | Flags::NUMERIC_PAD)
        );
        assert_eq!(
            engine.handle(&down(kc::ESCAPE, ALT), &setup.cx()),
            rewritten_to(kc::ESCAPE, cmd)
        );
        assert_eq!(
            engine.handle(&down(kc::Q, ALT), &setup.cx()),
            rewritten_to(kc::Q, cmd)
        );

        // Releasing Alt passes, then Command goes up and the switcher
        // commits. No synthetic Option release is posted.
        assert_eq!(
            engine.handle(&modifier(kc::RIGHT_OPTION, Flags::NONE, 0), &setup.cx()),
            KeyOutcome {
                after: vec![Synth::Modifier {
                    keycode: kc::COMMAND,
                    flags: NC
                }],
                ..KeyOutcome::PASS
            }
        );
        assert!(!engine.needs_ticks());
        assert_eq!(engine.switcher_alt_keycode(), None);
        // A switcher key held across the release comes up unchanged.
        assert!(
            engine
                .handle(&up(kc::Q, Flags::NONE), &setup.cx())
                .is_pass()
        );
        assert!(
            engine
                .handle(&down(kc::TAB, Flags::NONE), &setup.cx())
                .is_pass()
        );
    }

    /// A key that went down under a translation before the switcher opened
    /// keeps it: Win+Down's key-up releases the M it was sent as rather
    /// than coming up as Command+Down, and Win+Left's key-up stays
    /// swallowed. A fresh press of either key still goes to the switcher.
    #[test]
    fn a_key_held_under_a_translation_keeps_it_when_the_switcher_opens() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        let minimize = rewritten_to(kc::M, Flags::COMMAND | Flags::DEV_LCMD | NC);
        assert_eq!(engine.handle(&down(kc::DOWN, WIN), &setup.cx()), minimize);
        assert_eq!(
            engine.handle(&down(kc::LEFT, WIN), &setup.cx()).fate,
            Fate::Drop
        );
        engine.handle(&modifier(kc::RIGHT_CONTROL, Flags::NONE, 0), &setup.cx());
        engine.handle(&modifier(kc::RIGHT_OPTION, ALT, 0), &setup.cx());
        engine.handle(&down(kc::TAB, ALT), &setup.cx());
        assert!(engine.needs_ticks(), "the switcher is open");

        // The key-up of a key sent as another key carries no modifiers.
        assert_eq!(
            engine.handle(&up(kc::DOWN, ALT), &setup.cx()),
            rewritten_to(kc::M, NC)
        );
        assert_eq!(engine.handle(&up(kc::LEFT, ALT), &setup.cx()), dropped());
        let cmd_arrow = SYNTHETIC_COMMAND | NC | Flags::FN | Flags::NUMERIC_PAD;
        assert_eq!(
            engine.handle(&down(kc::DOWN, ALT), &setup.cx()),
            rewritten_to(kc::DOWN, cmd_arrow)
        );
        assert_eq!(
            engine.handle(&up(kc::DOWN, ALT), &setup.cx()),
            rewritten_to(kc::DOWN, cmd_arrow)
        );
    }

    /// Alt+Shift+Tab opens the switcher backwards: the synthetic Command
    /// goes down with Shift.
    #[test]
    fn alt_shift_tab_opens_the_switcher_with_shift() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        let cmd_shift = SYNTHETIC_COMMAND.union(SHIFT).union(NC);
        assert_eq!(
            engine.handle(&down(kc::TAB, ALT.union(SHIFT)), &setup.cx()),
            KeyOutcome {
                before: vec![Synth::Modifier {
                    keycode: kc::COMMAND,
                    flags: cmd_shift
                }],
                ..rewritten_to(kc::TAB, cmd_shift)
            }
        );
        // Alt released while Shift is still down: Command goes up, Shift
        // stays in the flags.
        let outcome = engine.handle(&modifier(kc::RIGHT_OPTION, SHIFT, 0), &setup.cx());
        assert_eq!(
            outcome.after,
            vec![Synth::Modifier {
                keycode: kc::COMMAND,
                flags: SHIFT.union(NC)
            }]
        );
    }

    /// Releasing Alt while Windows Ctrl (a right-hand Command) is held
    /// releases only the synthetic left-hand Command.
    #[test]
    fn the_command_release_keeps_a_real_right_hand_command() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        engine.handle(&down(kc::TAB, ALT), &setup.cx());
        // The flags may show the synthetic Command next to the real one.
        let release = modifier(kc::RIGHT_OPTION, CTRL.union(SYNTHETIC_COMMAND), 0);
        let outcome = engine.handle(&release, &setup.cx());
        assert_eq!(
            outcome.after,
            vec![Synth::Modifier {
                keycode: kc::COMMAND,
                flags: CTRL.union(NC)
            }]
        );
    }

    /// In a terminal Alt is the left Option; the session follows that key,
    /// and the watchdog is told to read it.
    #[test]
    fn alt_tab_works_in_terminals_with_the_left_option() {
        let setup = Setup::class(AppClass::Terminal);
        let alt = Flags::OPTION | Flags::DEV_LALT;
        let mut engine = KeyboardEngine::new();
        let outcome = engine.handle(&down(kc::TAB, alt), &setup.cx());
        assert_eq!(
            outcome.fate,
            Fate::Rewrite {
                keycode: kc::TAB,
                flags: SYNTHETIC_COMMAND.union(NC)
            }
        );
        assert_eq!(engine.switcher_alt_keycode(), Some(kc::OPTION));
        // The right Option going down and up does not end the session.
        let both = alt | Flags::DEV_RALT;
        assert!(
            engine
                .handle(&modifier(kc::RIGHT_OPTION, both, 0), &setup.cx())
                .after
                .is_empty()
        );
        assert!(
            engine
                .handle(&modifier(kc::RIGHT_OPTION, alt, 0), &setup.cx())
                .after
                .is_empty()
        );
        assert!(engine.needs_ticks());
        let outcome = engine.handle(&modifier(kc::OPTION, Flags::NONE, 0), &setup.cx());
        assert_eq!(outcome.after.len(), 1);
        assert!(!engine.needs_ticks());
    }

    /// With the windows group off Alt+Tab stays Option+Tab.
    #[test]
    fn alt_tab_passes_with_the_windows_group_off() {
        let mut setup = Setup::standard();
        setup.groups.windows = false;
        let mut engine = KeyboardEngine::new();
        assert!(engine.handle(&down(kc::TAB, ALT), &setup.cx()).is_pass());
        assert!(!engine.needs_ticks());
    }

    /// The watchdog ends a session whose Alt release was missed, and does
    /// nothing while Alt is still down; the late release then passes alone.
    #[test]
    fn the_watchdog_releases_command_when_alt_is_no_longer_down() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        assert!(engine.tick(0, false).is_empty());
        engine.handle(&down(kc::TAB, ALT), &setup.cx());
        assert!(engine.tick(150 * MS, true).is_empty());
        assert!(engine.needs_ticks());
        assert_eq!(
            engine.tick(300 * MS, false),
            vec![Synth::Modifier {
                keycode: kc::COMMAND,
                flags: NC
            }]
        );
        assert!(!engine.needs_ticks());
        assert!(engine.tick(450 * MS, false).is_empty());
        assert!(
            engine
                .handle(&modifier(kc::RIGHT_OPTION, Flags::NONE, 0), &setup.cx())
                .is_pass()
        );
    }

    /// A posted shortcut's key-up carries no modifiers, and the key-up of a
    /// key sent as another key does not either, so neither leaves Command
    /// held in the HID system's state; the key-down keeps them, which is
    /// what fires the shortcut.
    #[test]
    fn synthetic_key_ups_never_carry_modifiers() {
        let [down_event, up_event] = Chord::new(kc::SPACE, Mods::COMMAND).stroke();
        let Synth::Key {
            flags: down_flags, ..
        } = down_event
        else {
            panic!("a stroke is two key events");
        };
        let Synth::Key {
            flags: up_flags, ..
        } = up_event
        else {
            panic!("a stroke is two key events");
        };
        assert!(down_flags.contains(Flags::COMMAND));
        assert!(!up_flags.intersects(Flags::MODIFIERS.union(Flags::DEVICE_SIDES)));

        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        engine.handle(&down(kc::HOME, Flags::NONE), &setup.cx());
        let Fate::Rewrite { keycode, flags } =
            engine.handle(&up(kc::HOME, Flags::NONE), &setup.cx()).fate
        else {
            panic!("Home's key-up is sent as Left's");
        };
        assert_eq!(keycode, kc::LEFT);
        assert!(!flags.intersects(Flags::MODIFIERS));
    }

    /// Releasing everything ends the session with a Command release, lets
    /// go of keys held under another key's name, and forgets every pairing.
    #[test]
    fn release_all_lets_go_of_everything_isthmus_holds() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        engine.handle(&down(kc::HOME, Flags::NONE), &setup.cx());
        engine.handle(&down(kc::LEFT, CTRL), &setup.cx());
        engine.handle(&down(kc::D, WIN), &setup.cx());
        engine.handle(&down(kc::TAB, ALT), &setup.cx());

        let released = engine.release_all();

        let home_as_left = Chord::new(kc::LEFT, Mods::COMMAND);
        assert_eq!(
            released,
            vec![
                Synth::Key {
                    keycode: kc::LEFT,
                    flags: home_as_left.flags().non_modifier_bits(),
                    down: false
                },
                Synth::Modifier {
                    keycode: kc::COMMAND,
                    flags: NC
                },
            ]
        );
        assert!(!engine.needs_ticks());
        assert!(engine.release_all().is_empty());
        // The pairings are gone: key-ups pass, and an autorepeat is
        // decided afresh.
        assert!(
            engine
                .handle(&up(kc::D, Flags::NONE), &setup.cx())
                .is_pass()
        );
        assert!(
            engine
                .handle(&up(kc::HOME, Flags::NONE), &setup.cx())
                .is_pass()
        );
        assert_eq!(
            engine.handle(&repeat(kc::LEFT, CTRL), &setup.cx()).fate,
            Fate::Rewrite {
                keycode: kc::LEFT,
                flags: Flags::OPTION | Flags::DEV_LALT | Flags::FN | Flags::NUMERIC_PAD | NC
            }
        );
    }

    /// An autorepeat decided afresh never posts a hotkey or runs an action:
    /// only a real key-down does.
    #[test]
    fn an_unpaired_autorepeat_never_posts_a_hotkey_or_an_action() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        assert_eq!(engine.handle(&repeat(kc::D, WIN), &setup.cx()), dropped());
        assert_eq!(engine.handle(&repeat(kc::E, WIN), &setup.cx()), dropped());
        assert_eq!(engine.handle(&up(kc::E, WIN), &setup.cx()), dropped());
    }

    /// Win pressed and released alone within 300 ms opens Spotlight after
    /// the release passes.
    #[test]
    fn a_quick_lone_win_tap_opens_spotlight() {
        let setup = Setup::standard();
        let spotlight = setup.hotkeys.spotlight.unwrap();
        let mut engine = KeyboardEngine::new();
        assert!(
            engine
                .handle(&modifier(kc::RIGHT_CONTROL, WIN, 1_000 * MS), &setup.cx())
                .is_pass()
        );
        assert_eq!(
            engine.handle(
                &modifier(kc::RIGHT_CONTROL, Flags::NONE, 1_300 * MS),
                &setup.cx()
            ),
            KeyOutcome {
                after: spotlight.stroke().to_vec(),
                ..KeyOutcome::PASS
            }
        );
        // In a NativeCtrl app Win is the right Command.
        let native = Setup::class(AppClass::NativeCtrl);
        engine.handle(&modifier(kc::RIGHT_COMMAND, NATIVE_WIN, 0), &native.cx());
        let outcome = engine.handle(
            &modifier(kc::RIGHT_COMMAND, Flags::NONE, 10 * MS),
            &native.cx(),
        );
        assert_eq!(outcome.after, spotlight.stroke().to_vec());
    }

    /// A Win press held past 300 ms is not a tap.
    #[test]
    fn a_slow_win_press_is_not_a_tap() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        engine.handle(&modifier(kc::RIGHT_CONTROL, WIN, 0), &setup.cx());
        assert!(
            engine
                .handle(
                    &modifier(kc::RIGHT_CONTROL, Flags::NONE, 301 * MS),
                    &setup.cx()
                )
                .is_pass()
        );
    }

    /// A key, a click, the wheel or another modifier between Win's press and
    /// release make it a chord, not a tap; so does any Win+key chord.
    #[test]
    fn anything_between_win_down_and_up_cancels_the_tap() {
        let setup = Setup::standard();
        let press = modifier(kc::RIGHT_CONTROL, WIN, 0);
        let release = modifier(kc::RIGHT_CONTROL, Flags::NONE, 100 * MS);
        let interruptions: [&dyn Fn(&mut KeyboardEngine); 4] = [
            &|engine| {
                engine.handle(&down(kc::D, WIN), &setup.cx());
                engine.handle(&up(kc::D, WIN), &setup.cx());
            },
            &|engine| {
                engine.handle(&down(kc::A, WIN), &setup.cx());
            },
            &|engine| engine.note_pointer_activity(),
            &|engine| {
                engine.handle(&modifier(kc::SHIFT, WIN.union(SHIFT), 0), &setup.cx());
                engine.handle(&modifier(kc::SHIFT, WIN, 0), &setup.cx());
            },
        ];
        for (index, interrupt) in interruptions.iter().enumerate() {
            let mut engine = KeyboardEngine::new();
            engine.handle(&press, &setup.cx());
            interrupt(&mut engine);
            assert!(
                engine.handle(&release, &setup.cx()).is_pass(),
                "case {index}"
            );
        }
    }

    /// Win pressed while another modifier is held never arms a tap.
    #[test]
    fn win_pressed_with_another_modifier_is_not_a_tap() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        engine.handle(
            &modifier(kc::RIGHT_CONTROL, WIN.union(SHIFT), 0),
            &setup.cx(),
        );
        assert!(
            engine
                .handle(&modifier(kc::RIGHT_CONTROL, SHIFT, 50 * MS), &setup.cx())
                .is_pass()
        );
    }

    /// With the start menu group off, or Spotlight's hotkey switched off,
    /// a Win tap does nothing.
    #[test]
    fn the_win_tap_needs_its_group_and_spotlight() {
        let press = modifier(kc::RIGHT_CONTROL, WIN, 0);
        let release = modifier(kc::RIGHT_CONTROL, Flags::NONE, 50 * MS);
        let mut off = Setup::standard();
        off.groups.start_menu = false;
        let mut no_spotlight = Setup::standard();
        no_spotlight.hotkeys.spotlight = None;
        for setup in [off, no_spotlight] {
            let mut engine = KeyboardEngine::new();
            engine.handle(&press, &setup.cx());
            assert!(engine.handle(&release, &setup.cx()).is_pass());
        }
    }

    /// Ctrl+Esc opens Spotlight too, as the Start menu does on Windows.
    #[test]
    fn ctrl_esc_opens_spotlight() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        assert_eq!(
            engine.handle(&down(kc::ESCAPE, CTRL), &setup.cx()),
            KeyOutcome {
                before: setup.hotkeys.spotlight.unwrap().stroke().to_vec(),
                ..dropped()
            }
        );
    }

    /// Keycodes outside the pairing table are decided on every event
    /// rather than paired, and never panic.
    #[test]
    fn keycodes_without_a_pairing_slot_pass() {
        let setup = Setup::standard();
        let mut engine = KeyboardEngine::new();
        assert!(engine.handle(&down(0x80, CTRL), &setup.cx()).is_pass());
        assert!(engine.handle(&up(0x80, CTRL), &setup.cx()).is_pass());
        assert!(
            engine
                .handle(&down(u16::MAX, Flags::NONE), &setup.cx())
                .is_pass()
        );
    }
}
