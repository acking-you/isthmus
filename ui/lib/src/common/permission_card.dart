import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

/// The native drag-to-authorize card (`ui/macos/Runner/PermissionHelper.swift`):
/// shown under System Settings' Accessibility pane, it holds Isthmus's icon
/// for the user to drag into the list, so turning the switch on is all that
/// is left to do.
///
/// Every call is best effort: without the native side (a test, another
/// platform) it does nothing, and a failure never reaches the user, who can
/// still add the app with "+".
class PermissionCard {
  PermissionCard._();

  static const _channel = MethodChannel('isthmus/permission_helper');

  static bool _shown = false;

  /// Whether the card was shown and not hidden since.
  static bool get shown => _shown;

  /// Shows the card with these texts; it follows the Settings window.
  static Future<void> show({
    required String title,
    required String subtitle,
    required String appName,
  }) async {
    _shown = true;
    await _invoke('show', {
      'title': title,
      'subtitle': subtitle,
      'appName': appName,
    });
  }

  /// Hides the card, if it is up.
  static Future<void> hide() async {
    if (!_shown) return;
    _shown = false;
    await _invoke('hide');
  }

  static Future<void> _invoke(String method, [Object? arguments]) async {
    try {
      await _channel.invokeMethod<void>(method, arguments);
    } on MissingPluginException {
      // No native side: nothing to show.
    } on PlatformException catch (error) {
      debugPrint('The permission card failed: $error');
    }
  }
}
