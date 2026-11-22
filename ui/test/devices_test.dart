import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/ffi/isthmus_api.dart';
import 'package:isthmus_ui/src/pages/devices_page.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:isthmus_ui/src/state/link_store.dart';
import 'package:material_ui/material_ui.dart';

import 'fake_isthmus_api.dart';
import 'support.dart';

const _pc = '0123456789abcdef';
const _laptop = '1111222233334444';

LinkStore _link(WidgetTester tester) =>
    AppScope.of(tester.element(find.byType(DevicesPage))).store.link;

/// Tells the app file sharing changed, as Rust's hint would.
Future<void> _hint(WidgetTester tester) async {
  changes.add(const Change(ChangeKind.link));
  await settle(tester);
}

void main() {
  testWidgets('dropped files wait, and Send hands them to that device', (
    tester,
  ) async {
    final api = FakeIsthmusApi(
      link: linkJson(peers: [peerJson(_pc, 'Desk PC')]),
    );
    await pumpApp(tester, api, page: AppPage.devices);
    final send = find.byKey(const Key('peer-send-$_pc'));
    expect(tester.widget<ButtonStyleButton>(send).onPressed, isNull);

    _link(tester)
        .stage(['/Users/me/Trip', '/Users/me/notes.txt', '/Users/me/Trip']);
    await settle(tester);
    expect(find.text('Trip'), findsOneWidget);
    expect(find.text('Send 2 items'), findsOneWidget);

    await tester.tap(send);
    await settle(tester);
    expect(
      api.calls,
      contains('link_send $_pc /Users/me/Trip|/Users/me/notes.txt'),
    );
    expect(_link(tester).staged, isEmpty);
    await finish(tester);
  });

  testWidgets('an offline device cannot be sent to', (tester) async {
    final api = FakeIsthmusApi(
      link: linkJson(peers: [peerJson(_pc, 'Desk PC', online: false)]),
    );
    await pumpApp(tester, api, page: AppPage.devices);
    _link(tester).stage(['/Users/me/a.txt']);
    await settle(tester);
    expect(find.text('Offline'), findsOneWidget);
    expect(
      tester
          .widget<ButtonStyleButton>(find.byKey(const Key('peer-send-$_pc')))
          .onPressed,
      isNull,
    );
    await finish(tester);
  });

  testWidgets('a nearby device pairs with the code typed here', (tester) async {
    final api = FakeIsthmusApi(
      link: linkJson(
        peers: [peerJson(_laptop, 'Laptop', paired: false, os: 'macos')],
      ),
    );
    await pumpApp(tester, api, page: AppPage.devices);
    await tester.tap(find.byKey(const Key('peer-pair-$_laptop')));
    await settle(tester);
    expect(api.calls, contains('link_pair $_laptop'));

    api.link = linkJson(
      peers: [peerJson(_laptop, 'Laptop', paired: false, os: 'macos')],
      pairing: {
        'role': 'outgoing',
        'peerId': _laptop,
        'peerName': 'Laptop',
        'peerOs': 'macos',
        'stage': 'code',
      },
    );
    await _hint(tester);
    expect(find.text('Enter the code shown on Laptop'), findsOneWidget);
    await tester.enterText(
      find.byKey(const Key('pairing-code-field')),
      '123 456',
    );
    await tester.tap(find.byKey(const Key('pairing-submit')));
    await settle(tester);
    expect(api.calls, contains('link_pair_code 123456'));
    await finish(tester);
  });

  testWidgets('a pairing request shows its code until declined', (
    tester,
  ) async {
    final api = FakeIsthmusApi(
      link: linkJson(
        pairing: {
          'role': 'incoming',
          'peerId': _pc,
          'peerName': 'Desk PC',
          'peerOs': 'windows',
          'code': '482913',
          'stage': 'code',
        },
      ),
    );
    await pumpApp(tester, api, page: AppPage.devices);
    expect(find.text('Desk PC wants to pair'), findsOneWidget);
    expect(find.text('482 913'), findsOneWidget);
    await tester.tap(find.byKey(const Key('pairing-cancel')));
    await settle(tester);
    expect(api.calls, contains('link_pair_cancel'));
    await finish(tester);
  });

  testWidgets('a wrong code says so and the pairing can be tried again', (
    tester,
  ) async {
    final api = FakeIsthmusApi(
      link: linkJson(
        pairing: {
          'role': 'outgoing',
          'peerId': _laptop,
          'peerName': 'Laptop',
          'peerOs': 'macos',
          'stage': 'failed',
          'failure': 'wrongCode',
        },
      ),
    );
    await pumpApp(tester, api, page: AppPage.devices);
    expect(
      find.text(
        'The code did not match. Try again: a new code shows each time.',
      ),
      findsOneWidget,
    );
    await tester.tap(find.byKey(const Key('pairing-retry')));
    await settle(tester);
    expect(api.calls, contains('link_pair $_laptop'));
    await finish(tester);
  });

  testWidgets('transfers show their path, and received files can be shown', (
    tester,
  ) async {
    final api = FakeIsthmusApi(
      link: linkJson(
        peers: [peerJson(_pc, 'Desk PC')],
        transfers: [
          transferJson('t-run'),
          transferJson(
            't-in',
            direction: 'receive',
            state: 'done',
            name: 'Photos',
            done: 2000000,
            savedTo: '/Users/someone/Downloads/Photos',
          ),
          transferJson('t-fail', state: 'failed', failure: 'disk'),
        ],
      ),
    );
    await pumpApp(tester, api, page: AppPage.devices);
    await tester.scrollUntilVisible(
      find.byKey(const Key('transfer-t-fail')),
      200,
      scrollable: find
          .descendant(
            of: find.byKey(const Key('devices-list')),
            matching: find.byType(Scrollable),
          )
          .first,
    );
    expect(
      find.text('500 KB of 2.0 MB · 1.5 MB/s · Direct cable'),
      findsOneWidget,
    );
    expect(
      find.text('The other computer could not save the files'),
      findsOneWidget,
    );
    await tester.tap(find.byKey(const Key('transfer-reveal-t-in')));
    await tester.tap(find.byKey(const Key('transfer-cancel-t-run')));
    await tester.tap(find.byKey(const Key('transfers-clear')));
    await settle(tester);
    expect(
      api.calls,
      containsAllInOrder([
        'link_reveal t-in',
        'link_transfer_cancel t-run',
        'link_transfers_clear',
      ]),
    );
    await finish(tester);
  });

  testWidgets('file sharing held by another copy says so', (tester) async {
    final api = FakeIsthmusApi()
      ..failing['link_status'] = ErrorCode.anotherInstance;
    await pumpApp(tester, api, page: AppPage.devices);
    expect(find.byKey(const Key('devices-unavailable')), findsOneWidget);
    expect(
      find.textContaining('Another Isthmus on this computer is sharing files'),
      findsOneWidget,
    );
    await finish(tester);
  });

  group('LinkStore', () {
    /// A receive that finishes is announced once, not again on every read
    /// after it, and not for one already done at the first read.
    test('announces a finished receive once', () async {
      final api = FakeIsthmusApi(
        link: linkJson(
          transfers: [
            transferJson('old', direction: 'receive', state: 'done'),
            transferJson('new', direction: 'receive'),
          ],
        ),
      );
      final store = LinkStore(api: api);
      final received = <String>[];
      store.onReceived = (transfer) => received.add(transfer.id);
      await store.refresh();
      api.link = linkJson(
        transfers: [
          transferJson('old', direction: 'receive', state: 'done'),
          transferJson('new', direction: 'receive', state: 'done'),
        ],
      );
      await store.refresh();
      await store.refresh();
      expect(received, ['new']);
      store.dispose();
    });

    /// An incoming pairing is announced when its code first shows, so the
    /// app can bring the window up, and not on every progress read.
    test('announces a pairing request when its code appears', () async {
      final api = FakeIsthmusApi();
      final store = LinkStore(api: api);
      var asked = 0;
      store.onPairingRequest = (_) => asked++;
      await store.refresh();
      api.link = linkJson(
        pairing: {
          'role': 'incoming',
          'peerId': _pc,
          'peerName': 'Desk PC',
          'code': '000111',
          'stage': 'code',
        },
      );
      await store.refresh();
      api.link = {
        ...api.link,
        'transfers': [transferJson('t')],
      };
      await store.refresh();
      expect(asked, 1);
      store.dispose();
    });

    /// A send that fails keeps the files waiting, so nothing is lost.
    test('a failed send keeps the files waiting', () async {
      final api = FakeIsthmusApi()
        ..failing['link_send'] = ErrorCode.unavailable;
      final failures = <String>[];
      final store = LinkStore(
        api: api,
        onFailure: (_, error) => failures.add(error.code),
      );
      store.stage(['/a']);
      expect(await store.sendStaged(_pc), isFalse);
      expect(store.staged, ['/a']);
      expect(failures, [ErrorCode.unavailable]);
      store.dispose();
    });
  });

  /// A newer library's values read as something this app can show.
  test('a snapshot with values from a later version still reads', () {
    final status = LinkStatus.fromJson({
      ...linkJson(
        peers: [
          peerJson(_pc, 'PC', os: 'haiku', routes: ['satellite']),
        ],
        transfers: [transferJson('t', state: 'paused', failure: 'gremlins')],
      ),
      'clipboard': true,
    });
    expect(status.peers.single.os, PeerOs.other);
    expect(status.peers.single.routes, [PathKind.lan]);
    expect(status.transfers.single.state, TransferState.connecting);
    expect(status.transfers.single.failure, TransferFailure.other);
    expect(formatBytes(1536000), '1.5 MB');
    expect(baseName(r'C:\Users\me\Trip'), 'Trip');
  });
}
