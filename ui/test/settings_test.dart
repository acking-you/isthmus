import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/ffi/models.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:material_ui/material_ui.dart';

import 'fake_isthmus_api.dart';
import 'support.dart';

void main() {
  testWidgets('putting everything back asks first, and only a yes restores', (
    tester,
  ) async {
    final api = FakeIsthmusApi(settings: {'enabled': true});
    await pumpApp(tester, api, page: AppPage.settings);

    await tapInList(
      tester,
      find.byKey(const Key('settings-restore')),
      list: 'settings-list',
    );
    await tester.tap(find.byKey(const Key('confirm-cancel')));
    await settle(tester);
    expect(api.calls, isNot(contains('restore')));

    await tester.tap(find.byKey(const Key('settings-restore')));
    await settle(tester);
    await tester.tap(find.byKey(const Key('confirm-destructive')));
    await settle(tester);

    expect(api.calls, contains('restore'));
    expect(find.text('Everything is back as macOS had it.'), findsOneWidget);
    await finish(tester);
  });

  testWidgets('choosing Chinese changes every word at once', (tester) async {
    await pumpApp(tester, FakeIsthmusApi(), page: AppPage.settings);
    expect(find.text('Overview'), findsOneWidget);

    await tester.tap(find.byKey(const Key('settings-language-zh')));
    await settle(tester);

    expect(find.text('概览'), findsOneWidget);
    expect(find.text('Overview'), findsNothing);
    await finish(tester);
  });

  testWidgets('a login item turned off in System Settings offers the pane', (
    tester,
  ) async {
    final api = FakeIsthmusApi(login: LoginItem.requiresApproval);
    await pumpApp(tester, api, page: AppPage.settings);

    await tester.tap(find.byKey(const Key('settings-login-approve')));
    await settle(tester);

    expect(api.calls, contains('open_settings_pane loginItems'));
    await finish(tester);
  });

  testWidgets('the login switch writes the login item', (tester) async {
    final api = FakeIsthmusApi();
    await pumpApp(tester, api, page: AppPage.settings);

    await tester.tap(find.byKey(const Key('settings-login')));
    await settle(tester);

    expect(api.calls, contains('login_item_set true'));
    expect(
      tester.widget<Switch>(find.byKey(const Key('settings-login'))).value,
      isTrue,
    );
    await finish(tester);
  });

  testWidgets('an unavailable login item cannot be switched', (tester) async {
    final api = FakeIsthmusApi(login: LoginItem.unavailable);
    await pumpApp(tester, api, page: AppPage.settings);
    expect(
      tester.widget<Switch>(find.byKey(const Key('settings-login'))).onChanged,
      isNull,
    );
    await finish(tester);
  });
}
