import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/shell/app_prefs.dart';

void main() {
  /// The app restarts itself for a new Accessibility grant at most once in
  /// a while: a second request soon after means a restart does not help,
  /// and looping would make the app unusable.
  test(
    'a relaunch is remembered and blocks another within the cooldown',
    () async {
      final prefs = AppPrefs(persist: false);
      final start = DateTime(2026, 11, 22, 7);
      expect(
        prefs.relaunchedWithin(AppPrefs.relaunchCooldown, now: start),
        isFalse,
      );

      await prefs.markAccessibilityRelaunch(now: start);
      expect(
        prefs.relaunchedWithin(
          AppPrefs.relaunchCooldown,
          now: start.add(const Duration(minutes: 1)),
        ),
        isTrue,
      );
      expect(
        prefs.relaunchedWithin(
          AppPrefs.relaunchCooldown,
          now: start.add(const Duration(minutes: 4)),
        ),
        isFalse,
      );
    },
  );
}
