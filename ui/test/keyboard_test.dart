import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:isthmus_ui/src/widgets/keycap.dart';
import 'package:material_ui/material_ui.dart';

import 'fake_isthmus_api.dart';
import 'support.dart';

/// The keyboard entries the last `settings_set` carried.
List<Map<String, Object?>> _savedKeyboards(FakeIsthmusApi api) => [
  for (final entry in (api.saved.last['keyboard']! as Map)['devices'] as List)
    (entry as Map).cast<String, Object?>(),
];

void main() {
  testWidgets('a layout override writes the keyboard its own entry', (
    tester,
  ) async {
    final api = FakeIsthmusApi(
      devices: [deviceJson('kbd:1', 'keyboard', name: 'K8 Pro')],
    );
    await pumpApp(tester, api, page: AppPage.keyboard);
    expect(find.text('K8 Pro'), findsOneWidget);
    expect(find.text('Detected: PC, medium confidence'), findsOneWidget);

    await tester.tap(find.byKey(const Key('keyboard-layout-kbd:1-mac')));
    await settle(tester);

    // The entry starts from what the keyboard has now, so the override
    // changes the layout and nothing else.
    expect(_savedKeyboards(api), [
      {
        'id': 'kbd:1',
        'name': 'K8 Pro',
        'style': 'windows',
        'layout': 'mac',
        'functionKeys': 'standard',
      },
    ]);
    await finish(tester);
  });

  testWidgets('the per-keyboard switches change only their own field', (
    tester,
  ) async {
    final api = FakeIsthmusApi(
      settings: {
        'keyboard': {
          'devices': [
            {
              'id': 'kbd:1',
              'name': 'K8',
              'style': 'windows',
              'layout': 'pc',
              'functionKeys': 'standard',
            },
          ],
        },
      },
      devices: [deviceJson('kbd:1', 'keyboard', name: 'K8')],
    );
    await pumpApp(tester, api, page: AppPage.keyboard);

    await tester.tap(find.byKey(const Key('keyboard-fkeys-kbd:1')));
    await settle(tester);
    expect(_savedKeyboards(api).single['functionKeys'], 'system');
    expect(_savedKeyboards(api).single['layout'], 'pc');

    await tester.tap(find.byKey(const Key('keyboard-style-kbd:1')));
    await settle(tester);
    expect(_savedKeyboards(api).single['style'], 'mac');
    expect(_savedKeyboards(api).single['functionKeys'], 'system');
    await finish(tester);
  });

  testWidgets('a group switch writes just that group', (tester) async {
    final api = FakeIsthmusApi();
    await pumpApp(tester, api, page: AppPage.keyboard);

    await tapInList(
      tester,
      find.byKey(const Key('keyboard-group-editing')),
      list: 'keyboard-list',
    );

    final groups = (api.saved.last['keyboard']! as Map)['groups'] as Map;
    expect(groups, {'editing': false});
    // One group off is a style of its own.
    await tester.drag(
      find.byKey(const Key('keyboard-list')),
      const Offset(0, 3000),
    );
    await settle(tester);
    expect(find.byKey(const Key('keyboard-style-custom')), findsOneWidget);
    await finish(tester);
  });

  testWidgets('a group shows its chords as keycaps when opened', (
    tester,
  ) async {
    await pumpApp(tester, FakeIsthmusApi(), page: AppPage.keyboard);
    expect(find.byType(Keycap), findsNothing);

    await tapInList(
      tester,
      find.byKey(const Key('keyboard-card-editing-expand')),
      list: 'keyboard-list',
    );

    expect(find.text('Redo'), findsOneWidget);
    expect(
      find.descendant(
        of: find.byKey(const Key('keyboard-card-editing')),
        matching: find.widgetWithText(Keycap, 'Ctrl'),
      ),
      findsWidgets,
    );
    expect(
      find.descendant(
        of: find.byKey(const Key('keyboard-card-editing')),
        matching: find.widgetWithText(Keycap, '⌘'),
      ),
      findsWidgets,
    );
    await finish(tester);
  });

  testWidgets('the keyboard style applies a preset to the keyboard only', (
    tester,
  ) async {
    final api = FakeIsthmusApi();
    await pumpApp(tester, api, page: AppPage.keyboard);

    await tester.tap(find.byKey(const Key('keyboard-style-mac')));
    await settle(tester);

    expect(api.calls, contains('preset_apply mac keyboard'));
    await finish(tester);
  });

  testWidgets('an app picked from the running apps becomes a rule', (
    tester,
  ) async {
    final api = FakeIsthmusApi(
      runningApps: [
        appJson('com.example.Editor', 'Editor'),
        appJson('com.googlecode.iterm2', 'iTerm2', appClass: 'terminal'),
      ],
    );
    await pumpApp(tester, api, page: AppPage.keyboard);

    await tapInList(
      tester,
      find.byKey(const Key('keyboard-add-app')),
      list: 'keyboard-list',
    );
    await tester.tap(find.byKey(const Key('add-app-com.example.Editor')));
    await settle(tester);

    expect((api.saved.last['keyboard']! as Map)['appRules'], [
      {'bundleId': 'com.example.Editor', 'class': 'standard'},
    ]);

    // The class menu changes the rule in place.
    await tapInList(
      tester,
      find.byKey(const Key('keyboard-rule-com.example.Editor-class')),
      list: 'keyboard-list',
    );
    await tester.tap(find.text('Passthrough').last);
    await settle(tester);
    expect((api.saved.last['keyboard']! as Map)['appRules'], [
      {'bundleId': 'com.example.Editor', 'class': 'passthrough'},
    ]);

    await tester.tap(
      find.byKey(const Key('keyboard-rule-com.example.Editor-remove')),
    );
    await settle(tester);
    expect((api.saved.last['keyboard']! as Map)['appRules'], isEmpty);
    await finish(tester);
  });
}
