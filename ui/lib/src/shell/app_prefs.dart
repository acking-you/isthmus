import 'dart:ui' show Locale, PlatformDispatcher;

import 'package:flutter/foundation.dart';
import 'package:isthmus_ui/src/common/locale_controller.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// The user's choices about the app itself: the language, whether the
/// window has been shown once, and the update check. Remembered across launches; a failed read or
/// write only costs the choice, never the launch.
class AppPrefs extends ChangeNotifier {
  AppPrefs({this.persist = true});

  /// Off in tests, which have no preference store.
  final bool persist;

  static const _onboardedKey = 'onboarded';
  static const _relaunchKey = 'accessibilityRelaunchAtMs';
  static const _autoUpdateKey = 'updateAutoCheck';
  static const _lastUpdateCheckKey = 'updateLastCheckMs';
  static const _announcedUpdateKey = 'updateAnnounced';

  /// Two automatic relaunches this close together mean the relaunch did not
  /// help: the second time the app asks the user instead of looping.
  static const relaunchCooldown = Duration(minutes: 3);

  /// Null follows the system language.
  Locale? locale;

  /// The window has been shown once. Until then the app opens its window at
  /// launch, so a new user sees what it is; after that it waits in the menu
  /// bar.
  bool onboarded = false;

  /// When the app last relaunched itself to apply a new Accessibility
  /// grant, or null.
  DateTime? lastAccessibilityRelaunch;

  /// Whether the app looks for a newer release on its own, about twice a
  /// day. Nothing is downloaded without the user.
  bool autoCheckUpdates = true;

  /// When it last did.
  DateTime? lastUpdateCheck;

  /// The newest version the user has been told about, so each is announced
  /// once.
  String? announcedUpdate;

  /// The app relaunched itself for Accessibility within [window]: the new
  /// instance shows its window so the user sees the grant took effect, and
  /// does not relaunch again.
  bool relaunchedWithin(Duration window, {DateTime? now}) {
    final last = lastAccessibilityRelaunch;
    if (last == null) return false;
    return (now ?? DateTime.now()).difference(last) < window;
  }

  /// Records an automatic relaunch before it happens.
  Future<void> markAccessibilityRelaunch({DateTime? now}) async {
    lastAccessibilityRelaunch = now ?? DateTime.now();
    if (!persist) return;
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setInt(
        _relaunchKey,
        lastAccessibilityRelaunch!.millisecondsSinceEpoch,
      );
    } catch (_) {}
  }

  Future<void> load() async {
    if (!persist) return;
    try {
      final prefs = await SharedPreferences.getInstance();
      onboarded = prefs.getBool(_onboardedKey) ?? false;
      final relaunchMs = prefs.getInt(_relaunchKey);
      lastAccessibilityRelaunch = relaunchMs == null
          ? null
          : DateTime.fromMillisecondsSinceEpoch(relaunchMs);
      autoCheckUpdates = prefs.getBool(_autoUpdateKey) ?? true;
      final lastCheck = prefs.getInt(_lastUpdateCheckKey);
      lastUpdateCheck = lastCheck == null
          ? null
          : DateTime.fromMillisecondsSinceEpoch(lastCheck);
      announcedUpdate = prefs.getString(_announcedUpdateKey);
    } catch (_) {}
    locale = await LocaleController.load();
    notifyListeners();
  }

  /// The language actually shown.
  Locale get activeLocale =>
      locale ?? LocaleController.resolve(PlatformDispatcher.instance.locale);

  /// [next] null follows the system again.
  Future<void> setLocale(Locale? next) async {
    if (next == locale) return;
    locale = next;
    notifyListeners();
    if (!persist) return;
    await LocaleController.save(next);
  }

  Future<void> markOnboarded() async {
    if (onboarded) return;
    onboarded = true;
    if (!persist) return;
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setBool(_onboardedKey, true);
    } catch (_) {}
  }

  Future<void> setAutoCheckUpdates(bool value) async {
    if (value == autoCheckUpdates) return;
    autoCheckUpdates = value;
    notifyListeners();
    if (!persist) return;
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setBool(_autoUpdateKey, value);
    } catch (_) {}
  }

  Future<void> markUpdateCheck(DateTime at) async {
    lastUpdateCheck = at;
    if (!persist) return;
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setInt(_lastUpdateCheckKey, at.millisecondsSinceEpoch);
    } catch (_) {}
  }

  Future<void> markUpdateAnnounced(String version) async {
    announcedUpdate = version;
    if (!persist) return;
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setString(_announcedUpdateKey, version);
    } catch (_) {}
  }
}
