import 'dart:async';

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
  // The "Windows habits" checkbox.
  String toggleLabel,
  bool enabled,
  String openLabel,
  String quitLabel,
});

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
  static const String iconAsset = 'assets/tray/tray_icon@2x.png';

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
  }) async {
    _onOpen = onOpen;
    _onToggle = onToggle;
    _onQuit = onQuit;
    if (_initialized) return update(view);
    _initialized = true;
    _tray.addListener(this);
    await _call(() => _tray.setIcon(iconAsset, isTemplate: true));
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
    if (!_initialized || !_supported || view == null || view == _rendered) {
      return;
    }
    _statusItem.label = view.status;
    _toggleItem
      ..label = view.toggleLabel
      ..checked = view.enabled;
    _openItem.label = view.openLabel;
    _quitItem.label = view.quitLabel;
    _menu.items = [_statusItem, _toggleItem, _openItem, _separator, _quitItem];
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
    // A left click opens the menu, as every menu bar item does on macOS.
    if (!_initialized || !_supported || _menuOpen) return;
    _menuOpen = true;
    unawaited(_showMenu());
  }

  @override
  void onTrayIconRightMouseDown() => onTrayIconMouseDown();

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

  @override
  void onTrayMenuItemClick(MenuItem menuItem) {
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
