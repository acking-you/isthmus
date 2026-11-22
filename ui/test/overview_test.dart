import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/ffi/models.dart';
import 'package:material_ui/material_ui.dart';

import 'fake_isthmus_api.dart';
import 'support.dart';

/// Settings with every keyboard and mouse value at the Windows preset.
const _allWindows = <String, Object?>{'enabled': true};

/// Settings whose mouse is at the Mac preset, so the one click has work.
final _macMouse = <String, Object?>{
  'enabled': true,
  'mouse': {
    'defaults': {
      'reverseWheel': false,
      'linearWheel': false,
      'pointer': 'system',
      'sideButtons': false,
      'ctrlWheelZoom': false,
    },
  },
};

void main() {
  testWidgets('the hero switch writes settings.enabled', (tester) async {
    final api = FakeIsthmusApi();
    await pumpApp(tester, api);
    expect(
      tester.widget<Switch>(find.byKey(const Key('overview-enabled'))).value,
      isFalse,
    );

    await tester.tap(find.byKey(const Key('overview-enabled')));
    await settle(tester);

    expect(api.calls, contains('settings_set'));
    expect(api.saved.last['enabled'], isTrue);
    expect(
      tester.widget<Switch>(find.byKey(const Key('overview-enabled'))).value,
      isTrue,
    );
    await finish(tester);
  });

  testWidgets('the one-click button applies the Windows preset to everything', (
    tester,
  ) async {
    final api = FakeIsthmusApi(settings: _macMouse);
    await pumpApp(tester, api);
    expect(find.text('Mouse: Mac'), findsOneWidget);

    await tester.tap(find.byKey(const Key('overview-apply-windows')));
    await settle(tester);

    expect(api.calls, contains('preset_apply windows all'));
    // Everything is Windows now, so there is nothing left to click.
    expect(find.byKey(const Key('overview-apply-windows')), findsNothing);
    expect(find.text('Mouse: Windows'), findsOneWidget);
    await finish(tester);
  });

  testWidgets('the one-click button hides while everything is Windows', (
    tester,
  ) async {
    await pumpApp(tester, FakeIsthmusApi(settings: _allWindows));
    expect(find.byKey(const Key('overview-apply-windows')), findsNothing);
    await finish(tester);
  });

  testWidgets('the permission card shows only while Accessibility is missing', (
    tester,
  ) async {
    await pumpApp(tester, FakeIsthmusApi());
    expect(find.byKey(const Key('overview-permission')), findsNothing);
    await finish(tester);

    final api = FakeIsthmusApi(status: statusJson(accessibility: 'denied'));
    await pumpApp(tester, api);
    expect(find.byKey(const Key('overview-permission')), findsOneWidget);

    await tester.tap(find.byKey(const Key('overview-grant')));
    await settle(tester);

    expect(api.calls, contains('permission_request accessibility'));
    // The answer says it is granted now, so the card goes.
    expect(find.byKey(const Key('overview-permission')), findsNothing);
    await finish(tester);
  });

  testWidgets('another instance is named, and the switch cannot be used', (
    tester,
  ) async {
    final api = FakeIsthmusApi(status: statusJson(anotherInstance: true));
    await pumpApp(tester, api);
    expect(find.byKey(const Key('overview-another-instance')), findsOneWidget);
    expect(
      tester
          .widget<Switch>(find.byKey(const Key('overview-enabled')))
          .onChanged,
      isNull,
    );
    await finish(tester);
  });

  testWidgets('the front app is shown with what its class does to Ctrl', (
    tester,
  ) async {
    final api = FakeIsthmusApi(
      status: statusJson(
        frontApp: appJson(
          'com.apple.Terminal',
          'Terminal',
          appClass: 'terminal',
        ),
        secureInput: true,
      ),
    );
    await pumpApp(tester, api);
    expect(find.byKey(const Key('overview-front-app')), findsOneWidget);
    expect(find.text('Terminal: Ctrl stays Ctrl'), findsOneWidget);
    expect(find.textContaining('Secure Keyboard Entry'), findsOneWidget);
    await finish(tester);
  });

  testWidgets('every issue is explained in a sentence', (tester) async {
    final api = FakeIsthmusApi(
      status: statusJson(
        issues: [
          {'kind': 'restorePending', 'count': 2},
          {'kind': 'foreignPointerTool', 'name': 'G HUB'},
          {'kind': 'settingsUnreadable'},
          {'kind': 'somethingNew'},
        ],
      ),
    );
    await pumpApp(tester, api);
    await tester.scrollUntilVisible(
      find.byKey(const Key('overview-issues')),
      200,
      scrollable: find
          .descendant(
            of: find.byKey(const Key('overview-list')),
            matching: find.byType(Scrollable),
          )
          .first,
    );
    expect(
      find.textContaining('2 changes from an earlier run'),
      findsOneWidget,
    );
    expect(
      find.textContaining('G HUB also sets the pointer speed'),
      findsOneWidget,
    );
    expect(
      find.textContaining('The settings file could not be read'),
      findsOneWidget,
    );
    expect(
      find.text('Isthmus reported something this version cannot describe.'),
      findsOneWidget,
    );
    await finish(tester);
  });

  testWidgets('a status read on a change hint updates the page', (
    tester,
  ) async {
    final api = FakeIsthmusApi();
    await pumpApp(tester, api);
    expect(find.text('Terminal'), findsNothing);

    api.current = Status(
      statusJson(frontApp: appJson('com.apple.Terminal', 'Terminal')),
    );
    changes.add(const Change(ChangeKind.frontApp, seq: 1));
    await tester.pump(const Duration(milliseconds: 200));
    await settle(tester);

    expect(find.text('Terminal'), findsOneWidget);
    await finish(tester);
  });
}
