import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/ffi/models.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:material_ui/material_ui.dart';

import 'fake_isthmus_api.dart';
import 'support.dart';

Map<String, Object?> _section(FakeIsthmusApi api) =>
    (api.saved.last['mouse']! as Map).cast<String, Object?>();

void main() {
  testWidgets('dragging the lines slider writes linesPerNotch once', (
    tester,
  ) async {
    final api = FakeIsthmusApi();
    await pumpApp(tester, api, page: AppPage.mouse);
    expect(find.text('3 lines'), findsOneWidget);

    // Far past the end: the slider stops at its maximum.
    await tester.drag(
      find.byKey(const Key('mouse-defaults-lines')),
      const Offset(600, 0),
    );
    await settle(tester);

    expect(api.saved, hasLength(1));
    expect((_section(api)['defaults']! as Map)['linesPerNotch'], 12);
    expect(find.text('12 lines'), findsOneWidget);
    await finish(tester);
  });

  testWidgets('middle-button drag scrolling has its own switch', (
    tester,
  ) async {
    final api = FakeIsthmusApi();
    await pumpApp(tester, api, page: AppPage.mouse);
    final drag = find.byKey(const Key('mouse-middle-drag'));
    expect(tester.widget<Switch>(drag).value, isTrue);
    await tester.tap(drag);
    await settle(tester);
    expect(api.saved.last['mouse'], containsPair('middleDragScroll', false));
    expect(api.stored.mouseStyle, ProfileStyle.custom);
    await finish(tester);
  });

  testWidgets('the lines slider is off while the wheel is not linear', (
    tester,
  ) async {
    final api = FakeIsthmusApi(
      settings: {
        'mouse': {
          'defaults': {'linearWheel': false},
        },
      },
    );
    await pumpApp(tester, api, page: AppPage.mouse);
    expect(
      tester
          .widget<Slider>(find.byKey(const Key('mouse-defaults-lines')))
          .onChanged,
      isNull,
    );
    await finish(tester);
  });

  testWidgets('a mouse leaves the defaults with a copy of them', (
    tester,
  ) async {
    final api = FakeIsthmusApi(
      devices: [
        deviceJson('mouse:1', 'mouse', name: 'MX Master'),
        deviceJson('tp:1', 'trackpad', name: 'Magic Trackpad'),
      ],
    );
    await pumpApp(tester, api, page: AppPage.mouse);

    await tapInList(
      tester,
      find.byKey(const Key('mouse-use-defaults-mouse:1')),
      list: 'mouse-list',
    );

    final devices = _section(api)['devices']! as List;
    expect(devices, hasLength(1));
    final entry = devices.single as Map;
    expect(entry['id'], 'mouse:1');
    expect((entry['profile'] as Map)['linesPerNotch'], 3);
    expect(find.byKey(const Key('mouse-mouse:1-lines')), findsOneWidget);

    // Trackpads are listed, never offered a setting.
    await tester.scrollUntilVisible(
      find.byKey(const Key('mouse-trackpad-tp:1')),
      200,
      scrollable: find
          .descendant(
            of: find.byKey(const Key('mouse-list')),
            matching: find.byType(Scrollable),
          )
          .first,
    );
    expect(find.text('Left as macOS has it'), findsOneWidget);
    await finish(tester);
  });

  testWidgets('the mouse style applies a preset to the mouse only', (
    tester,
  ) async {
    final api = FakeIsthmusApi();
    await pumpApp(tester, api, page: AppPage.mouse);

    await tester.tap(find.byKey(const Key('mouse-style-mac')));
    await settle(tester);

    expect(api.calls, contains('preset_apply mac mouse'));
    await finish(tester);
  });
}
