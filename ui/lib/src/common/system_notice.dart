import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// Notifications outside the window, for what happens while Isthmus waits
/// in the menu bar or the notification area: a clipboard copy that crossed.
///
/// Both systems take them from the Runner over the `isthmus/notifier`
/// channel: a Mac posts through the user notification center
/// (`macos/Runner/Notifier.swift`), Windows as a balloon on Isthmus's own
/// notification-area icon, which Windows 10 and 11 show as a notification
/// (`windows/runner/flutter_window.cpp`). Both handle the call on the
/// platform thread. Never throws: a notice that cannot be shown is only a
/// missed notice.
abstract final class SystemNotice {
  static const _channel = MethodChannel('isthmus/notifier');

  static Future<void> show({
    required String id,
    required String title,
    required String body,
  }) async {
    try {
      await _channel.invokeMethod<Object?>('notify', {
        'id': id,
        'title': title,
        'body': body,
      });
    } catch (error) {
      debugPrint('The notification could not be shown: $error');
    }
  }
}
