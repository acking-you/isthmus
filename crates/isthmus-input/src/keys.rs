//! The numbers macOS uses for keys, modifier flags and HID usages.
//!
//! Virtual keycodes are `kVK_*` from HIToolbox `Events.h`; flag bits are
//! `CGEventFlags` plus the device-dependent `NX_DEVICE*` side bits from
//! `IOLLEvent.h`; HID usages are page 7 (keyboard) as `UserKeyMapping` encodes
//! them. They live here, free of any OS crate, so the engine and its tests can
//! use them on every platform.

/// A macOS virtual keycode (`CGKeyCode`).
pub type KeyCode = u16;

/// `kVK_*` virtual keycodes (HIToolbox `Events.h`). ANSI positions: a keycode
/// names a place on the keyboard, not a character.
pub mod kc {
    use super::KeyCode;

    pub const A: KeyCode = 0x00;
    pub const S: KeyCode = 0x01;
    pub const D: KeyCode = 0x02;
    pub const F: KeyCode = 0x03;
    pub const H: KeyCode = 0x04;
    pub const G: KeyCode = 0x05;
    pub const Z: KeyCode = 0x06;
    pub const X: KeyCode = 0x07;
    pub const C: KeyCode = 0x08;
    pub const V: KeyCode = 0x09;
    pub const ISO_SECTION: KeyCode = 0x0A;
    pub const B: KeyCode = 0x0B;
    pub const Q: KeyCode = 0x0C;
    pub const W: KeyCode = 0x0D;
    pub const E: KeyCode = 0x0E;
    pub const R: KeyCode = 0x0F;
    pub const Y: KeyCode = 0x10;
    pub const T: KeyCode = 0x11;
    pub const N1: KeyCode = 0x12;
    pub const N2: KeyCode = 0x13;
    pub const N3: KeyCode = 0x14;
    pub const N4: KeyCode = 0x15;
    pub const N6: KeyCode = 0x16;
    pub const N5: KeyCode = 0x17;
    pub const EQUAL: KeyCode = 0x18;
    pub const N9: KeyCode = 0x19;
    pub const N7: KeyCode = 0x1A;
    pub const MINUS: KeyCode = 0x1B;
    pub const N8: KeyCode = 0x1C;
    pub const N0: KeyCode = 0x1D;
    pub const RIGHT_BRACKET: KeyCode = 0x1E;
    pub const O: KeyCode = 0x1F;
    pub const U: KeyCode = 0x20;
    pub const LEFT_BRACKET: KeyCode = 0x21;
    pub const I: KeyCode = 0x22;
    pub const P: KeyCode = 0x23;
    pub const RETURN: KeyCode = 0x24;
    pub const L: KeyCode = 0x25;
    pub const J: KeyCode = 0x26;
    pub const QUOTE: KeyCode = 0x27;
    pub const K: KeyCode = 0x28;
    pub const SEMICOLON: KeyCode = 0x29;
    pub const BACKSLASH: KeyCode = 0x2A;
    pub const COMMA: KeyCode = 0x2B;
    pub const SLASH: KeyCode = 0x2C;
    pub const N: KeyCode = 0x2D;
    pub const M: KeyCode = 0x2E;
    pub const PERIOD: KeyCode = 0x2F;
    pub const TAB: KeyCode = 0x30;
    pub const SPACE: KeyCode = 0x31;
    pub const GRAVE: KeyCode = 0x32;
    /// Backspace on a PC keyboard.
    pub const DELETE: KeyCode = 0x33;
    pub const ESCAPE: KeyCode = 0x35;
    pub const RIGHT_COMMAND: KeyCode = 0x36;
    pub const COMMAND: KeyCode = 0x37;
    pub const SHIFT: KeyCode = 0x38;
    pub const CAPS_LOCK: KeyCode = 0x39;
    pub const OPTION: KeyCode = 0x3A;
    pub const CONTROL: KeyCode = 0x3B;
    pub const RIGHT_SHIFT: KeyCode = 0x3C;
    pub const RIGHT_OPTION: KeyCode = 0x3D;
    pub const RIGHT_CONTROL: KeyCode = 0x3E;
    pub const FUNCTION: KeyCode = 0x3F;
    pub const F17: KeyCode = 0x40;
    pub const KEYPAD_DECIMAL: KeyCode = 0x41;
    pub const KEYPAD_MULTIPLY: KeyCode = 0x43;
    pub const KEYPAD_PLUS: KeyCode = 0x45;
    pub const KEYPAD_CLEAR: KeyCode = 0x47;
    pub const VOLUME_UP: KeyCode = 0x48;
    pub const VOLUME_DOWN: KeyCode = 0x49;
    pub const MUTE: KeyCode = 0x4A;
    pub const KEYPAD_DIVIDE: KeyCode = 0x4B;
    pub const KEYPAD_ENTER: KeyCode = 0x4C;
    pub const KEYPAD_MINUS: KeyCode = 0x4E;
    pub const F18: KeyCode = 0x4F;
    pub const F19: KeyCode = 0x50;
    pub const KEYPAD_EQUALS: KeyCode = 0x51;
    pub const KEYPAD_0: KeyCode = 0x52;
    pub const KEYPAD_1: KeyCode = 0x53;
    pub const KEYPAD_2: KeyCode = 0x54;
    pub const KEYPAD_3: KeyCode = 0x55;
    pub const KEYPAD_4: KeyCode = 0x56;
    pub const KEYPAD_5: KeyCode = 0x57;
    pub const KEYPAD_6: KeyCode = 0x58;
    pub const KEYPAD_7: KeyCode = 0x59;
    pub const F20: KeyCode = 0x5A;
    pub const KEYPAD_8: KeyCode = 0x5B;
    pub const KEYPAD_9: KeyCode = 0x5C;
    pub const JIS_YEN: KeyCode = 0x5D;
    pub const JIS_UNDERSCORE: KeyCode = 0x5E;
    pub const JIS_KEYPAD_COMMA: KeyCode = 0x5F;
    pub const F5: KeyCode = 0x60;
    pub const F6: KeyCode = 0x61;
    pub const F7: KeyCode = 0x62;
    pub const F3: KeyCode = 0x63;
    pub const F8: KeyCode = 0x64;
    pub const F9: KeyCode = 0x65;
    pub const JIS_EISU: KeyCode = 0x66;
    pub const F11: KeyCode = 0x67;
    pub const JIS_KANA: KeyCode = 0x68;
    /// PrintScreen on a PC keyboard arrives as F13.
    pub const F13: KeyCode = 0x69;
    pub const F16: KeyCode = 0x6A;
    /// ScrollLock on a PC keyboard arrives as F14.
    pub const F14: KeyCode = 0x6B;
    pub const F10: KeyCode = 0x6D;
    /// The PC Menu (Application) key.
    pub const CONTEXTUAL_MENU: KeyCode = 0x6E;
    pub const F12: KeyCode = 0x6F;
    /// Pause on a PC keyboard arrives as F15.
    pub const F15: KeyCode = 0x71;
    /// Insert on a PC keyboard arrives as Help.
    pub const HELP: KeyCode = 0x72;
    pub const HOME: KeyCode = 0x73;
    pub const PAGE_UP: KeyCode = 0x74;
    /// Delete on a PC keyboard.
    pub const FORWARD_DELETE: KeyCode = 0x75;
    pub const F4: KeyCode = 0x76;
    pub const END: KeyCode = 0x77;
    pub const F2: KeyCode = 0x78;
    pub const PAGE_DOWN: KeyCode = 0x79;
    pub const F1: KeyCode = 0x7A;
    pub const LEFT: KeyCode = 0x7B;
    pub const RIGHT: KeyCode = 0x7C;
    pub const DOWN: KeyCode = 0x7D;
    pub const UP: KeyCode = 0x7E;

    /// Aliases in Windows words, for rule tables that read like Windows.
    pub const PRINT_SCREEN: KeyCode = F13;
    pub const SCROLL_LOCK: KeyCode = F14;
    pub const PAUSE: KeyCode = F15;
    pub const INSERT: KeyCode = HELP;

    /// The function-row keys, F1 to F20.
    pub const F_KEYS: [KeyCode; 20] = [
        F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20,
    ];

    /// Is `code` a modifier key (one that arrives as `flagsChanged`)?
    pub const fn is_modifier(code: KeyCode) -> bool {
        matches!(
            code,
            RIGHT_COMMAND
                | COMMAND
                | SHIFT
                | CAPS_LOCK
                | OPTION
                | CONTROL
                | RIGHT_SHIFT
                | RIGHT_OPTION
                | RIGHT_CONTROL
                | FUNCTION
        )
    }

    /// Is `code` an arrow key? Hardware arrow events carry `Fn | NumericPad`.
    pub const fn is_arrow(code: KeyCode) -> bool {
        matches!(code, LEFT | RIGHT | DOWN | UP)
    }

    /// Is `code` one of the navigation keys that hardware sends with `Fn`?
    pub const fn is_fn_navigation(code: KeyCode) -> bool {
        matches!(
            code,
            HOME | END | PAGE_UP | PAGE_DOWN | FORWARD_DELETE | HELP
        )
    }

    /// Is `code` an F-key?
    pub const fn is_function_key(code: KeyCode) -> bool {
        let mut i = 0;
        while i < F_KEYS.len() {
            if F_KEYS[i] == code {
                return true;
            }
            i += 1;
        }
        false
    }
}

/// `CGEventFlags` as raw bits: device-independent modifier masks plus the
/// device-dependent side bits. Isthmus reads the side bits to tell a
/// keyboard it remapped from one it left alone (see [`crate::hidmap`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Flags(pub u64);

impl Flags {
    pub const NONE: Flags = Flags(0);
    pub const CAPS_LOCK: Flags = Flags(0x0001_0000);
    pub const SHIFT: Flags = Flags(0x0002_0000);
    pub const CONTROL: Flags = Flags(0x0004_0000);
    pub const OPTION: Flags = Flags(0x0008_0000);
    pub const COMMAND: Flags = Flags(0x0010_0000);
    pub const NUMERIC_PAD: Flags = Flags(0x0020_0000);
    pub const HELP: Flags = Flags(0x0040_0000);
    pub const FN: Flags = Flags(0x0080_0000);
    pub const NON_COALESCED: Flags = Flags(0x0000_0100);

    pub const DEV_LCTL: Flags = Flags(0x0000_0001);
    pub const DEV_LSHIFT: Flags = Flags(0x0000_0002);
    pub const DEV_RSHIFT: Flags = Flags(0x0000_0004);
    pub const DEV_LCMD: Flags = Flags(0x0000_0008);
    pub const DEV_RCMD: Flags = Flags(0x0000_0010);
    pub const DEV_LALT: Flags = Flags(0x0000_0020);
    pub const DEV_RALT: Flags = Flags(0x0000_0040);
    pub const DEV_RCTL: Flags = Flags(0x0000_2000);

    /// Shift, Control, Option and Command, device-independent.
    pub const MODIFIERS: Flags = Flags(0x001E_0000);
    /// Every device-dependent side bit.
    pub const DEVICE_SIDES: Flags = Flags(0x0000_207F);

    pub const fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Flags) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: Flags) -> Flags {
        Flags(self.0 | other.0)
    }

    pub const fn without(self, other: Flags) -> Flags {
        Flags(self.0 & !other.0)
    }

    /// The bits that are neither modifiers nor side bits (Fn, NumericPad,
    /// CapsLock, NonCoalesced, ...), which a rewrite keeps.
    pub const fn non_modifier_bits(self) -> Flags {
        self.without(Self::MODIFIERS).without(Self::DEVICE_SIDES)
    }
}

impl std::fmt::Debug for Flags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Flags({:#x})", self.0)
    }
}

impl std::ops::BitOr for Flags {
    type Output = Flags;

    fn bitor(self, rhs: Flags) -> Flags {
        self.union(rhs)
    }
}

/// The four modifiers as a small set, side-agnostic: what a rule matches on
/// and what an output chord asks for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct Mods {
    pub control: bool,
    pub option: bool,
    pub shift: bool,
    pub command: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        control: false,
        option: false,
        shift: false,
        command: false,
    };
    pub const CONTROL: Mods = Mods {
        control: true,
        ..Mods::NONE
    };
    pub const OPTION: Mods = Mods {
        option: true,
        ..Mods::NONE
    };
    pub const SHIFT: Mods = Mods {
        shift: true,
        ..Mods::NONE
    };
    pub const COMMAND: Mods = Mods {
        command: true,
        ..Mods::NONE
    };

    pub const fn with(self, other: Mods) -> Mods {
        Mods {
            control: self.control || other.control,
            option: self.option || other.option,
            shift: self.shift || other.shift,
            command: self.command || other.command,
        }
    }

    pub const fn is_empty(self) -> bool {
        !(self.control || self.option || self.shift || self.command)
    }

    /// The device-independent flags for these modifiers, with the left-side
    /// device bit of each, as a hardware event from a left key carries.
    pub const fn to_flags(self) -> Flags {
        let mut bits = 0u64;
        if self.control {
            bits |= Flags::CONTROL.0 | Flags::DEV_LCTL.0;
        }
        if self.option {
            bits |= Flags::OPTION.0 | Flags::DEV_LALT.0;
        }
        if self.shift {
            bits |= Flags::SHIFT.0 | Flags::DEV_LSHIFT.0;
        }
        if self.command {
            bits |= Flags::COMMAND.0 | Flags::DEV_LCMD.0;
        }
        Flags(bits)
    }
}

/// HID usages as `UserKeyMapping` encodes them: `(usage page << 32) | usage`.
/// The value must be written as a 64-bit CFNumber.
pub mod usage {
    /// A `UserKeyMapping` key value.
    pub type HidUsage = u64;

    const fn keyboard(usage: u64) -> HidUsage {
        0x7_0000_0000 | usage
    }

    pub const LEFT_CONTROL: HidUsage = keyboard(0xE0);
    pub const LEFT_SHIFT: HidUsage = keyboard(0xE1);
    /// Left Alt on a PC keyboard, Option on a Mac.
    pub const LEFT_ALT: HidUsage = keyboard(0xE2);
    /// The Windows key on a PC keyboard, Command on a Mac.
    pub const LEFT_GUI: HidUsage = keyboard(0xE3);
    pub const RIGHT_CONTROL: HidUsage = keyboard(0xE4);
    pub const RIGHT_SHIFT: HidUsage = keyboard(0xE5);
    pub const RIGHT_ALT: HidUsage = keyboard(0xE6);
    pub const RIGHT_GUI: HidUsage = keyboard(0xE7);
    pub const CAPS_LOCK: HidUsage = keyboard(0x39);
    pub const APPLICATION: HidUsage = keyboard(0x65);
    /// Apple's Fn / Globe key (AppleVendorTopCase page 0x00FF, usage 0x03).
    pub const APPLE_FN: HidUsage = 0xFF_0000_0003;
    /// The alternate Fn form (AppleVendorKeyboard page 0xFF01, usage 0x03).
    pub const APPLE_FN_ALT: HidUsage = 0xFF01_0000_0003;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The usage encoding is the one TN2450 and the HID filter use: page in
    /// the high 32 bits. A 32-bit truncation would silently map nothing.
    #[test]
    fn usages_carry_their_page_above_bit_32() {
        assert_eq!(usage::LEFT_GUI, 30_064_771_299);
        assert_eq!(usage::RIGHT_ALT, 0x7_0000_00E6);
        assert_eq!(usage::APPLE_FN, 1_095_216_660_483);
    }

    /// A synthesized chord carries the left-side bits a real left key sends.
    #[test]
    fn mods_become_left_side_hardware_flags() {
        let flags = Mods::COMMAND.with(Mods::SHIFT).to_flags();
        assert!(flags.contains(Flags::COMMAND | Flags::DEV_LCMD));
        assert!(flags.contains(Flags::SHIFT | Flags::DEV_LSHIFT));
        assert!(!flags.intersects(Flags::CONTROL));
    }

    /// Fn and NumericPad on an arrow survive a modifier rewrite.
    #[test]
    fn rewrites_keep_the_non_modifier_bits() {
        let arrow = Flags::FN | Flags::NUMERIC_PAD | Flags::CONTROL | Flags::DEV_LCTL;
        assert_eq!(arrow.non_modifier_bits(), Flags::FN | Flags::NUMERIC_PAD);
    }

    /// The masks are exactly their parts: `DEVICE_SIDES` is the eight side
    /// bits and `MODIFIERS` the four modifiers, so stripping them in a
    /// rewrite removes every modifier and nothing else.
    #[test]
    fn the_masks_are_exactly_their_parts() {
        let sides = [
            Flags::DEV_LCTL,
            Flags::DEV_LSHIFT,
            Flags::DEV_RSHIFT,
            Flags::DEV_LCMD,
            Flags::DEV_RCMD,
            Flags::DEV_LALT,
            Flags::DEV_RALT,
            Flags::DEV_RCTL,
        ];
        assert_eq!(
            sides.into_iter().fold(Flags::NONE, Flags::union),
            Flags::DEVICE_SIDES
        );
        assert_eq!(
            Flags::SHIFT | Flags::CONTROL | Flags::OPTION | Flags::COMMAND,
            Flags::MODIFIERS
        );
        assert!(!Flags::MODIFIERS.intersects(Flags::DEVICE_SIDES));
    }

    /// The F-key row is twenty distinct keys, none of them a modifier or a
    /// navigation key.
    #[test]
    fn the_function_row_is_twenty_distinct_keys() {
        let mut keys = kc::F_KEYS.to_vec();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 20);
        assert!(
            keys.iter()
                .all(|&key| !kc::is_modifier(key) && !kc::is_fn_navigation(key))
        );
        assert!(kc::is_fn_navigation(kc::INSERT));
    }

    /// Modifiers, arrows and F-keys are told apart by keycode.
    #[test]
    fn classifies_keys() {
        assert!(kc::is_modifier(kc::RIGHT_CONTROL));
        assert!(!kc::is_modifier(kc::TAB));
        assert!(kc::is_arrow(kc::UP));
        assert!(kc::is_function_key(kc::F13));
        assert!(!kc::is_function_key(kc::HOME));
    }
}
