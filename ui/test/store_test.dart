import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/ffi/isthmus_api.dart';
import 'package:isthmus_ui/src/state/app_store.dart';

import 'fake_isthmus_api.dart';

/// A store over [api] that does not poll, with its failures collected.
(AppStore, List<(StoreAction, IsthmusCallException)>) _store(
  FakeIsthmusApi api, {
  Stream<Change>? changes,
}) {
  final failures = <(StoreAction, IsthmusCallException)>[];
  final store = AppStore(
    api: api,
    changes: changes ?? const Stream.empty(),
    pollInterval: null,
    onFailure: (action, error) => failures.add((action, error)),
  );
  addTearDown(store.dispose);
  return (store, failures);
}

void main() {
  /// A newer library's keys reach Rust again untouched: the store edits a
  /// copy of the raw map, never a model that forgets what it cannot name.
  test('a settings round trip keeps keys this app does not know', () async {
    final api = FakeIsthmusApi(
      settings: {
        'version': 1,
        'enabled': false,
        'futureThing': {'a': 1},
        'keyboard': {
          'futureKey': 3,
          'devices': [
            {'id': 'kbd:1', 'layout': 'pc', 'futureField': true},
          ],
        },
        'mouse': {
          'defaults': {'linesPerNotch': 5, 'futureKnob': 'x'},
        },
      },
    );
    final (store, _) = _store(api);
    await store.start();

    expect(await store.update((s) => s.withEnabled(true)), isTrue);
    expect(
      await store.update(
        (s) =>
            s.withKeyboardDevice('kbd:1', name: 'K8', layout: LayoutChoice.mac),
      ),
      isTrue,
    );
    expect(
      await store.update(
        (s) => s.withMouseDefaults(s.mouseDefaults.copyWith(linesPerNotch: 7)),
      ),
      isTrue,
    );

    final saved = api.saved.last;
    expect(saved['enabled'], isTrue);
    expect(saved['futureThing'], {'a': 1});
    final keyboard = saved['keyboard']! as Map;
    expect(keyboard['futureKey'], 3);
    final device = (keyboard['devices'] as List).single as Map;
    expect(device['futureField'], isTrue);
    expect(device['layout'], 'mac');
    final defaults = (saved['mouse']! as Map)['defaults'] as Map;
    expect(defaults['futureKnob'], 'x');
    expect(defaults['linesPerNotch'], 7);
  });

  /// A refused write leaves the screen as Rust has it and says why, with
  /// the code to branch on.
  test('a failed write rolls back and reports its code', () async {
    final api = FakeIsthmusApi()..failing['settings_set'] = ErrorCode.io;
    final (store, failures) = _store(api);
    await store.start();

    final write = store.setEnabled(true);
    // Optimistic: on at once.
    expect(store.settings.enabled, isTrue);
    expect(await write, isFalse);

    expect(store.settings.enabled, isFalse);
    expect(failures.single.$1, StoreAction.saveSettings);
    expect(failures.single.$2.code, ErrorCode.io);
  });

  /// Two quick edits: the second is sent against what the first saved, so
  /// it never carries a stale copy that undoes the first.
  test('queued writes build on the writes before them', () async {
    final api = FakeIsthmusApi()..writeGate = Completer<void>();
    final (store, _) = _store(api);
    await store.start();

    final first = store.update(
      (s) => s.withGroup(ShortcutGroup.editing, false),
    );
    final second = store.update(
      (s) => s.withGroup(ShortcutGroup.terminal, false),
    );
    expect(store.settings.groupEnabled(ShortcutGroup.editing), isFalse);
    expect(store.settings.groupEnabled(ShortcutGroup.terminal), isFalse);

    api.writeGate!.complete();
    expect(await first, isTrue);
    expect(await second, isTrue);

    expect(api.saved, hasLength(2));
    final groups = (api.saved.last['keyboard']! as Map)['groups'];
    expect(groups, {'editing': false, 'terminal': false});
  });

  /// A failure drops only its own change; the next write still goes, and
  /// does not resend the refused one.
  test('a failed write drops only its own change', () async {
    final api = FakeIsthmusApi()..writeGate = Completer<void>();
    final (store, _) = _store(api);
    await store.start();

    api.failing['settings_set'] = ErrorCode.invalidArgument;
    final first = store.update(
      (s) => s.withGroup(ShortcutGroup.editing, false),
    );
    api.writeGate!.complete();
    expect(await first, isFalse);

    api.failing.remove('settings_set');
    expect(
      await store.update((s) => s.withGroup(ShortcutGroup.system, false)),
      isTrue,
    );
    expect((api.saved.last['keyboard']! as Map)['groups'], {'system': false});
  });

  /// A write made while the service still starts (the tray's switch works
  /// from launch) waits for the first settings read and edits the file, never
  /// the defaults, which would wipe the user's devices, rules and profiles.
  test('a write before the first settings read keeps the saved file', () async {
    final api = FakeIsthmusApi(
      settings: {
        'enabled': false,
        'keyboard': {
          'appRules': [
            {'bundleId': 'com.example.vm', 'class': 'passthrough'},
          ],
        },
        'mouse': {
          'defaults': {'linesPerNotch': 5},
        },
      },
    )..startGate = Completer<void>();
    final (store, _) = _store(api);
    final starting = store.start();

    final write = store.setEnabled(true);
    // Optimistic: on at once, though nothing has been read or sent yet.
    expect(store.settings.enabled, isTrue);
    await Future<void>.delayed(Duration.zero);
    expect(api.saved, isEmpty);

    api.startGate!.complete();
    await starting;
    expect(await write, isTrue);

    final saved = api.saved.single;
    expect(saved['enabled'], isTrue);
    final rule = ((saved['keyboard']! as Map)['appRules'] as List).single;
    expect((rule as Map)['bundleId'], 'com.example.vm');
    expect(((saved['mouse']! as Map)['defaults'] as Map)['linesPerNotch'], 5);
  });

  /// After a failed first read, an edit reads the settings before it is
  /// sent. If they still cannot be read, nothing is sent and the read's own
  /// code is reported.
  test('an edit after a failed settings read reads them first', () async {
    final api = FakeIsthmusApi(
      settings: {
        'enabled': true,
        'keyboard': {
          'groups': {'editing': false},
        },
      },
    )..failing['settings_get'] = ErrorCode.io;
    final (store, failures) = _store(api);
    await store.start();
    expect(await store.ready, isFalse);

    Future<bool> edit() =>
        store.update((s) => s.withGroup(ShortcutGroup.terminal, false));
    expect(await edit(), isFalse);
    expect(api.saved, isEmpty);
    expect(failures.single.$1, StoreAction.saveSettings);
    expect(failures.single.$2.code, ErrorCode.io);

    api.failing.remove('settings_get');
    expect(await edit(), isTrue);
    final saved = api.saved.single;
    expect(saved['enabled'], isTrue);
    expect((saved['keyboard']! as Map)['groups'], {
      'editing': false,
      'terminal': false,
    });
  });

  /// Putting everything back is an op Rust applies to its own copy, so it
  /// never waits on a settings read that keeps failing.
  test('restore goes even when the settings cannot be read', () async {
    final api = FakeIsthmusApi(settings: {'enabled': true})
      ..failing['settings_get'] = ErrorCode.io;
    final (store, failures) = _store(api);
    await store.start();

    expect(await store.restore(), isTrue);
    expect(api.calls, contains('restore'));
    expect(store.settings.enabled, isFalse);
    expect(failures, isEmpty);
  });

  /// The preset is sent as the preset, so Rust's own apply_preset decides;
  /// the screen shows the Dart mirror of it until Rust answers.
  test('a preset is sent as an op, not as edited settings', () async {
    final api = FakeIsthmusApi();
    final (store, _) = _store(api);
    await store.start();

    expect(await store.applyPreset(Preset.mac, PresetTarget.mouse), isTrue);
    expect(api.calls, contains('preset_apply mac mouse'));
    expect(api.saved, isEmpty);
    expect(store.settings.mouseStyle, ProfileStyle.mac);
    expect(store.settings.keyboardStyle, ProfileStyle.windows);
  });

  /// A failed start is kept as its error, and the rest still loads, so the
  /// overview can say what went wrong.
  test('a failed start keeps its code and still reads the rest', () async {
    final api = FakeIsthmusApi(devices: [deviceJson('kbd:1', 'keyboard')])
      ..failing['start'] = ErrorCode.anotherInstance;
    final (store, _) = _store(api);
    await store.start();

    expect(await store.ready, isFalse);
    expect(store.anotherInstance, isTrue);
    expect(store.devices, hasLength(1));
    expect(api.calls, contains('status'));
  });

  /// Hints name what went stale; a burst becomes one read of each.
  test('change hints re-read what they name, once per burst', () async {
    final hints = StreamController<Change>();
    addTearDown(hints.close);
    final api = FakeIsthmusApi();
    final (store, _) = _store(api, changes: hints.stream);
    await store.start();
    api.calls.clear();

    api.devicesNow = [DeviceEntry.fromJson(deviceJson('m:1', 'mouse'))];
    hints
      ..add(const Change(ChangeKind.devices, seq: 1))
      ..add(const Change(ChangeKind.devices, seq: 2));
    await Future<void>.delayed(const Duration(milliseconds: 250));

    expect(api.calls, ['devices']);
    expect(store.devicesOf(DeviceKind.mouse), hasLength(1));
  });

  /// Polling only while someone can see the window: Secure Input and the
  /// front app change without a hint.
  test('the status is polled only while the window is visible', () async {
    final api = FakeIsthmusApi();
    final store = AppStore(
      api: api,
      changes: const Stream.empty(),
      pollInterval: const Duration(milliseconds: 20),
    );
    addTearDown(store.dispose);
    await store.start();
    api.calls.clear();

    await Future<void>.delayed(const Duration(milliseconds: 80));
    expect(api.calls, isEmpty);

    store.setVisible(true);
    await Future<void>.delayed(const Duration(milliseconds: 80));
    expect(api.calls.where((c) => c == 'status').length, greaterThan(1));

    store.setVisible(false);
    await Future<void>.delayed(const Duration(milliseconds: 10));
    api.calls.clear();
    await Future<void>.delayed(const Duration(milliseconds: 80));
    expect(api.calls, isEmpty);
  });

  /// Opening the Accessibility pane puts the drag card up only while the
  /// grant is still missing; a request answered "granted" needs no card.
  test(
    'the drag card is asked for only while Accessibility is missing',
    () async {
      final api = FakeIsthmusApi()..grantsTo = 'denied';
      final (store, _) = _store(api);
      var asked = 0;
      store.onPermissionRequested = () => asked++;
      await store.start();

      await store.requestPermission();
      expect(asked, 1);

      api.grantsTo = 'granted';
      await store.requestPermission();
      expect(asked, 1);
    },
  );
}
