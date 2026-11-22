import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/ffi/isthmus_api.dart';
import 'package:isthmus_ui/src/pages/keyboard_page.dart';
import 'package:isthmus_ui/src/pages/mouse_page.dart';
import 'package:isthmus_ui/src/pages/overview_page.dart';
import 'package:isthmus_ui/src/pages/settings_page.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:material_ui/material_ui.dart';

import 'fake_isthmus_api.dart';
import 'support.dart';

/// Scroll [list] from top to bottom, failing on the first exception.
Future<void> _scrollThrough(WidgetTester tester, String list) async {
  final scrollable = find
      .descendant(of: find.byKey(Key(list)), matching: find.byType(Scrollable))
      .first;
  for (var i = 0; i < 30; i++) {
    expect(tester.takeException(), isNull, reason: '$list at step $i');
    final position = tester.state<ScrollableState>(scrollable).position;
    if (position.pixels >= position.maxScrollExtent) break;
    await tester.drag(scrollable, const Offset(0, -250));
    await tester.pumpAndSettle();
  }
  expect(tester.takeException(), isNull, reason: list);
}

void main() {
  testWidgets('the rail opens each of the four pages', (tester) async {
    await pumpApp(tester, FakeIsthmusApi());
    expect(find.byType(OverviewPage), findsOneWidget);

    await tester.tap(find.byKey(const Key('nav-keyboard')));
    await settle(tester);
    expect(find.byType(KeyboardPage), findsOneWidget);

    await tester.tap(find.byKey(const Key('nav-mouse')));
    await settle(tester);
    expect(find.byType(MousePage), findsOneWidget);

    await tester.tap(find.byKey(const Key('nav-settings')));
    await settle(tester);
    expect(find.byType(SettingsPage), findsOneWidget);

    await tester.tap(find.byKey(const Key('nav-overview')));
    await settle(tester);
    expect(find.byType(OverviewPage), findsOneWidget);
    await finish(tester);
  });

  testWidgets('a refused write rolls back and says why in a toast', (
    tester,
  ) async {
    final api = FakeIsthmusApi()..failing['settings_set'] = ErrorCode.io;
    await pumpApp(tester, api);

    await tester.tap(find.byKey(const Key('overview-enabled')));
    await settle(tester);

    expect(
      tester.widget<Switch>(find.byKey(const Key('overview-enabled'))).value,
      isFalse,
    );
    expect(find.text('The change was not saved'), findsOneWidget);
    expect(find.text('settings_set refused'), findsOneWidget);
    await finish(tester);
  });

  testWidgets('the rail foot says what is in the way first', (tester) async {
    await pumpApp(
      tester,
      FakeIsthmusApi(
        status: statusJson(accessibility: 'denied'),
        settings: {'enabled': true},
      ),
    );
    expect(
      find.descendant(
        of: find.byKey(const Key('rail-status')),
        matching: find.text('Needs Accessibility'),
      ),
      findsOneWidget,
    );
    await finish(tester);
  });

  for (final locale in const [Locale('en'), Locale('zh')]) {
    testWidgets(
      'nothing overflows at the smallest window in ${locale.languageCode}',
      (tester) async {
        final api = busyApi();
        for (final (page, list) in [
          (AppPage.overview, 'overview-list'),
          (AppPage.keyboard, 'keyboard-list'),
          (AppPage.mouse, 'mouse-list'),
          (AppPage.settings, 'settings-list'),
        ]) {
          await pumpApp(
            tester,
            api,
            page: page,
            size: const Size(760, 560),
            locale: locale,
          );
          if (page == AppPage.keyboard) {
            // Open a card of examples: keycaps are the widest rows.
            await tapInList(
              tester,
              find.byKey(const Key('keyboard-card-system-expand')),
              list: list,
            );
            await tester.drag(find.byKey(Key(list)), const Offset(0, 4000));
            await tester.pumpAndSettle();
          }
          await _scrollThrough(tester, list);
          await finish(tester);
        }
      },
    );
  }
}
