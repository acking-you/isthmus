import 'dart:async';
import 'dart:io';

import 'package:flutter/services.dart';

import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/app.dart';
import 'package:isthmus_ui/src/ffi/models.dart';
import 'package:isthmus_ui/src/shell/app_prefs.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:isthmus_ui/src/updates/update_controller.dart';
import 'package:material_ui/material_ui.dart';
import 'package:toastification/toastification.dart' show toastification;

import 'fake_isthmus_api.dart';

/// The root the app is pumped under, for previews.
const Key appRootKey = Key('app-root');

/// Change hints a test can send, standing in for the Rust callback.
final changes = StreamController<Change>.broadcast();

/// Pump the whole app over [api] at [size], opening on [page], in [locale],
/// with motion reduced so every animation lands at once. No status polling:
/// a periodic timer would outlive the test.
Future<void> pumpApp(
  WidgetTester tester,
  FakeIsthmusApi api, {
  AppPage page = AppPage.overview,
  Size size = const Size(980, 680),
  Locale locale = const Locale('en'),
  UpdateController? updates,
}) async {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  tester.platformDispatcher.accessibilityFeaturesTestValue =
      const FakeAccessibilityFeatures(disableAnimations: true);
  addTearDown(tester.platformDispatcher.clearAccessibilityFeaturesTestValue);
  final prefs = AppPrefs(persist: false)..locale = locale;
  await tester.pumpWidget(
    RepaintBoundary(
      key: appRootKey,
      child: IsthmusApp(
        api: api,
        changes: changes.stream,
        prefs: prefs,
        initialPage: page,
        pollInterval: null,
        updates: updates,
      ),
    ),
  );
  await settle(tester);
}

/// Let async reads land and the page settle.
Future<void> settle(WidgetTester tester) async {
  for (var i = 0; i < 4; i++) {
    await tester.pump(const Duration(milliseconds: 50));
  }
  await tester.pumpAndSettle(const Duration(milliseconds: 16));
}

/// Close every toast, unmount the app, and let any toast's timer run out, so
/// no timer outlives the test. An error toast stays until closed, and one
/// left open would keep toastification holding an overlay the next test no
/// longer has.
Future<void> finish(WidgetTester tester) async {
  toastification.dismissAll(delayForAnimation: false);
  await tester.pump(const Duration(seconds: 1));
  await tester.pumpWidget(const SizedBox.shrink());
  await tester.pump(const Duration(seconds: 6));
}

/// Scroll the page's list into view of [finder] and tap it.
Future<void> tapInList(
  WidgetTester tester,
  Finder finder, {
  required String list,
}) async {
  await tester.scrollUntilVisible(
    finder,
    200,
    scrollable: find
        .descendant(
          of: find.byKey(Key(list)),
          matching: find.byType(Scrollable),
        )
        .first,
  );
  await tester.pumpAndSettle();
  await tester.tap(finder);
  await settle(tester);
}

/// Loads the app's real faces, which `flutter test` otherwise draws as
/// boxes, both Lucide strokes and, when [cjk] names a .ttf/.otf, a Chinese
/// face under the name the theme falls back to.
Future<void> loadRealFonts({String? cjk}) async {
  Future<void> family(String name, List<Future<ByteData>> fonts) async {
    final loader = FontLoader(name);
    fonts.forEach(loader.addFont);
    await loader.load();
  }

  await family('GoogleSansFlex', [
    for (final weight in ['Regular', 'Medium', 'SemiBold', 'Bold'])
      rootBundle.load('assets/fonts/GoogleSansFlex-$weight.ttf'),
  ]);
  await family('GeistMono', [
    for (final weight in ['Regular', 'Medium', 'SemiBold'])
      rootBundle.load('assets/fonts/GeistMono-$weight.ttf'),
  ]);
  await family('packages/lucide_icons_flutter/Lucide400', [
    rootBundle.load(
      'packages/lucide_icons_flutter/assets/build_font/LucideVariable-w400.ttf',
    ),
  ]);
  await family('packages/lucide_icons_flutter/Lucide300', [
    rootBundle.load(
      'packages/lucide_icons_flutter/assets/build_font/LucideVariable-w300.ttf',
    ),
  ]);
  // Windows' Chinese face, bundled (fonts.dart puts it first there).
  await family('NotoSansSC', [
    for (final weight in ['Regular', 'Medium', 'SemiBold', 'Bold'])
      rootBundle.load('assets/fonts/NotoSansSC-$weight.subset.ttf'),
  ]);
  if (cjk != null) {
    final bytes = File(cjk).readAsBytesSync();
    await family('PingFang SC', [Future.value(ByteData.sublistView(bytes))]);
  }
}
