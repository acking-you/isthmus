//! The settings file, read and written from the command line.
//!
//! The command line edits the file directly instead of asking a running
//! Isthmus: there is no channel to the app, and the file is the one place
//! both agree on. A missing file is the defaults (Windows habits, switched
//! off), as it is for the service. An unreadable or invalid one is an error
//! and is never replaced: overwriting it would silently throw away whatever
//! the user had, and the service would not have started from it either.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use isthmus_core::{durable_file, paths};
use isthmus_input::context::AppClass;
use isthmus_input::settings::{
    FunctionKeys, KeyboardStyle, LayoutChoice, MouseProfile, PointerMode, ProfileStyle,
    ShortcutGroups,
};
use isthmus_input::{Preset, PresetTarget, Settings};
use snafu::ResultExt;

use crate::error::{
    CliError, EncodeSettingsSnafu, ParseSettingsSnafu, ReadSettingsSnafu, SaveSettingsSnafu,
};
use crate::output::on_off;

/// The settings as read from the data directory.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SettingsFile {
    pub path: PathBuf,
    /// Normalized, as the service would apply them.
    pub settings: Settings,
    /// The file exists; otherwise `settings` are the defaults.
    pub exists: bool,
}

/// Reads `settings.json` in `dir`.
pub(crate) fn load(dir: &Path) -> Result<SettingsFile, CliError> {
    let path = paths::settings_file(dir);
    let (settings, exists) = match std::fs::read_to_string(&path) {
        Ok(text) => {
            let settings =
                Settings::from_json(&text).context(ParseSettingsSnafu { path: &path })?;
            (settings, true)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => (Settings::default(), false),
        Err(error) => return Err(error).context(ReadSettingsSnafu { path }),
    };
    Ok(SettingsFile {
        path,
        settings,
        exists,
    })
}

/// Replaces the file with `settings`, atomically, so a crash or a full
/// disk leaves the old file rather than half of the new one.
pub(crate) fn save(path: &Path, settings: &Settings) -> Result<(), CliError> {
    let mut text = settings.to_json_pretty().context(EncodeSettingsSnafu)?;
    text.push('\n');
    durable_file::replace_contents(path, text.as_bytes()).context(SaveSettingsSnafu { path })
}

/// Applies `preset` to `target` in the file in `dir` and saves it; nothing
/// is written when the file cannot be read.
pub(crate) fn apply_preset(
    dir: &Path,
    preset: Preset,
    target: PresetTarget,
) -> Result<SettingsFile, CliError> {
    let mut file = load(dir)?;
    let mut settings = file.settings;
    settings.apply_preset(preset, target);
    file.settings = settings.normalized();
    save(&file.path, &file.settings)?;
    Ok(file)
}

/// The settings for a person: the switches, which preset each section
/// matches, and every per-device entry and app rule.
pub(crate) fn describe(file: &SettingsFile) -> String {
    let settings = &file.settings;
    let mut lines = Vec::new();
    let origin = if file.exists {
        String::new()
    } else {
        " (no file yet: these are the defaults)".to_owned()
    };
    lines.push(format!("file      {}{origin}", file.path.display()));
    lines.push(format!("isthmus   {}", on_off(settings.enabled)));

    let keyboard = &settings.keyboard;
    lines.push(format!(
        "keyboard  {}, {}; keyboards without an entry: {}",
        on_off(keyboard.enabled),
        style_name(keyboard.style()),
        keyboard_style_name(keyboard.default_style)
    ));
    lines.push(format!(
        "  shortcut groups: {}",
        groups_text(&keyboard.groups)
    ));
    for device in &keyboard.devices {
        lines.push(format!(
            "  {} {}: {}, layout {}, F-keys {}",
            device.id,
            quoted(&device.name),
            keyboard_style_name(device.style),
            layout_choice_name(device.layout),
            function_keys_name(device.function_keys)
        ));
    }
    for rule in &keyboard.app_rules {
        lines.push(format!(
            "  app rule {} -> {}",
            rule.bundle_id,
            app_class_name(rule.class)
        ));
    }

    let mouse = &settings.mouse;
    lines.push(format!(
        "mouse     {}, {}",
        on_off(mouse.enabled),
        style_name(mouse.style())
    ));
    lines.push(format!(
        "  mice without an entry: {}",
        profile_text(&mouse.defaults)
    ));
    for device in &mouse.devices {
        lines.push(format!(
            "  {} {}: {}",
            device.id,
            quoted(&device.name),
            profile_text(&device.profile)
        ));
    }
    lines.iter().map(|line| format!("{line}\n")).collect()
}

fn quoted(name: &str) -> String {
    if name.is_empty() {
        String::new()
    } else {
        format!("\"{name}\"")
    }
}

fn style_name(style: ProfileStyle) -> &'static str {
    match style {
        ProfileStyle::Windows => "Windows preset",
        ProfileStyle::Mac => "Mac preset",
        ProfileStyle::Custom => "custom",
    }
}

fn keyboard_style_name(style: KeyboardStyle) -> &'static str {
    match style {
        KeyboardStyle::Windows => "Windows",
        KeyboardStyle::Mac => "Mac",
    }
}

fn layout_choice_name(layout: LayoutChoice) -> &'static str {
    match layout {
        LayoutChoice::Mac => "mac",
        LayoutChoice::Pc => "pc",
        LayoutChoice::Auto => "auto",
    }
}

fn function_keys_name(keys: FunctionKeys) -> &'static str {
    match keys {
        FunctionKeys::Standard => "standard",
        FunctionKeys::System => "as System Settings says",
    }
}

fn app_class_name(class: AppClass) -> &'static str {
    match class {
        AppClass::Terminal => "terminal",
        AppClass::NativeCtrl => "native Ctrl",
        AppClass::Passthrough => "passthrough",
        AppClass::Standard => "standard",
    }
}

/// The groups that are on, or "none".
fn groups_text(groups: &ShortcutGroups) -> String {
    let ShortcutGroups {
        editing,
        navigation,
        windows,
        start_menu,
        system,
        input_source,
        app_fixes,
        terminal,
    } = *groups;
    let on: Vec<&str> = [
        (editing, "editing"),
        (navigation, "navigation"),
        (windows, "windows"),
        (start_menu, "start menu"),
        (system, "system"),
        (input_source, "input source"),
        (app_fixes, "app fixes"),
        (terminal, "terminal"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect();
    if on.is_empty() {
        "none".to_owned()
    } else {
        on.join(", ")
    }
}

fn profile_text(profile: &MouseProfile) -> String {
    let MouseProfile {
        reverse_wheel,
        linear_wheel,
        lines_per_notch,
        pointer,
        pointer_speed,
        side_buttons,
        ctrl_wheel_zoom,
    } = *profile;
    let wheel = if linear_wheel {
        format!("linear, {lines_per_notch} lines per notch")
    } else {
        "accelerated".to_owned()
    };
    let pointer = match pointer {
        PointerMode::Windows => format!("Windows speed {pointer_speed}"),
        PointerMode::System => "as macOS has it".to_owned(),
    };
    format!(
        "wheel {} {wheel}; pointer {pointer}; side buttons {}; Ctrl+wheel zoom {}",
        if reverse_wheel {
            "Windows direction,"
        } else {
            "macOS direction,"
        },
        on_off(side_buttons),
        on_off(ctrl_wheel_zoom)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    /// No file yet is not an error: the service starts from the defaults,
    /// Windows habits switched off, and so does the command line.
    #[test]
    fn a_missing_file_reads_as_the_defaults() {
        let dir = TempDir::new("missing");
        let file = load(dir.path()).unwrap();
        assert!(!file.exists);
        assert_eq!(file.settings, Settings::default());
        assert_eq!(file.path, paths::settings_file(dir.path()));
    }

    /// What is on disk is read normalized, as the service applies it.
    #[test]
    fn an_existing_file_is_read_normalized() {
        let dir = TempDir::new("existing");
        let text = r#"{"enabled": true, "keyboard": {"defaultStyle": "mac"},
                      "mouse": {"defaults": {"linesPerNotch": 200}}}"#;
        std::fs::write(paths::settings_file(dir.path()), text).unwrap();
        let file = load(dir.path()).unwrap();
        assert!(file.exists);
        assert!(file.settings.enabled);
        assert_eq!(file.settings.keyboard.default_style, KeyboardStyle::Mac);
        assert_eq!(file.settings.mouse.defaults.lines_per_notch, 12);
    }

    /// An invalid file stops `preset` before anything is written, so the
    /// user's file is never replaced by defaults.
    #[test]
    fn a_preset_refuses_an_invalid_file_and_leaves_it_alone() {
        let dir = TempDir::new("invalid");
        let path = paths::settings_file(dir.path());
        std::fs::write(&path, "{ not json").unwrap();
        let error = apply_preset(dir.path(), Preset::Windows, PresetTarget::All).unwrap_err();
        assert!(matches!(error, CliError::ParseSettings { .. }), "{error:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }

    /// A file that exists but cannot be read is an error of its own, not the
    /// defaults.
    #[test]
    fn an_unreadable_file_is_an_error_not_the_defaults() {
        let dir = TempDir::new("unreadable");
        std::fs::create_dir(paths::settings_file(dir.path())).unwrap();
        let error = load(dir.path()).unwrap_err();
        assert!(matches!(error, CliError::ReadSettings { .. }), "{error:?}");
    }
}
