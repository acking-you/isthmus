import 'dart:ui' show Locale, PlatformDispatcher;

import 'package:flutter/foundation.dart';
import 'package:isthmus_ui/src/common/locale_controller.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// The user's choices about the app itself: the language, and whether the
/// window has been shown once. Remembered across launches; a failed read or
/// write only costs the choice, never the launch.
class AppPrefs extends ChangeNotifier {
  AppPrefs({this.persist = true});

  /// Off in tests, which have no preference store.
  final bool persist;

  static const _onboardedKey = 'onboarded';
  static const _relaunchKey = 'accessibilityRelaunchAtMs';

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
}
