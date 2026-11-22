import 'package:flutter/widgets.dart';
import 'package:isthmus_ui/l10n/app_localizations.dart';
import 'package:isthmus_ui/src/ffi/models.dart';
import 'package:isthmus_ui/src/theme/app_icons.dart';

/// The examples the keyboard page shows for each shortcut group, taken from
/// the shortcut table in `docs/keyboard-and-mouse.md`. They are examples, not
/// the engine's rules: the engine lives in Rust, and this list only tells a
/// person what to expect.
///
/// Key labels are printed as on the keys: Windows words on the left
/// ("Ctrl", "Win", "PrtSc"), macOS symbols on the right ("⌘", "⌥", "⌃", "⇧").
/// Words that need translating are read from [AppLocalizations].

/// Something said in the user's language.
typedef Words = String Function(AppLocalizations l10n);

/// One Windows shortcut and what it does on the Mac.
class ShortcutExample {
  const ShortcutExample(this.windows, {this.mac, required this.does, this.app});

  /// The Windows chord, key by key.
  final List<String> windows;

  /// The Mac chord it becomes; null when it becomes an action with no chord
  /// worth printing (Show Desktop, Mission Control).
  final List<String>? mac;

  /// What it does, in a few words.
  final Words does;

  /// The app it applies in, for the app fixes.
  final Words? app;
}

/// A card of examples on the keyboard page: a group the user can switch, or
/// (with a null [group]) behaviour that comes with the Windows style itself.
class ShortcutCard {
  const ShortcutCard({
    required this.id,
    required this.icon,
    required this.title,
    required this.summary,
    required this.examples,
    this.group,
  });

  /// The card's name in keys: `keyboard-group-<id>`.
  final String id;
  final ShortcutGroup? group;
  final IconData icon;
  final Words title;
  final Words summary;
  final List<ShortcutExample> examples;
}

String _browsers(AppLocalizations l) => l.ctxBrowsers;
String _vscode(AppLocalizations l) => l.ctxVsCode;
String _office(AppLocalizations l) => l.ctxOffice;
String _finder(AppLocalizations l) => l.ctxFinder;

/// The cards, top to bottom.
final List<ShortcutCard> shortcutCatalog = [
  ShortcutCard(
    id: 'ctrl',
    icon: AppIcons.shortcuts,
    title: (l) => l.groupCtrlTitle,
    summary: (l) => l.groupCtrlSummary,
    examples: [
      ShortcutExample(['Ctrl', 'C'], mac: ['⌘', 'C'], does: (l) => l.scCopy),
      ShortcutExample(['Ctrl', 'V'], mac: ['⌘', 'V'], does: (l) => l.scPaste),
      ShortcutExample(['Ctrl', 'Z'], mac: ['⌘', 'Z'], does: (l) => l.scUndo),
      ShortcutExample(['Ctrl', 'S'], mac: ['⌘', 'S'], does: (l) => l.scSave),
      ShortcutExample(['Ctrl', 'F'], mac: ['⌘', 'F'], does: (l) => l.scFind),
      ShortcutExample(
        ['Ctrl', 'W'],
        mac: ['⌘', 'W'],
        does: (l) => l.scCloseTab,
      ),
    ],
  ),
  ShortcutCard(
    id: 'guards',
    icon: AppIcons.guard,
    title: (l) => l.groupGuardsTitle,
    summary: (l) => l.groupGuardsSummary,
    examples: [
      ShortcutExample(
        ['Ctrl', 'Tab'],
        mac: ['⌃', 'Tab'],
        does: (l) => l.scNextTab,
      ),
      ShortcutExample(['Ctrl', 'Q'], mac: ['⌃', 'Q'], does: (l) => l.scNotQuit),
      ShortcutExample(['Ctrl', '←'], mac: ['⌥', '←'], does: (l) => l.scByWord),
      ShortcutExample(
        ['Ctrl', 'Home'],
        mac: ['⌘', '↑'],
        does: (l) => l.scDocStart,
      ),
      ShortcutExample(
        ['Ctrl', 'Backspace'],
        mac: ['⌥', '⌫'],
        does: (l) => l.scDeleteWord,
      ),
      ShortcutExample(
        ['Ctrl', 'Space'],
        mac: ['⌃', 'Space'],
        does: (l) => l.scPrevInputSource,
      ),
    ],
  ),
  ShortcutCard(
    id: 'editing',
    group: ShortcutGroup.editing,
    icon: AppIcons.editing,
    title: (l) => l.groupEditingTitle,
    summary: (l) => l.groupEditingSummary,
    examples: [
      ShortcutExample(
        ['Ctrl', 'Y'],
        mac: ['⌘', '⇧', 'Z'],
        does: (l) => l.scRedo,
      ),
      ShortcutExample(
        ['Ctrl', 'H'],
        mac: ['⌘', '⌥', 'F'],
        does: (l) => l.scReplace,
      ),
      ShortcutExample(['F3'], mac: ['⌘', 'G'], does: (l) => l.scFindNext),
      ShortcutExample(
        ['Ctrl', 'F4'],
        mac: ['⌘', 'W'],
        does: (l) => l.scCloseTab,
      ),
      ShortcutExample(
        ['Ctrl', 'PgDn'],
        mac: ['⌃', 'Tab'],
        does: (l) => l.scNextTab,
      ),
      ShortcutExample(
        ['Shift', 'Ins'],
        mac: ['⌘', 'V'],
        does: (l) => l.scPaste,
      ),
    ],
  ),
  ShortcutCard(
    id: 'navigation',
    group: ShortcutGroup.navigation,
    icon: AppIcons.navigation,
    title: (l) => l.groupNavigationTitle,
    summary: (l) => l.groupNavigationSummary,
    examples: [
      ShortcutExample(['Home'], mac: ['⌘', '←'], does: (l) => l.scLineStart),
      ShortcutExample(['End'], mac: ['⌘', '→'], does: (l) => l.scLineEnd),
      ShortcutExample(
        ['Shift', 'End'],
        mac: ['⌘', '⇧', '→'],
        does: (l) => l.scSelectToLineEnd,
      ),
    ],
  ),
  ShortcutCard(
    id: 'windows',
    group: ShortcutGroup.windows,
    icon: AppIcons.windows,
    title: (l) => l.groupWindowsTitle,
    summary: (l) => l.groupWindowsSummary,
    examples: [
      ShortcutExample(
        ['Alt', 'Tab'],
        mac: ['⌘', 'Tab'],
        does: (l) => l.scSwitchApps,
      ),
      ShortcutExample(
        ['Alt', 'F4'],
        mac: ['⌘', 'W'],
        does: (l) => l.scCloseWindow,
      ),
      ShortcutExample(['Win', 'D'], does: (l) => l.scShowDesktop),
      ShortcutExample(['Win', 'E'], does: (l) => l.scNewFinder),
      ShortcutExample(
        ['Win', 'L'],
        mac: ['⌃', '⌘', 'Q'],
        does: (l) => l.scLock,
      ),
      ShortcutExample(['Win', 'Tab'], does: (l) => l.scMissionControl),
      ShortcutExample(['Win', '←'], does: (l) => l.scTileLeft),
      ShortcutExample(['Win', '↑'], does: (l) => l.scFill),
      ShortcutExample(['Win', '↓'], mac: ['⌘', 'M'], does: (l) => l.scMinimize),
      ShortcutExample(['Win', 'Ctrl', '→'], does: (l) => l.scNextSpace),
      ShortcutExample(['Win', 'I'], does: (l) => l.scSystemSettings),
      ShortcutExample(
        ['Win', '.'],
        mac: ['⌃', '⌘', 'Space'],
        does: (l) => l.scEmoji,
      ),
    ],
  ),
  ShortcutCard(
    id: 'startMenu',
    group: ShortcutGroup.startMenu,
    icon: AppIcons.startMenu,
    title: (l) => l.groupStartMenuTitle,
    summary: (l) => l.groupStartMenuSummary,
    examples: [
      ShortcutExample(['Win'], mac: ['⌘', 'Space'], does: (l) => l.scSpotlight),
    ],
  ),
  ShortcutCard(
    id: 'system',
    group: ShortcutGroup.system,
    icon: AppIcons.system,
    title: (l) => l.groupSystemTitle,
    summary: (l) => l.groupSystemSummary,
    examples: [
      ShortcutExample([
        'Ctrl',
        'Shift',
        'Esc',
      ], does: (l) => l.scActivityMonitor),
      ShortcutExample(
        ['Ctrl', 'Alt', 'Del'],
        mac: ['⌘', '⌥', 'Esc'],
        does: (l) => l.scForceQuit,
      ),
      ShortcutExample(
        ['PrtSc'],
        mac: ['⌃', '⇧', '⌘', '4'],
        does: (l) => l.scAreaShot,
      ),
      ShortcutExample(
        ['Win', 'Shift', 'S'],
        mac: ['⌃', '⇧', '⌘', '4'],
        does: (l) => l.scAreaShot,
      ),
      ShortcutExample(
        ['Win', 'PrtSc'],
        mac: ['⇧', '⌘', '3'],
        does: (l) => l.scFullShot,
      ),
    ],
  ),
  ShortcutCard(
    id: 'inputSource',
    group: ShortcutGroup.inputSource,
    icon: AppIcons.inputSource,
    title: (l) => l.groupInputSourceTitle,
    summary: (l) => l.groupInputSourceSummary,
    examples: [
      ShortcutExample(['Win', 'Space'], does: (l) => l.scNextInputSource),
    ],
  ),
  ShortcutCard(
    id: 'appFixes',
    group: ShortcutGroup.appFixes,
    icon: AppIcons.appFixes,
    title: (l) => l.groupAppFixesTitle,
    summary: (l) => l.groupAppFixesSummary,
    examples: [
      ShortcutExample(
        ['F5'],
        mac: ['⌘', 'R'],
        does: (l) => l.scReload,
        app: _browsers,
      ),
      ShortcutExample(
        ['Alt', '←'],
        mac: ['⌘', '['],
        does: (l) => l.scBack,
        app: _browsers,
      ),
      ShortcutExample(
        ['F6'],
        mac: ['⌘', 'L'],
        does: (l) => l.scAddressBar,
        app: _browsers,
      ),
      ShortcutExample(
        ['F12'],
        mac: ['⌘', '⌥', 'I'],
        does: (l) => l.scDevTools,
        app: _browsers,
      ),
      ShortcutExample(
        ['Ctrl', 'G'],
        mac: ['⌃', 'G'],
        does: (l) => l.scGoToLine,
        app: _vscode,
      ),
      ShortcutExample(
        ['F12'],
        mac: ['⌘', '⇧', 'S'],
        does: (l) => l.scSaveAs,
        app: _office,
      ),
      ShortcutExample(
        ['F2'],
        mac: ['↩'],
        does: (l) => l.scRename,
        app: _finder,
      ),
      ShortcutExample(
        ['Alt', '↑'],
        mac: ['⌘', '↑'],
        does: (l) => l.scEnclosingFolder,
        app: _finder,
      ),
    ],
  ),
  ShortcutCard(
    id: 'terminal',
    group: ShortcutGroup.terminal,
    icon: AppIcons.terminal,
    title: (l) => l.groupTerminalTitle,
    summary: (l) => l.groupTerminalSummary,
    examples: [
      ShortcutExample(
        ['Ctrl', 'Shift', 'C'],
        mac: ['⌘', 'C'],
        does: (l) => l.scCopy,
      ),
      ShortcutExample(
        ['Ctrl', 'Shift', 'V'],
        mac: ['⌘', 'V'],
        does: (l) => l.scPaste,
      ),
      ShortcutExample(
        ['Ctrl', 'Shift', 'T'],
        mac: ['⌘', 'T'],
        does: (l) => l.scNewTab,
      ),
      ShortcutExample(['Ctrl', '='], mac: ['⌘', '='], does: (l) => l.scZoomIn),
      ShortcutExample(
        ['Ctrl', 'Backspace'],
        mac: ['⌃', 'W'],
        does: (l) => l.scDeleteWord,
      ),
      ShortcutExample(
        ['Ctrl', 'C'],
        mac: ['⌃', 'C'],
        does: (l) => l.scInterrupt,
      ),
    ],
  ),
];
