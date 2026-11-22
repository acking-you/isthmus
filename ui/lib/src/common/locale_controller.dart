import 'dart:ui' show Locale;

import 'package:shared_preferences/shared_preferences.dart';

/// Remembers the user's language choice.
///
/// A null locale means "follow the system", which is the first-run default so
/// a Chinese Mac starts in Chinese without being asked.
abstract final class LocaleController {
  static const String _storageKey = 'app_locale';

  /// The languages the app ships, in the switcher's order.
  static const List<Locale> supported = [Locale('en'), Locale('zh')];

  /// The saved choice, or null to follow the system. A failed read only
  /// costs the choice, never the launch.
  static Future<Locale?> load() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      final code = prefs.getString(_storageKey);
      if (code == null || code.isEmpty) return null;
      return supported.firstWhere(
        (locale) => locale.languageCode == code,
        orElse: () => supported.first,
      );
    } catch (_) {
      return null;
    }
  }

  /// Remember [locale]; null goes back to following the system.
  static Future<void> save(Locale? locale) async {
    try {
      final prefs = await SharedPreferences.getInstance();
      if (locale == null) {
        await prefs.remove(_storageKey);
      } else {
        await prefs.setString(_storageKey, locale.languageCode);
      }
    } catch (_) {
      // Losing the preference only costs the choice on next launch.
    }
  }

  /// The supported locale a system locale maps onto: Chinese for any
  /// Chinese, English for everything else.
  static Locale resolve(Locale systemLocale) => supported.firstWhere(
    (locale) => locale.languageCode == systemLocale.languageCode,
    orElse: () => supported.first,
  );
}
