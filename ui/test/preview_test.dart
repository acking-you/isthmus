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

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart' show Scrollable;
import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';

import 'fake_isthmus_api.dart';
import 'support.dart';

final String? _dir = Platform.environment['ISTHMUS_UI_PREVIEW_DIR'];
final String? _cjk = Platform.environment['ISTHMUS_UI_PREVIEW_CJK'];

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
          await tester.runAsync(() => loadRealFonts(cjk: _cjk));
          await pumpApp(tester, busyApi(), page: page, locale: locale);
          await _capture(tester, '${page.name}-${locale.languageCode}');
          if (page == AppPage.devices) {
            await tester.scrollUntilVisible(
              find.byKey(const Key('transfer-t3')),
              300,
              scrollable: find
                  .descendant(
                    of: find.byKey(const Key('devices-list')),
                    matching: find.byType(Scrollable),
                  )
                  .first,
            );
            // A retrying transfer's bar never settles: pump, do not settle.
            await tester.pump(const Duration(milliseconds: 300));
            await _capture(tester, 'transfers-${locale.languageCode}');
          }
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
