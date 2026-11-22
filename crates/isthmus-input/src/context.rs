//! Which app is in front, and what that means for the keyboard.
//!
//! Every app falls into one [`AppClass`], which decides both the HID mapping
//! the keyboards get while it is frontmost ([`crate::hidmap::HidProfile`])
//! and which shortcut groups the event tap translates. [`AppTraits`] add the
//! per-app fixes (browsers, VS Code, Office, Finder) on top of the standard
//! class. The user's [`AppRule`]s win over the built-in lists here.

use serde::{Deserialize, Serialize};

use crate::settings::AppRule;

/// How an app is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppClass {
    /// Terminals: Ctrl stays Ctrl (Ctrl+C interrupts), Ctrl+Shift+C/V copy
    /// and paste, window and system shortcuts still work.
    Terminal,
    /// Apps whose own Windows keymap uses Ctrl, such as JetBrains IDEs with
    /// their Windows keymap, Emacs and Vim: Ctrl stays Ctrl, and only window
    /// and system shortcuts are translated.
    NativeCtrl,
    /// Virtual machines and remote desktops: keys go through as the keyboard
    /// has them (positions only), and nothing is translated, so the Windows
    /// on the other side sees Ctrl, Win and Alt.
    Passthrough,
    /// Windows habits in full: Ctrl acts as Command. Also what a class this
    /// build does not know reads as.
    #[default]
    #[serde(other)]
    Standard,
}

/// Per-app fixes inside [`AppClass::Standard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppTraits {
    /// A web browser: F5 reloads, Alt+arrows go back and forward, Home/End
    /// are left alone (Command+Left is Back there outside a text field).
    pub browser: bool,
    /// Handles mouse buttons 4 and 5 itself, so they pass through untouched
    /// instead of becoming a back/forward swipe: Chrome, Edge, Brave, Arc,
    /// Vivaldi, Chromium, VS Code, VSCodium, Cursor, Antigravity, JetBrains
    /// IDEs and Android Studio ([`builtin::NATIVE_SIDE_BUTTONS`]). Every
    /// other app, Safari, Firefox and other Electron apps included, gets the
    /// swipe.
    pub native_side_buttons: bool,
    /// The VS Code family, whose macOS keymap differs from Ctrl→Command in
    /// known places.
    pub vscode: bool,
    /// Microsoft Office: redo is Command+Y, Ctrl+wheel stays Command+wheel.
    pub office: bool,
    /// Finder: F2 renames, Alt+F4 closes the window.
    pub finder: bool,
    /// Drags with the middle button itself (orbit, pan): Blender, Figma,
    /// CAD and game engines ([`builtin::NATIVE_MIDDLE_DRAG`]), where holding
    /// the middle button must not become scrolling.
    pub native_middle_drag: bool,
}

/// An app's class and traits together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppProfile {
    /// Decides the HID mapping and the shortcut groups.
    pub class: AppClass,
    /// The per-app fixes, whatever the class.
    pub traits: AppTraits,
}

/// The frontmost app as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FrontApp {
    /// `CFBundleIdentifier`; empty for a process without a bundle.
    pub bundle_id: String,
    /// The localized name the Dock shows.
    pub name: String,
    /// The process id, so the UI can tell two copies of one app apart.
    pub pid: i32,
    /// What [`classify`] made of it.
    pub profile: AppProfile,
}

/// Does `pattern` (a bundle id, or a prefix ending in `*`) match `bundle_id`?
/// Bundle ids compare without regard to case, as LaunchServices does.
pub fn pattern_matches(pattern: &str, bundle_id: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => bundle_id
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix)),
        None => pattern.eq_ignore_ascii_case(bundle_id),
    }
}

/// The class and traits of `bundle_id`: the user's rules first, then the
/// built-in lists, then [`AppClass::Standard`]. Traits always come from the
/// built-in lists.
///
/// Among the user's rules the most specific match wins: an exact bundle id
/// over any prefix, a longer prefix over a shorter one, and the earlier rule
/// on a tie. The rules are a set the UI edits, not an ordered list, so
/// "`com.jetbrains.*` is standard, but GoLand keeps Ctrl" must work in
/// either order. The built-in lists are checked Passthrough first, so a
/// virtual machine is never translated even if a terminal list ever
/// overlapped it, then Terminal, then NativeCtrl.
pub fn classify(bundle_id: &str, user_rules: &[AppRule]) -> AppProfile {
    let class = user_class(bundle_id, user_rules)
        .or_else(|| builtin_class(bundle_id))
        .unwrap_or(AppClass::Standard);
    AppProfile {
        class,
        traits: builtin_traits(bundle_id),
    }
}

/// The class of the most specific user rule matching `bundle_id`, if any.
fn user_class(bundle_id: &str, user_rules: &[AppRule]) -> Option<AppClass> {
    user_rules
        .iter()
        .filter(|rule| pattern_matches(&rule.bundle_id, bundle_id))
        // `max_by_key` keeps the last of equals; reversing first makes the
        // earliest rule win a tie.
        .rev()
        .max_by_key(|rule| specificity(&rule.bundle_id))
        .map(|rule| rule.class)
}

/// How specific a pattern is, for ordering matching rules: any exact id
/// beats any prefix, and a longer prefix beats a shorter one.
fn specificity(pattern: &str) -> (bool, usize) {
    match pattern.strip_suffix('*') {
        Some(prefix) => (false, prefix.len()),
        None => (true, pattern.len()),
    }
}

/// The class the built-in lists give `bundle_id`, if any.
fn builtin_class(bundle_id: &str) -> Option<AppClass> {
    [
        (builtin::PASSTHROUGH, AppClass::Passthrough),
        (builtin::TERMINAL, AppClass::Terminal),
        (builtin::NATIVE_CTRL, AppClass::NativeCtrl),
    ]
    .into_iter()
    .find(|(list, _)| in_list(list, bundle_id))
    .map(|(_, class)| class)
}

/// The per-app fixes the built-in lists give `bundle_id`. They describe what
/// the app is, which a user's class does not change: a browser the user
/// passes through still handles its own side buttons.
fn builtin_traits(bundle_id: &str) -> AppTraits {
    AppTraits {
        browser: in_list(builtin::BROWSER, bundle_id),
        native_side_buttons: in_list(builtin::NATIVE_SIDE_BUTTONS, bundle_id),
        vscode: in_list(builtin::VSCODE, bundle_id),
        office: in_list(builtin::OFFICE, bundle_id),
        finder: in_list(builtin::FINDER, bundle_id),
        native_middle_drag: in_list(builtin::NATIVE_MIDDLE_DRAG, bundle_id),
    }
}

/// Does any pattern of `list` match `bundle_id`?
fn in_list(list: &[&str], bundle_id: &str) -> bool {
    list.iter()
        .any(|pattern| pattern_matches(pattern, bundle_id))
}

/// The built-in lists, verified bundle identifiers (checked against
/// Homebrew casks, installed apps' Info.plists and Karabiner-Elements'
/// shared lists; see docs/keyboard-and-mouse.md, "App classes"). A trailing
/// `*` matches a prefix.
pub mod builtin {
    /// Terminals.
    pub const TERMINAL: &[&str] = &[
        "com.apple.Terminal",
        "com.googlecode.iterm2",
        "dev.warp.Warp*",
        "org.alacritty",
        "io.alacritty",
        "net.kovidgoyal.kitty",
        "com.github.wez.wezterm",
        "com.mitchellh.ghostty",
        "co.zeit.hyper",
        "co.zeit.hyperterm",
        "org.tabby",
        "com.termius-dmg.mac",
        "com.raphaelamorim.rio",
    ];

    /// Apps that keep Ctrl as Ctrl: JetBrains IDEs (use their Windows
    /// keymap), Android Studio, Emacs, Vim, X11.
    pub const NATIVE_CTRL: &[&str] = &[
        "com.jetbrains.*",
        "com.google.android.studio",
        "org.gnu.Emacs",
        "org.vim.*",
        "com.qvacua.VimR",
        "org.macosforge.xquartz.X11",
        "org.macports.X11",
    ];

    /// Virtual machines and remote desktops. Apple's Screen Sharing is not
    /// here: the other side is a Mac, so local translation is right there.
    pub const PASSTHROUGH: &[&str] = &[
        "com.parallels.desktop.console",
        "com.parallels.vm",
        "com.parallels.winapp.*",
        "com.vmware.fusion",
        "com.vmware.horizon",
        "com.vmware.view",
        "com.vmware.proxyApp.*",
        "com.omnissa.horizon.client.mac",
        "com.utmapp.UTM",
        "org.virtualbox.app.VirtualBoxVM",
        "com.microsoft.rdc.macos",
        "com.microsoft.rdc",
        "com.microsoft.rdc.mac",
        "com.microsoft.rdc.osx.beta",
        "com.microsoft.windowsapp",
        "com.p5sys.jump.mac.viewer*",
        "com.jumpdesktop.JumpDesktop",
        "com.carriez.rustdesk",
        "com.teamviewer.TeamViewer",
        "com.philandro.anydesk",
        "tv.parsec.www",
        "com.moonlight-stream.Moonlight",
        "com.citrix.receiver.icaviewer*",
        "com.citrix.XenAppViewer",
        "com.nomachine.nxplayer",
        "com.lemonmojo.RoyalTSX.App",
        "com.devolutions.remotedesktopmanager",
        "com.splashtop.stb.macosx",
        "com.realvnc.vncviewer",
        "com.realvnc.rvncconnect",
        "com.youqu.todesk.mac",
        "com.oray.*",
        "com.codeweavers.CrossOver",
        "com.isaacmarovitz.Whisky",
    ];

    /// Browsers.
    pub const BROWSER: &[&str] = &[
        "com.google.Chrome",
        "com.google.Chrome.beta",
        "com.google.Chrome.canary",
        "com.google.Chrome.dev",
        "com.microsoft.edgemac*",
        "com.apple.Safari",
        "com.apple.SafariTechnologyPreview",
        "org.mozilla.firefox",
        "org.mozilla.firefoxdeveloperedition",
        "org.mozilla.nightly",
        "com.brave.Browser*",
        "company.thebrowser.Browser",
        "com.vivaldi.Vivaldi",
        "com.operasoftware.Opera",
        "org.chromium.Chromium",
    ];

    /// Apps that handle mouse buttons 4 and 5 themselves.
    pub const NATIVE_SIDE_BUTTONS: &[&str] = &[
        "com.google.Chrome*",
        "com.microsoft.edgemac*",
        "com.brave.Browser*",
        "company.thebrowser.Browser",
        "com.vivaldi.Vivaldi",
        "org.chromium.Chromium",
        "com.microsoft.VSCode*",
        "com.vscodium",
        "com.todesktop.230313mzl4w4u92",
        "com.google.antigravity",
        "com.jetbrains.*",
        "com.google.android.studio",
    ];

    /// Apps that orbit, pan or draw with the middle button held.
    pub const NATIVE_MIDDLE_DRAG: &[&str] = &[
        "org.blenderfoundation.blender",
        "com.figma.Desktop",
        "com.bohemiancoding.sketch3",
        "com.autodesk.*",
        "com.sketchup.*",
        "com.mcneel.rhinoceros*",
        "org.freecad.FreeCAD",
        "org.freecadweb.FreeCAD",
        "net.maxon.cinema4d",
        "com.unity3d.*",
        "com.epicgames.*",
        "org.godotengine.godot",
        "org.kicad.*",
        "com.seriflabs.*",
        "org.krita",
        "org.gimp.gimp-*",
        "org.inkscape.Inkscape",
    ];

    /// The VS Code family.
    pub const VSCODE: &[&str] = &[
        "com.microsoft.VSCode",
        "com.microsoft.VSCodeInsiders",
        "com.vscodium",
        "com.todesktop.230313mzl4w4u92",
        "com.google.antigravity",
        "com.trae.app",
        "dev.kiro.desktop",
        "com.exafunction.windsurf",
    ];

    /// Microsoft Office.
    pub const OFFICE: &[&str] = &[
        "com.microsoft.Word",
        "com.microsoft.Excel",
        "com.microsoft.Powerpoint",
        "com.microsoft.Outlook",
    ];

    /// Finder.
    pub const FINDER: &[&str] = &["com.apple.finder"];
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rule for one app, as the settings hold it.
    fn rule(bundle_id: &str, class: AppClass) -> AppRule {
        AppRule {
            bundle_id: bundle_id.to_owned(),
            class,
            ..AppRule::default()
        }
    }

    /// A trailing `*` matches ids that start with the rest, and nothing
    /// else; a pattern without it matches one id exactly.
    #[test]
    fn prefix_patterns_match_ids_that_start_with_the_prefix() {
        assert!(pattern_matches("com.jetbrains.*", "com.jetbrains.goland"));
        assert!(pattern_matches("dev.warp.Warp*", "dev.warp.Warp-Stable"));
        assert!(!pattern_matches("com.jetbrains.*", "com.jetbrain"));
        assert!(!pattern_matches(
            "com.apple.Terminal",
            "com.apple.Terminal2"
        ));
        assert_eq!(
            classify("com.parallels.winapp.notepad", &[]).class,
            AppClass::Passthrough
        );
    }

    /// Bundle ids compare as LaunchServices compares them, ignoring case, in
    /// exact ids and prefixes alike; a prefix that would split a multi-byte
    /// character simply does not match.
    #[test]
    fn bundle_ids_match_without_regard_to_case() {
        assert_eq!(
            classify("COM.APPLE.TERMINAL", &[]).class,
            AppClass::Terminal
        );
        assert_eq!(
            classify("COM.JetBrains.PyCharm", &[]).class,
            AppClass::NativeCtrl
        );
        assert!(pattern_matches("COM.EXAMPLE.*", "com.example.app"));
        assert!(!pattern_matches("ab*", "a\u{e9}"));
    }

    /// Each built-in list gives its class, Apple's Screen Sharing and
    /// unknown apps are standard, and so is a process without a bundle id.
    #[test]
    fn builtin_lists_give_terminal_native_ctrl_and_passthrough() {
        assert_eq!(
            classify("com.googlecode.iterm2", &[]).class,
            AppClass::Terminal
        );
        assert_eq!(classify("org.gnu.Emacs", &[]).class, AppClass::NativeCtrl);
        assert_eq!(classify("com.utmapp.UTM", &[]).class, AppClass::Passthrough);
        assert_eq!(
            classify("com.apple.ScreenSharing", &[]).class,
            AppClass::Standard
        );
        assert_eq!(classify("com.example.unknown", &[]), AppProfile::default());
        assert_eq!(classify("", &[]), AppProfile::default());
    }

    /// The user's rule decides the class even where a built-in list says
    /// otherwise, in both directions.
    #[test]
    fn a_user_rule_wins_over_the_builtin_lists() {
        let rules = [
            rule("com.apple.Terminal", AppClass::Standard),
            rule("com.example.*", AppClass::Passthrough),
        ];
        assert_eq!(
            classify("com.apple.Terminal", &rules).class,
            AppClass::Standard
        );
        assert_eq!(
            classify("com.example.vm", &rules).class,
            AppClass::Passthrough
        );
        assert_eq!(
            classify("com.googlecode.iterm2", &rules).class,
            AppClass::Terminal
        );
    }

    /// Among matching user rules the most specific wins, whatever their
    /// order: an exact id over a prefix, a longer prefix over a shorter one,
    /// and the earlier of two equal patterns.
    #[test]
    fn the_most_specific_user_rule_wins_in_any_order() {
        let broad = rule("com.jetbrains.*", AppClass::Standard);
        let narrow = rule("com.jetbrains.goland*", AppClass::Terminal);
        let exact = rule("com.jetbrains.goland", AppClass::NativeCtrl);
        for rules in [
            vec![broad.clone(), narrow.clone(), exact.clone()],
            vec![exact.clone(), narrow.clone(), broad.clone()],
        ] {
            assert_eq!(
                classify("com.jetbrains.goland", &rules).class,
                AppClass::NativeCtrl
            );
            assert_eq!(
                classify("com.jetbrains.goland.eap", &rules).class,
                AppClass::Terminal
            );
            assert_eq!(
                classify("com.jetbrains.clion", &rules).class,
                AppClass::Standard
            );
        }
        let twins = [
            rule("com.example.app", AppClass::Terminal),
            rule("COM.EXAMPLE.APP", AppClass::Passthrough),
        ];
        assert_eq!(
            classify("com.example.app", &twins).class,
            AppClass::Terminal
        );
    }

    /// Traits describe the app, so a browser the user passes through still
    /// counts as a browser that handles its own side buttons.
    #[test]
    fn traits_stay_set_when_a_user_rule_changes_the_class() {
        let rules = [rule("com.google.Chrome", AppClass::Passthrough)];
        let profile = classify("com.google.Chrome", &rules);
        assert_eq!(profile.class, AppClass::Passthrough);
        assert!(profile.traits.browser);
        assert!(profile.traits.native_side_buttons);
    }

    /// Each trait comes from its own list: Safari is a browser that does not
    /// handle side buttons itself, JetBrains IDEs handle them without being
    /// browsers, and the VS Code, Office and Finder fixes are told apart.
    #[test]
    fn traits_come_from_their_own_lists() {
        let safari = classify("com.apple.Safari", &[]).traits;
        assert!(safari.browser && !safari.native_side_buttons);
        let idea = classify("com.jetbrains.intellij", &[]);
        assert_eq!(idea.class, AppClass::NativeCtrl);
        assert!(idea.traits.native_side_buttons && !idea.traits.browser);
        let code = classify("com.microsoft.VSCode", &[]).traits;
        assert!(code.vscode && code.native_side_buttons && !code.office);
        assert!(classify("com.microsoft.Word", &[]).traits.office);
        assert_eq!(
            classify("com.apple.finder", &[]).traits,
            AppTraits {
                finder: true,
                ..AppTraits::default()
            }
        );
    }
}
