import 'dart:async';
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:tray_manager/tray_manager.dart';

/// What the menu bar item shows, words included: the tray is not a widget,
/// so it cannot read localizations, and the app hands it the current strings
/// with every change. A record, so an unchanged menu compares equal and is
/// not rebuilt.
typedef TrayView = ({
  // The disabled first line: on, off, or what is in the way.
  String status,
  // The "Windows habits" checkbox; empty leaves it out (a PC has none).
  String toggleLabel,
  bool enabled,
  String openLabel,
  String quitLabel,
  // Keyboards with a Mac/Windows switch, each a checkbox that is on in
  // Windows mode: after flipping the keyboard's switch, one click here.
  List<TrayKeyboardMode> keyboards,
});

/// One keyboard's mode checkbox in the menu.
typedef TrayKeyboardMode = ({String id, String label, bool windows});

/// The menu bar item: a template icon and a four-line menu.
///
/// A template image is black plus alpha, and macOS tints it to match the menu
/// bar: light in dark mode, dark in light mode, inverted while the item is
/// highlighted. The state therefore reads from the menu, not the icon.
class TrayService with TrayListener {
  TrayService._();

  /// The one item; the app owns its lifecycle.
  static final TrayService instance = TrayService._();

  /// The plugin reads the asset with `rootBundle.load`, which takes an exact
  /// key and never resolves a resolution variant, then pins the image to
  /// 18 pt. The 2x file fills those points on Retina, and macOS scales it
  /// down cleanly on a 1x display.
  ///
  /// Windows has no template images and wants an `.ico`; it gets the app's
  /// own colours, which read on a dark taskbar and a light one.
  static String get iconAsset => Platform.isWindows
      ? 'assets/tray/tray_icon.ico'
      : 'assets/tray/tray_icon@2x.png';

  final TrayManager _tray = TrayManager.instance;

  // Every item is kept and reused, so a click that arrives while the menu is
  // being replaced still resolves to the same action.
  final _statusItem = MenuItem(key: 'status', disabled: true);
  final _toggleItem = MenuItem.checkbox(key: 'toggle', checked: false);
  final _openItem = MenuItem(key: 'open');
  final _separator = MenuItem.separator();
  final _quitItem = MenuItem(key: 'quit');
  final _menu = Menu();

  VoidCallback? _onOpen;
  ValueChanged<bool>? _onToggle;
  void Function(String id, bool windows)? _onKeyboardMode;
  final _keyboardSeparator = MenuItem.separator();
  final Map<String, MenuItem> _keyboardItems = {};
  VoidCallback? _onQuit;
  bool _initialized = false;
  bool _menuOpen = false;
  bool _supported = true;
  TrayView? _wanted;
  TrayView? _rendered;
  Future<void> _rendering = Future<void>.value();

  /// Put the item in the menu bar. [onToggle] receives the checkbox's new
  /// value; the app writes it to the settings, and the next [update] shows
  /// whatever was saved.
  Future<void> initialize({
    required TrayView view,
    required VoidCallback onOpen,
    required ValueChanged<bool> onToggle,
    required VoidCallback onQuit,
    void Function(String id, bool windows)? onKeyboardMode,
  }) async {
    _onOpen = onOpen;
    _onToggle = onToggle;
    _onKeyboardMode = onKeyboardMode;
    _onQuit = onQuit;
    if (_initialized) return update(view);
    _initialized = true;
    _tray.addListener(this);
    await _call(
      () => _tray.setIcon(iconAsset, isTemplate: !Platform.isWindows),
    );
    await update(view);
  }

  /// Show [view]. Renders are serialized, and one wanted while the menu is
  /// open waits for it to close, so the menu never changes under the pointer.
  Future<void> update(TrayView view) {
    _wanted = view;
    if (!_initialized || _menuOpen) return Future<void>.value();
    return _rendering = _rendering.then((_) => _render());
  }

  Future<void> _render() async {
    final view = _wanted;
    if (!_initialized ||
        !_supported ||
        view == null ||
        _sameView(view, _rendered)) {
      return;
    }
    _statusItem.label = view.status;
    _toggleItem
      ..label = view.toggleLabel
      ..checked = view.enabled;
    _openItem.label = view.openLabel;
    _quitItem.label = view.quitLabel;
    final keyboards = [
      for (final keyboard in view.keyboards)
        (_keyboardItems[keyboard.id] ??= MenuItem.checkbox(
            key: 'mode:${keyboard.id}',
            checked: false,
          ))
          ..label = keyboard.label
          ..checked = keyboard.windows,
    ];
    _menu.items = [
      _statusItem,
      if (view.toggleLabel.isNotEmpty) _toggleItem,
      if (keyboards.isNotEmpty) ...[_keyboardSeparator, ...keyboards],
      _separator,
      _openItem,
      _quitItem,
    ];
    await _call(() => _tray.setContextMenu(_menu));
    await _call(() => _tray.setToolTip(view.status));
    _rendered = view;
  }

  /// One plugin call; a platform without the method turns the tray off
  /// rather than failing every later update.
  Future<void> _call(Future<void> Function() action) async {
    if (!_supported) return;
    try {
      await action();
    } on MissingPluginException {
      _supported = false;
    } on PlatformException catch (e) {
      debugPrint('Tray call failed: ${e.code} ${e.message ?? ''}');
    }
  }

  @override
  void onTrayIconMouseDown() {
    // A left click opens the menu, as every menu bar item does on macOS; on
    // Windows it opens the window, and the menu is the right click's.
    if (Platform.isWindows) {
      _onOpen?.call();
      return;
    }
    _popUp();
  }

  @override
  void onTrayIconRightMouseDown() => _popUp();

  void _popUp() {
    if (!_initialized || !_supported || _menuOpen) return;
    _menuOpen = true;
    unawaited(_showMenu());
  }

  Future<void> _showMenu() async {
    try {
      await _rendering;
      await _render();
      await _call(_tray.popUpContextMenu);
    } finally {
      _menuOpen = false;
      final wanted = _wanted;
      if (wanted != null && wanted != _rendered) unawaited(update(wanted));
    }
  }

  /// Records compare lists by identity, so the keyboards compare by value.
  static bool _sameView(TrayView a, TrayView? b) =>
      b != null &&
      a.status == b.status &&
      a.toggleLabel == b.toggleLabel &&
      a.enabled == b.enabled &&
      a.openLabel == b.openLabel &&
      a.quitLabel == b.quitLabel &&
      listEquals(a.keyboards, b.keyboards);

  @override
  void onTrayMenuItemClick(MenuItem menuItem) {
    final key = menuItem.key ?? '';
    if (key.startsWith('mode:')) {
      final id = key.substring('mode:'.length);
      final now = _rendered?.keyboards.where((k) => k.id == id).firstOrNull;
      if (now != null) _onKeyboardMode?.call(id, !now.windows);
      return;
    }
    switch (menuItem.key) {
      case 'toggle':
        _onToggle?.call(!(_rendered?.enabled ?? false));
      case 'open':
        _onOpen?.call();
      case 'quit':
        _onQuit?.call();
    }
  }

  /// Take the item out of the menu bar's event flow. The process is about
  /// to end, and AppKit removes the item with it.
  void dispose() {
    if (!_initialized) return;
    _tray.removeListener(this);
    _initialized = false;
    _rendered = null;
  }
}
