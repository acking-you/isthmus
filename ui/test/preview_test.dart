/// Renders the app with its real typefaces to PNGs, for looking at the
/// design without a Mac build. Opt-in: runs only when
/// `ISTHMUS_UI_PREVIEW_DIR` names a directory to write into. Chinese needs a
/// CJK face the test can load: `ISTHMUS_UI_PREVIEW_CJK` names a .ttf/.otf
/// (PingFang is a system collection a test cannot open).
///
/// ```sh
/// ISTHMUS_UI_PREVIEW_DIR=/tmp/isthmus flutter test test/preview_test.dart
/// ```
library;

import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';

import 'fake_isthmus_api.dart';
import 'support.dart';

final String? _dir = Platform.environment['ISTHMUS_UI_PREVIEW_DIR'];
final String? _cjk = Platform.environment['ISTHMUS_UI_PREVIEW_CJK'];

/// Loads the app's real faces, which `flutter test` otherwise draws as
/// boxes, and both Lucide strokes.
Future<void> _loadRealFonts() async {
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
  final cjk = _cjk;
  if (cjk != null) {
    final bytes = File(cjk).readAsBytesSync();
    await family('PingFang SC', [Future.value(ByteData.sublistView(bytes))]);
  }
}

Future<void> _capture(WidgetTester tester, String name) async {
  final boundary = tester.renderObject<RenderRepaintBoundary>(
    find.byKey(appRootKey),
  );
  await tester.runAsync(() async {
    final image = await boundary.toImage(pixelRatio: 1.5);
    final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
    File('$_dir/$name.png').writeAsBytesSync(bytes!.buffer.asUint8List());
  });
}

void main() {
  if (_dir == null) {
    test('previews are opt-in', () {}, skip: 'set ISTHMUS_UI_PREVIEW_DIR');
    return;
  }
  for (final locale in [
    const Locale('en'),
    if (_cjk != null) const Locale('zh'),
  ]) {
    for (final page in AppPage.values) {
      testWidgets('preview ${page.name} ${locale.languageCode}', (
        tester,
      ) async {
        debugDisableShadows = false;
        try {
          await tester.runAsync(_loadRealFonts);
          await pumpApp(tester, busyApi(), page: page, locale: locale);
          await _capture(tester, '${page.name}-${locale.languageCode}');
          if (page == AppPage.keyboard) {
            await tapInList(
              tester,
              find.byKey(const Key('keyboard-card-editing-expand')),
              list: 'keyboard-list',
            );
            await _capture(tester, 'shortcuts-${locale.languageCode}');
          }
          await finish(tester);
        } finally {
          debugDisableShadows = true;
        }
      });
    }
  }
}
