/// The Windows app, run for real (CI's `windows-app` job): closing the
/// window puts it in the notification area, and the tray's click brings it
/// back drawing and answering. Up to 0.1.3 the app stopped responding at
/// that point: closing the window also asked Flutter to exit the app, and
/// the app's answer released the tray and the library.
///
/// Tray clicks arrive over tray_manager's platform channel, as the plugin's
/// native side sends them, so a tray listener that was dropped is caught.
///
/// Windows only: on a Mac the tray's click opens its menu instead.
///
/// ```sh
/// flutter test integration_test/tray_test.dart -d windows
/// ```
library;

import 'dart:async';
import 'dart:io' show Platform;

import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:isthmus_ui/src/app.dart';
import 'package:isthmus_ui/src/common/system_notice.dart';
import 'package:isthmus_ui/src/shell/app_prefs.dart';
import 'package:isthmus_ui/src/shell/window_chrome.dart';
import 'package:material_ui/material_ui.dart';
import 'package:window_manager/window_manager.dart';

import '../test/fake_isthmus_api.dart';

/// Waits, in real time, until [ready] holds, failing after [limit].
Future<void> _until(
  String what,
  Future<bool> Function() ready, {
  Duration limit = const Duration(seconds: 10),
}) async {
  final deadline = DateTime.now().add(limit);
  while (!await ready()) {
    if (DateTime.now().isAfter(deadline)) fail('timed out waiting for $what');
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
}

/// A left click on the tray icon, delivered as tray_manager's native side
/// delivers it on Windows: a method call on its channel.
Future<void> _trayClick() {
  final answered = Completer<void>();
  ServicesBinding.instance.channelBuffers.push(
    'tray_manager',
    const StandardMethodCodec().encodeMethodCall(
      const MethodCall('onTrayIconMouseDown'),
    ),
    (_) => answered.complete(),
  );
  return answered.future.timeout(const Duration(seconds: 5));
}

/// A frame drawn within a few seconds, or a failure saying frames stopped.
Future<void> _drawsAFrame(WidgetTester tester, String when) async {
  expect(
    SchedulerBinding.instance.framesEnabled,
    isTrue,
    reason:
        'frames are off $when (lifecycle '
        '${SchedulerBinding.instance.lifecycleState})',
  );
  await tester
      .pump(const Duration(milliseconds: 50))
      .timeout(
        const Duration(seconds: 5),
        onTimeout: () => fail('no frame was drawn $when'),
      );
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('the window comes back from the tray drawing and answering', (
    tester,
  ) async {
    await windowManager.ensureInitialized();
    final ready = windowManager.waitUntilReadyToShow(isthmusWindowOptions);
    // Onboarded, so the launch shows the window once (the runner's first
    // frame) and never again behind the test's back.
    final prefs = AppPrefs(persist: false)
      ..autoCheckUpdates = false
      ..onboarded = true;
    await tester.pumpWidget(
      IsthmusApp(
        api: FakeIsthmusApi(),
        prefs: prefs,
        windowReady: ready,
        pollInterval: null,
      ),
    );
    await _until('the window to show', windowManager.isVisible);
    await _drawsAFrame(tester, 'at launch');

    // Closing hides to the notification area (setPreventClose).
    await windowManager.close();
    await _until('the window to hide', () async {
      return !await windowManager.isVisible();
    });

    // A clipboard copy crossing while hidden posts a notification, which
    // Windows dismisses a few seconds later; 0.1.2's notifications froze
    // the app at that dismissal.
    await SystemNotice.show(
      id: 'isthmus-clipboard',
      title: 'Clipboard from Desk PC',
      body: 'hello from the other computer',
    );
    await Future<void>.delayed(const Duration(seconds: 12));

    // What Windows sends for a left click on the tray icon.
    await _trayClick();
    await _until('the window to come back', windowManager.isVisible);
    await _drawsAFrame(tester, 'after coming back from the tray');

    // And it still answers a click.
    await tester.tap(find.byKey(const Key('nav-settings')));
    await _drawsAFrame(tester, 'after a click');
    await tester.pump(const Duration(milliseconds: 500));
    expect(find.byKey(const Key('settings-list')), findsOneWidget);

    // Twice, as a day of use does.
    await windowManager.close();
    await _until('the window to hide again', () async {
      return !await windowManager.isVisible();
    });
    await _trayClick();
    await _until('the window to come back again', windowManager.isVisible);
    await _drawsAFrame(tester, 'after the second return');
  }, skip: !Platform.isWindows);
}
