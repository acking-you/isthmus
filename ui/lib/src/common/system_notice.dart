import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:local_notifier/local_notifier.dart';

/// Notifications outside the window, for what happens while Isthmus waits
/// in the menu bar or the notification area: a clipboard copy that crossed.
///
/// A Mac posts them through the Runner (`Notifier.swift`, the user
/// notification center: local_notifier's macOS side uses the deprecated
/// `NSUserNotification`); Windows through local_notifier's toasts. A
/// notice with the id of one still shown replaces it on a Mac. Never
/// throws: a notice that cannot be shown is only a missed notice.
abstract final class SystemNotice {
  static const _mac = MethodChannel('isthmus/notifier');
  static bool _windowsReady = false;

  /// Registers the app for toasts on Windows; nothing to do elsewhere.
  static Future<void> setup() async {
    if (!Platform.isWindows || _windowsReady) return;
    try {
      await localNotifier.setup(
        appName: 'Isthmus',
        shortcutPolicy: ShortcutPolicy.requireCreate,
      );
      _windowsReady = true;
    } catch (error) {
      debugPrint('Windows notifications are unavailable: $error');
    }
  }

  static Future<void> show({
    required String id,
    required String title,
    required String body,
  }) async {
    try {
      if (Platform.isMacOS) {
        await _mac.invokeMethod<void>('notify', {
          'id': id,
          'title': title,
          'body': body,
        });
      } else if (Platform.isWindows) {
        await setup();
        if (!_windowsReady) return;
        await LocalNotification(
          identifier: id,
          title: title,
          body: body,
        ).show();
      }
    } catch (error) {
      debugPrint('The notification could not be shown: $error');
    }
  }
}
