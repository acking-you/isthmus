//! What the user asked for, as one versioned JSON file.
//!
//! The file is read by every later version, so fields are added with
//! defaults, unknown keys are kept when the file is rewritten, and an unknown
//! enum value reads as the variant that changes nothing. Unknown keys are
//! kept wherever a struct carries an `unknown` map: the top level, the
//! keyboard and mouse sections, each device entry and each app rule.
//! [`ShortcutGroups`] and [`MouseProfile`] carry none, because they are
//! `Copy` values compared against the presets, so a later version adds its
//! fields next to them, never inside them. Presets are values,
//! not modes: picking "Windows" writes the Windows values, and a page shows
//! "Custom" when the values match neither preset.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::context::AppClass;

/// The version this build writes.
pub const SETTINGS_VERSION: u32 = 1;

/// The fewest and most lines one wheel notch may scroll.
pub const LINES_PER_NOTCH_RANGE: std::ops::RangeInclusive<u8> = 1..=12;
/// Windows' pointer speed scale (Control Panel `MouseSensitivity`).
pub const POINTER_SPEED_RANGE: std::ops::RangeInclusive<u8> = 1..=20;
/// Windows' default speed, the sixth of eleven notches.
pub const WINDOWS_DEFAULT_POINTER_SPEED: u8 = 10;
/// Windows' default wheel distance.
pub const WINDOWS_DEFAULT_LINES_PER_NOTCH: u8 = 3;

/// Everything Isthmus does to the keyboard and mouse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub version: u32,
    /// The one switch: off puts back every system change and removes the
    /// event tap; on applies the keyboard and mouse sections below.
    pub enabled: bool,
    pub keyboard: KeyboardSettings,
    pub mouse: MouseSettings,
    /// Keys a newer version wrote, kept so this one does not delete them.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

impl Default for Settings {
    /// Windows habits, switched off: nothing changes until the user turns
    /// Isthmus on.
    fn default() -> Self {
        Settings {
            version: SETTINGS_VERSION,
            enabled: false,
            keyboard: KeyboardSettings::default(),
            mouse: MouseSettings::default(),
            unknown: Map::new(),
        }
    }
}

/// How a keyboard behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyboardStyle {
    /// Ctrl acts as Command, the keys sit where Windows puts them, and the
    /// Windows shortcuts below are translated.
    Windows,
    /// Left as macOS has it.
    #[serde(other)]
    Mac,
}

/// Which modifier row a keyboard has, when detection guesses wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LayoutChoice {
    /// fn | control | option | command | space.
    Mac,
    /// Ctrl | Win | Alt | space.
    Pc,
    /// Whatever [`crate::device::detect_layout`] decides.
    #[serde(other)]
    Auto,
}

/// What the F1–F12 row sends on a keyboard whose firmware offers media keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FunctionKeys {
    /// F1–F12 without holding Fn, as on Windows (F2 rename, F5 refresh).
    Standard,
    /// Whatever System Settings says.
    #[serde(other)]
    System,
}

/// The keyboard section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KeyboardSettings {
    pub enabled: bool,
    /// The style of a keyboard with no entry in `devices`.
    pub default_style: KeyboardStyle,
    /// Per-keyboard choices, keyed by [`crate::device::DeviceId`].
    pub devices: Vec<KeyboardDeviceSettings>,
    pub groups: ShortcutGroups,
    /// The user's own app classes, on top of the built-in lists in
    /// [`crate::context`].
    pub app_rules: Vec<AppRule>,
    /// Keys a newer version wrote in this section, kept so this one does not
    /// delete them.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

impl Default for KeyboardSettings {
    fn default() -> Self {
        KeyboardSettings {
            enabled: true,
            default_style: KeyboardStyle::Windows,
            devices: Vec::new(),
            groups: ShortcutGroups::all(true),
            app_rules: Vec::new(),
            unknown: Map::new(),
        }
    }
}

/// One keyboard's choices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KeyboardDeviceSettings {
    pub id: String,
    /// The product name when the entry was made, for a keyboard that is not
    /// connected now.
    pub name: String,
    pub style: KeyboardStyle,
    pub layout: LayoutChoice,
    pub function_keys: FunctionKeys,
    /// Keys a newer version wrote for this keyboard, kept so a rewrite by
    /// this one (a preset from the CLI, say) does not delete them.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

impl Default for KeyboardDeviceSettings {
    fn default() -> Self {
        KeyboardDeviceSettings {
            id: String::new(),
            name: String::new(),
            style: KeyboardStyle::Windows,
            layout: LayoutChoice::Auto,
            function_keys: FunctionKeys::Standard,
            unknown: Map::new(),
        }
    }
}

/// The shortcut groups the event tap translates. Ctrl acting as Command is
/// not a group: it comes with the Windows style, together with the guards
/// that keep Ctrl+Q, Ctrl+H, Ctrl+Tab and Ctrl+Alt chords from becoming
/// macOS Command shortcuts.
///
/// A `Copy` value compared against [`ShortcutGroups::all`], so it keeps no
/// unknown keys: a later group belongs in [`KeyboardSettings`], which does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShortcutGroups {
    /// Ctrl+Y redo, Ctrl+H replace, Ctrl+F4 close, F3 find next,
    /// Ctrl+PageUp/PageDown tabs, Ctrl/Shift+Insert.
    pub editing: bool,
    /// Home/End (and Shift+Home/End) by line instead of by document. The
    /// Ctrl+arrow, Ctrl+Home/End and Ctrl+Backspace/Delete chords are guards
    /// that come with the Windows style: left alone, they would mean
    /// Command+arrow (Back in browsers) or switch Spaces.
    pub navigation: bool,
    /// Alt+Tab, Alt+F4, Win+D/E/L/Tab/arrows/I/R/period.
    pub windows: bool,
    /// Tapping Win alone opens search, like the Start menu.
    pub start_menu: bool,
    /// Ctrl+Shift+Esc, Ctrl+Alt+Delete, PrintScreen, Win+Shift+S.
    pub system: bool,
    /// Ctrl+Space and Win+Space switch input sources.
    pub input_source: bool,
    /// Fixes for browsers, VS Code, Office and Finder.
    pub app_fixes: bool,
    /// Ctrl+Shift+C/V and friends in terminals, where plain Ctrl stays Ctrl.
    pub terminal: bool,
}

impl ShortcutGroups {
    pub const fn all(on: bool) -> Self {
        ShortcutGroups {
            editing: on,
            navigation: on,
            windows: on,
            start_menu: on,
            system: on,
            input_source: on,
            app_fixes: on,
            terminal: on,
        }
    }
}

impl Default for ShortcutGroups {
    fn default() -> Self {
        ShortcutGroups::all(true)
    }
}

/// The user's class for one app, which wins over the built-in lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppRule {
    /// A bundle identifier, or a prefix ending in `*`.
    pub bundle_id: String,
    pub class: AppClass,
    /// Keys a newer version wrote for this rule, kept so this one does not
    /// delete them.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

impl Default for AppRule {
    fn default() -> Self {
        AppRule {
            bundle_id: String::new(),
            class: AppClass::Standard,
            unknown: Map::new(),
        }
    }
}

/// The mouse section. Trackpads and the Magic Mouse are never changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MouseSettings {
    pub enabled: bool,
    /// The profile of a wheel mouse with no entry in `devices`.
    pub defaults: MouseProfile,
    /// Per-mouse profiles, keyed by [`crate::device::DeviceId`].
    pub devices: Vec<MouseDeviceSettings>,
    /// Keys a newer version wrote in this section, kept so this one does not
    /// delete them.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

impl Default for MouseSettings {
    fn default() -> Self {
        MouseSettings {
            enabled: true,
            defaults: MouseProfile::WINDOWS,
            devices: Vec::new(),
            unknown: Map::new(),
        }
    }
}

/// One mouse's own profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MouseDeviceSettings {
    pub id: String,
    pub name: String,
    pub profile: MouseProfile,
    /// Keys a newer version wrote for this mouse, kept so a rewrite by this
    /// one does not delete them.
    #[serde(flatten)]
    pub unknown: Map<String, Value>,
}

impl Default for MouseDeviceSettings {
    fn default() -> Self {
        MouseDeviceSettings {
            id: String::new(),
            name: String::new(),
            profile: MouseProfile::WINDOWS,
            unknown: Map::new(),
        }
    }
}

/// How the pointer moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PointerMode {
    /// Windows with "Enhance pointer precision" off: every count of the
    /// mouse moves the pointer the same distance, scaled by the speed.
    Windows,
    /// Left as macOS has it.
    #[serde(other)]
    System,
}

/// What a wheel mouse does.
///
/// A `Copy` value compared against [`MouseProfile::WINDOWS`] and
/// [`MouseProfile::MAC`], so it keeps no unknown keys: a later setting
/// belongs next to a profile, in [`MouseSettings`] or
/// [`MouseDeviceSettings`], which do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MouseProfile {
    /// Rolling the wheel toward you scrolls down, whatever natural scrolling
    /// says for the trackpad.
    pub reverse_wheel: bool,
    /// Each notch scrolls `lines_per_notch` lines, without acceleration.
    pub linear_wheel: bool,
    pub lines_per_notch: u8,
    pub pointer: PointerMode,
    /// 1–20, Windows' own scale; 10 is Windows' default.
    pub pointer_speed: u8,
    /// Buttons 4 and 5 go back and forward in every app.
    pub side_buttons: bool,
    /// Ctrl+wheel zooms, one step per notch.
    pub ctrl_wheel_zoom: bool,
}

impl MouseProfile {
    pub const WINDOWS: MouseProfile = MouseProfile {
        reverse_wheel: true,
        linear_wheel: true,
        lines_per_notch: WINDOWS_DEFAULT_LINES_PER_NOTCH,
        pointer: PointerMode::Windows,
        pointer_speed: WINDOWS_DEFAULT_POINTER_SPEED,
        side_buttons: true,
        ctrl_wheel_zoom: true,
    };

    pub const MAC: MouseProfile = MouseProfile {
        reverse_wheel: false,
        linear_wheel: false,
        lines_per_notch: WINDOWS_DEFAULT_LINES_PER_NOTCH,
        pointer: PointerMode::System,
        pointer_speed: WINDOWS_DEFAULT_POINTER_SPEED,
        side_buttons: false,
        ctrl_wheel_zoom: false,
    };

    /// Which preset these values are, if any. The numbers count only when
    /// the switch they tune is on.
    pub fn style(&self) -> ProfileStyle {
        let MouseProfile {
            reverse_wheel,
            linear_wheel,
            lines_per_notch,
            pointer,
            pointer_speed,
            side_buttons,
            ctrl_wheel_zoom,
        } = *self;
        let windows = reverse_wheel
            && linear_wheel
            && lines_per_notch == WINDOWS_DEFAULT_LINES_PER_NOTCH
            && pointer == PointerMode::Windows
            && pointer_speed == WINDOWS_DEFAULT_POINTER_SPEED
            && side_buttons
            && ctrl_wheel_zoom;
        let mac = !reverse_wheel
            && !linear_wheel
            && pointer == PointerMode::System
            && !side_buttons
            && !ctrl_wheel_zoom;
        match (windows, mac) {
            (true, _) => ProfileStyle::Windows,
            (false, true) => ProfileStyle::Mac,
            (false, false) => ProfileStyle::Custom,
        }
    }

    /// Clamped into the ranges the engine accepts.
    pub fn normalized(self) -> Self {
        MouseProfile {
            lines_per_notch: clamp(self.lines_per_notch, LINES_PER_NOTCH_RANGE),
            pointer_speed: clamp(self.pointer_speed, POINTER_SPEED_RANGE),
            ..self
        }
    }
}

impl Default for MouseProfile {
    fn default() -> Self {
        MouseProfile::WINDOWS
    }
}

/// A preset name, or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProfileStyle {
    Windows,
    Mac,
    Custom,
}

/// A one-click preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Preset {
    Windows,
    Mac,
}

/// What a preset is applied to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PresetTarget {
    All,
    Keyboard,
    Mouse,
}

impl Settings {
    /// Applies `preset` to `target`. Windows also switches Isthmus on;
    /// Mac on everything leaves it on but changing nothing.
    pub fn apply_preset(&mut self, preset: Preset, target: PresetTarget) {
        let (keyboard, mouse) = match target {
            PresetTarget::All => (true, true),
            PresetTarget::Keyboard => (true, false),
            PresetTarget::Mouse => (false, true),
        };
        if keyboard {
            self.keyboard.apply_preset(preset);
        }
        if mouse {
            self.mouse.apply_preset(preset);
        }
        if preset == Preset::Windows {
            self.enabled = true;
        }
    }

    /// The same settings with every number in range, device lists without
    /// duplicates or empty ids, and app rules without blanks. Saving and
    /// applying always go through this.
    pub fn normalized(mut self) -> Self {
        self.version = SETTINGS_VERSION;
        self.keyboard.devices.retain(|d| !d.id.trim().is_empty());
        dedup_by_key(&mut self.keyboard.devices, |d| d.id.clone());
        for rule in &mut self.keyboard.app_rules {
            rule.bundle_id = rule.bundle_id.trim().to_owned();
        }
        self.keyboard.app_rules.retain(|r| !r.bundle_id.is_empty());
        dedup_by_key(&mut self.keyboard.app_rules, |r| r.bundle_id.clone());
        self.mouse.defaults = self.mouse.defaults.normalized();
        self.mouse.devices.retain(|d| !d.id.trim().is_empty());
        dedup_by_key(&mut self.mouse.devices, |d| d.id.clone());
        for device in &mut self.mouse.devices {
            device.profile = device.profile.normalized();
        }
        self
    }

    /// Reads a settings file's text. Anything unreadable is an error the
    /// caller reports; it never silently becomes the defaults.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str::<Settings>(text).map(Settings::normalized)
    }

    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

impl KeyboardSettings {
    /// The style a keyboard gets: its own entry, else the default.
    pub fn style_of(&self, id: &str) -> KeyboardStyle {
        self.device(id).map_or(self.default_style, |d| d.style)
    }

    pub fn device(&self, id: &str) -> Option<&KeyboardDeviceSettings> {
        self.devices.iter().find(|d| d.id == id)
    }

    /// Windows: every keyboard Windows, every group on. Mac: every keyboard
    /// Mac. Layout overrides and app rules are the user's facts, not style,
    /// so they stay.
    pub fn apply_preset(&mut self, preset: Preset) {
        let style = match preset {
            Preset::Windows => KeyboardStyle::Windows,
            Preset::Mac => KeyboardStyle::Mac,
        };
        self.enabled = true;
        self.default_style = style;
        for device in &mut self.devices {
            device.style = style;
            device.function_keys = match preset {
                Preset::Windows => FunctionKeys::Standard,
                Preset::Mac => FunctionKeys::System,
            };
        }
        if preset == Preset::Windows {
            self.groups = ShortcutGroups::all(true);
        }
    }

    /// Windows when every keyboard and every group is; Mac when every
    /// keyboard is; Custom otherwise.
    pub fn style(&self) -> ProfileStyle {
        let styles =
            std::iter::once(self.default_style).chain(self.devices.iter().map(|d| d.style));
        let all_windows = styles.clone().all(|s| s == KeyboardStyle::Windows);
        let all_mac = styles.into_iter().all(|s| s == KeyboardStyle::Mac);
        if all_windows && self.groups == ShortcutGroups::all(true) {
            ProfileStyle::Windows
        } else if all_mac {
            ProfileStyle::Mac
        } else {
            ProfileStyle::Custom
        }
    }
}

impl MouseSettings {
    /// The profile a mouse gets: its own entry, else the defaults.
    pub fn profile_of(&self, id: &str) -> MouseProfile {
        self.devices
            .iter()
            .find(|d| d.id == id)
            .map_or(self.defaults, |d| d.profile)
    }

    /// The preset becomes the defaults, and per-mouse entries are dropped so
    /// every mouse follows it.
    pub fn apply_preset(&mut self, preset: Preset) {
        self.enabled = true;
        self.defaults = match preset {
            Preset::Windows => MouseProfile::WINDOWS,
            Preset::Mac => MouseProfile::MAC,
        };
        self.devices.clear();
    }

    pub fn style(&self) -> ProfileStyle {
        let first = self.defaults.style();
        if self.devices.iter().all(|d| d.profile.style() == first) {
            first
        } else {
            ProfileStyle::Custom
        }
    }
}

fn clamp(value: u8, range: std::ops::RangeInclusive<u8>) -> u8 {
    value.clamp(*range.start(), *range.end())
}

/// Keeps the first of each key, in order.
fn dedup_by_key<T, K: Eq + std::hash::Hash>(items: &mut Vec<T>, key: impl Fn(&T) -> K) {
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(key(item)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file from a newer version keeps its extra keys through a rewrite,
    /// and an enum value this version does not know changes nothing.
    #[test]
    fn newer_files_survive_a_round_trip() {
        let text = r#"{
            "version": 7, "enabled": true, "futureThing": {"a": 1},
            "keyboard": {"defaultStyle": "linux", "futureKey": 3},
            "mouse": {"defaults": {"pointer": "warp", "linesPerNotch": 99}}
        }"#;
        let settings = Settings::from_json(text).unwrap();
        assert_eq!(settings.version, SETTINGS_VERSION);
        assert_eq!(settings.keyboard.default_style, KeyboardStyle::Mac);
        assert_eq!(settings.mouse.defaults.pointer, PointerMode::System);
        assert_eq!(settings.mouse.defaults.lines_per_notch, 12);
        let again = settings.to_json_pretty().unwrap();
        assert!(again.contains("futureThing"));
        assert!(again.contains("futureKey"));
    }

    /// Keys a newer version wrote inside a keyboard entry, a mouse entry and
    /// an app rule stay in that entry when this version rewrites the file
    /// (here after `isthmus preset mac --target keyboard`), and reading the
    /// rewrite back gives the same settings.
    #[test]
    fn unknown_keys_in_device_entries_and_app_rules_survive_a_rewrite() {
        let text = r#"{
            "keyboard": {
                "devices": [{"id": "kbd:1", "layout": "pc", "futureKeyboardKey": [1, 2]}],
                "appRules": [{"bundleId": "com.example.vm", "class": "passthrough", "futureRuleKey": "x"}]
            },
            "mouse": {
                "devices": [{"id": "mouse:1", "profile": {"pointerSpeed": 4}, "futureMouseKey": {"on": true}}]
            }
        }"#;
        let mut settings = Settings::from_json(text).unwrap();
        assert_eq!(settings.keyboard.devices[0].layout, LayoutChoice::Pc);
        assert_eq!(settings.keyboard.devices[0].unknown.len(), 1);
        assert_eq!(settings.keyboard.app_rules[0].class, AppClass::Passthrough);
        assert_eq!(settings.keyboard.app_rules[0].unknown.len(), 1);
        assert_eq!(settings.mouse.devices[0].profile.pointer_speed, 4);
        assert_eq!(settings.mouse.devices[0].unknown.len(), 1);

        settings.apply_preset(Preset::Mac, PresetTarget::Keyboard);
        let rewritten = settings.to_json_pretty().unwrap();

        let value: Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(
            value.pointer("/keyboard/devices/0/futureKeyboardKey"),
            Some(&serde_json::json!([1, 2]))
        );
        assert_eq!(
            value.pointer("/keyboard/devices/0/style"),
            Some(&serde_json::json!("mac"))
        );
        assert_eq!(
            value.pointer("/keyboard/appRules/0/futureRuleKey"),
            Some(&serde_json::json!("x"))
        );
        assert_eq!(
            value.pointer("/mouse/devices/0/futureMouseKey"),
            Some(&serde_json::json!({"on": true}))
        );
        assert_eq!(Settings::from_json(&rewritten).unwrap(), settings);
    }

    /// A new or emptied file is Windows habits, ready but switched off.
    #[test]
    fn an_empty_file_is_windows_habits_switched_off() {
        let settings = Settings::from_json("{}").unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.keyboard.style(), ProfileStyle::Windows);
        assert_eq!(settings.mouse.style(), ProfileStyle::Windows);
    }

    /// A preset writes values; changing one value afterwards reads as Custom,
    /// and the Windows preset on everything switches Isthmus on.
    #[test]
    fn presets_are_values_and_custom_is_what_matches_neither() {
        let mut settings = Settings::default();
        settings.apply_preset(Preset::Mac, PresetTarget::Mouse);
        assert_eq!(settings.mouse.style(), ProfileStyle::Mac);
        assert_eq!(settings.keyboard.style(), ProfileStyle::Windows);
        settings.mouse.defaults.reverse_wheel = true;
        assert_eq!(settings.mouse.style(), ProfileStyle::Custom);
        settings.apply_preset(Preset::Windows, PresetTarget::All);
        assert!(settings.enabled);
        assert_eq!(settings.mouse.style(), ProfileStyle::Windows);
    }

    /// Device entries without an id, repeated ids and blank app rules are
    /// dropped on the way in.
    #[test]
    fn normalizing_drops_blank_and_repeated_entries() {
        let mut settings = Settings::default();
        let kb = KeyboardDeviceSettings {
            id: "kbd:1".into(),
            ..Default::default()
        };
        settings.keyboard.devices = vec![kb.clone(), kb, KeyboardDeviceSettings::default()];
        settings.keyboard.app_rules = vec![AppRule {
            bundle_id: "  ".into(),
            class: AppClass::Terminal,
            ..AppRule::default()
        }];
        let settings = settings.normalized();
        assert_eq!(settings.keyboard.devices.len(), 1);
        assert!(settings.keyboard.app_rules.is_empty());
    }

    /// A file that is not valid settings is an error for the caller to
    /// report; it never quietly becomes the defaults (which would switch a
    /// user's choices back to Windows habits).
    #[test]
    fn unreadable_settings_are_an_error_not_the_defaults() {
        assert!(Settings::from_json("").is_err());
        assert!(Settings::from_json(r#"{"enabled": "yes"}"#).is_err());
    }

    /// An app class a newer version wrote reads as Standard, so one unknown
    /// rule does not make the whole file unreadable.
    #[test]
    fn an_unknown_app_class_reads_as_standard() {
        let text =
            r#"{"keyboard": {"appRules": [{"bundleId": "com.example.app", "class": "hologram"}]}}"#;
        let settings = Settings::from_json(text).unwrap();
        assert_eq!(settings.keyboard.app_rules[0].class, AppClass::Standard);
    }

    /// Per-device settings win over the defaults; a device without an entry
    /// follows the defaults.
    #[test]
    fn a_device_entry_wins_over_the_defaults() {
        let mut settings = Settings::default();
        settings.keyboard.devices.push(KeyboardDeviceSettings {
            id: "keyboard:usb:05ac:024f:loc=00112000".into(),
            style: KeyboardStyle::Mac,
            ..Default::default()
        });
        settings.mouse.devices.push(MouseDeviceSettings {
            id: "mouse:usb:046d:c54d:sn=386A344A3234".into(),
            profile: MouseProfile::MAC,
            ..Default::default()
        });
        let keyboard = &settings.keyboard;
        assert_eq!(
            keyboard.style_of("keyboard:usb:05ac:024f:loc=00112000"),
            KeyboardStyle::Mac
        );
        assert_eq!(
            keyboard.style_of("keyboard:usb:1a86:e010:loc=00111000"),
            KeyboardStyle::Windows
        );
        let mouse = &settings.mouse;
        assert_eq!(
            mouse.profile_of("mouse:usb:046d:c54d:sn=386A344A3234"),
            MouseProfile::MAC
        );
        assert_eq!(
            mouse.profile_of("mouse:usb:0000:0000:loc=00000000"),
            MouseProfile::WINDOWS
        );
    }

    /// The Mac keyboard preset changes every keyboard's style and F-row but
    /// keeps the user's facts: layout overrides, app rules and the groups.
    /// It does not switch Isthmus on.
    #[test]
    fn the_mac_keyboard_preset_keeps_layouts_rules_and_the_switch() {
        let mut settings = Settings::default();
        settings.keyboard.groups.terminal = false;
        settings.keyboard.devices.push(KeyboardDeviceSettings {
            id: "kbd".into(),
            layout: LayoutChoice::Pc,
            ..Default::default()
        });
        settings.keyboard.app_rules.push(AppRule {
            bundle_id: "com.example.vm".into(),
            class: AppClass::Passthrough,
            ..AppRule::default()
        });

        settings.apply_preset(Preset::Mac, PresetTarget::Keyboard);

        assert!(!settings.enabled);
        let device = &settings.keyboard.devices[0];
        assert_eq!(device.style, KeyboardStyle::Mac);
        assert_eq!(device.function_keys, FunctionKeys::System);
        assert_eq!(device.layout, LayoutChoice::Pc);
        assert_eq!(settings.keyboard.app_rules.len(), 1);
        assert!(!settings.keyboard.groups.terminal);
        assert_eq!(settings.keyboard.style(), ProfileStyle::Mac);
        assert_eq!(settings.mouse.style(), ProfileStyle::Windows);
    }

    /// A mouse preset becomes every mouse's profile: the per-mouse entries go.
    #[test]
    fn a_mouse_preset_drops_per_mouse_profiles() {
        let mut settings = Settings::default();
        settings.mouse.devices.push(MouseDeviceSettings {
            id: "mouse:1".into(),
            ..Default::default()
        });
        settings.apply_preset(Preset::Mac, PresetTarget::Mouse);
        assert!(settings.mouse.devices.is_empty());
        assert_eq!(settings.mouse.profile_of("mouse:1"), MouseProfile::MAC);
    }

    /// The numbers count only when their switch is on: a Mac profile with a
    /// changed wheel distance or speed is still Mac, a Windows profile with
    /// either changed is Custom.
    #[test]
    fn profile_style_ignores_numbers_whose_switch_is_off() {
        let mac = MouseProfile {
            lines_per_notch: 7,
            pointer_speed: 3,
            ..MouseProfile::MAC
        };
        assert_eq!(mac.style(), ProfileStyle::Mac);
        let faster = MouseProfile {
            pointer_speed: 12,
            ..MouseProfile::WINDOWS
        };
        assert_eq!(faster.style(), ProfileStyle::Custom);
    }

    /// Per-mouse profiles are clamped like the defaults, so the engine never
    /// sees a wheel distance or speed outside its range.
    #[test]
    fn normalizing_clamps_every_mouse_profile() {
        let mut settings = Settings::default();
        settings.mouse.devices.push(MouseDeviceSettings {
            id: "mouse:1".into(),
            profile: MouseProfile {
                lines_per_notch: 0,
                pointer_speed: 0,
                ..MouseProfile::WINDOWS
            },
            ..Default::default()
        });
        let settings = settings.normalized();
        let profile = settings.mouse.devices[0].profile;
        assert_eq!(profile.lines_per_notch, *LINES_PER_NOTCH_RANGE.start());
        assert_eq!(profile.pointer_speed, *POINTER_SPEED_RANGE.start());
    }
}
