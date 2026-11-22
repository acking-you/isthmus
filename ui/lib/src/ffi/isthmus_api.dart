import 'dart:convert';

import 'package:isthmus_ui/src/ffi/isthmus_service.dart';
import 'package:isthmus_ui/src/ffi/models.dart';

export 'package:isthmus_ui/src/ffi/models.dart';

/// The error codes Rust sends (`docs/architecture.md`). Callers branch on
/// these, never on the message, which is a sentence for a person.
abstract final class ErrorCode {
  static const invalidArgument = 'INVALID_ARGUMENT';
  static const notRunning = 'NOT_RUNNING';
  static const anotherInstance = 'ANOTHER_INSTANCE';
  static const permissionMissing = 'PERMISSION_MISSING';
  static const unsupported = 'UNSUPPORTED';
  static const io = 'IO';
  static const busy = 'BUSY';
  static const unavailable = 'UNAVAILABLE';
  static const internal = 'INTERNAL';

  static const _known = {
    invalidArgument,
    notRunning,
    anotherInstance,
    permissionMissing,
    unsupported,
    io,
    busy,
    unavailable,
    internal,
  };

  /// [raw], or [internal] for a code this build does not know: a newer
  /// library's code must not crash an older app.
  static String parse(Object? raw) =>
      raw is String && _known.contains(raw) ? raw : internal;
}

/// A failed call: the sentence for a person, the code to branch on.
class IsthmusCallException implements Exception {
  const IsthmusCallException(this.message, {required this.code});

  /// What went wrong, for a person; never parsed.
  final String message;

  /// One of [ErrorCode]; an unknown code reads as [ErrorCode.internal].
  final String code;

  @override
  String toString() => message;
}

/// Everything the UI asks of the Rust side: one method per op of
/// `isthmus_call_json` (the "Ops" table in `docs/architecture.md`).
///
/// An interface rather than a subclassable base class on purpose. A fake
/// that forgets a method fails to compile here; a fake that inherited from
/// the real class would quietly fall through to the FFI instead.
abstract interface class IsthmusApiClient {
  /// `start`: starts the service once; later calls return the status.
  Future<Status> start();

  /// `status`.
  Future<Status> status();

  /// `settings_get`.
  Future<Settings> settings();

  /// `settings_set`: answers the settings as saved (normalized).
  Future<Settings> saveSettings(Settings settings);

  /// `preset_apply`.
  Future<Settings> applyPreset(Preset preset, PresetTarget target);

  /// `devices`.
  Future<List<DeviceEntry>> devices();

  /// `running_apps`.
  Future<List<FrontApp>> runningApps();

  /// `permission_request`: may show macOS's own prompt.
  Future<Status> requestPermission(PermissionKind kind);

  /// `open_settings_pane`.
  Future<void> openSettingsPane(SettingsPane pane);

  /// `restore`: switches Isthmus off and puts everything back.
  Future<Settings> restore();

  /// `layout_probe_begin`: lifts one keyboard's mapping (for at most 20 s)
  /// so the key beside the space bar arrives as the keyboard sends it.
  Future<void> beginLayoutProbe(String deviceId);

  /// `layout_probe_end`: maps the probed keyboard again.
  Future<void> endLayoutProbe();

  /// `login_item_get`.
  Future<LoginItem> loginItem();

  /// `login_item_set`.
  Future<LoginItem> setLoginItem(bool enabled);

  /// `shutdown`: puts everything back and stops. The Runner's quit hook calls
  /// the library's own shutdown, so the app does not need this to quit.
  Future<void> shutdown();

  /// `link_status`: file sharing's snapshot.
  Future<LinkStatus> linkStatus();

  /// `link_pair`: asks [peerId] to pair; it shows a code.
  Future<void> linkPair(String peerId);

  /// `link_pair_code`: the code read off the other device.
  Future<void> linkPairCode(String code);

  /// `link_pair_accept`: accepts the pairing another device asked for.
  Future<void> linkPairAccept();

  /// `link_pair_cancel`: cancels, declines or dismisses the pairing shown.
  Future<void> linkPairCancel();

  /// `link_unpair`.
  Future<void> linkUnpair(String peerId);

  /// `link_send`: answers the transfer's id.
  Future<String> linkSend(String peerId, List<String> paths);

  /// `link_transfer_cancel`.
  Future<void> linkTransferCancel(String id);

  /// `link_transfers_clear`: drops finished transfers from the list.
  Future<void> linkTransfersClear();

  /// `link_rename`: answers the new snapshot.
  Future<LinkStatus> linkRename(String name);

  /// `link_reveal`: shows a received transfer, or without [id] the receive
  /// folder, in the file manager.
  Future<void> linkReveal([String? id]);

  /// `link_clipboard_set`: shares the clipboard with paired devices, or
  /// stops.
  Future<LinkStatus> linkClipboardSet(bool enabled);

  /// `clipboard_request_access`: raises macOS's clipboard prompt once.
  Future<void> clipboardRequestAccess();

  /// `awake_status`: keeping the computer awake, chosen and in effect.
  Future<AwakeStatus> awakeStatus();

  /// `awake_set`: turns keeping the computer awake on or off.
  Future<AwakeStatus> setAwake(bool enabled);

  /// `update_verify`: whether [signature] is [publicKey]'s Ed25519
  /// signature over exactly [message]. Needs no running service.
  Future<bool> verifySignature({
    required List<int> message,
    required List<int> signature,
    required List<int> publicKey,
  });
}

/// [IsthmusApiClient] over the FFI.
class IsthmusApi implements IsthmusApiClient {
  IsthmusApi._();

  static final IsthmusApi _instance = IsthmusApi._();

  /// The one client; it holds no state of its own.
  factory IsthmusApi() => _instance;

  final IsthmusService _service = IsthmusService();

  /// One operation's `data`, or its failure as an [IsthmusCallException].
  Future<Object?> _call(
    String op, [
    Map<String, Object?> args = const {},
  ]) async {
    final result = await _service.call({'op': op, ...args});
    if (result['success'] != true) {
      final message = result['message'];
      throw IsthmusCallException(
        message is String && message.isNotEmpty ? message : 'The call failed',
        code: ErrorCode.parse(result['code']),
      );
    }
    return result['data'];
  }

  Map<String, Object?> _object(Object? data) =>
      data is Map ? data.cast<String, Object?>() : const {};

  List<Map<String, Object?>> _objects(Object? data) => [
    if (data is List)
      for (final item in data)
        if (item is Map) item.cast<String, Object?>(),
  ];

  @override
  Future<Status> start() async =>
      Status.fromJson(_object(await _call('start')));

  @override
  Future<Status> status() async =>
      Status.fromJson(_object(await _call('status')));

  @override
  Future<Settings> settings() async =>
      Settings(_object(await _call('settings_get')));

  @override
  Future<Settings> saveSettings(Settings settings) async => Settings(
    _object(await _call('settings_set', {'settings': settings.json})),
  );

  @override
  Future<Settings> applyPreset(Preset preset, PresetTarget target) async =>
      Settings(
        _object(
          await _call('preset_apply', {
            'preset': preset.name,
            'target': target.name,
          }),
        ),
      );

  @override
  Future<List<DeviceEntry>> devices() async => [
    for (final item in _objects(await _call('devices')))
      DeviceEntry.fromJson(item),
  ];

  @override
  Future<List<FrontApp>> runningApps() async => [
    for (final item in _objects(await _call('running_apps')))
      FrontApp.fromJson(item),
  ];

  @override
  Future<Status> requestPermission(PermissionKind kind) async =>
      Status.fromJson(
        _object(await _call('permission_request', {'kind': kind.name})),
      );

  @override
  Future<void> openSettingsPane(SettingsPane pane) =>
      _call('open_settings_pane', {'pane': pane.name});

  @override
  Future<Settings> restore() async => Settings(_object(await _call('restore')));

  @override
  Future<void> beginLayoutProbe(String deviceId) =>
      _call('layout_probe_begin', {'deviceId': deviceId});

  @override
  Future<void> endLayoutProbe() => _call('layout_probe_end');

  @override
  Future<LoginItem> loginItem() async =>
      LoginItem.parse(_object(await _call('login_item_get'))['state']);

  @override
  Future<LoginItem> setLoginItem(bool enabled) async => LoginItem.parse(
    _object(await _call('login_item_set', {'enabled': enabled}))['state'],
  );

  @override
  Future<void> shutdown() => _call('shutdown');

  @override
  Future<LinkStatus> linkStatus() async =>
      LinkStatus.fromJson(_object(await _call('link_status')));

  @override
  Future<void> linkPair(String peerId) =>
      _call('link_pair', {'peerId': peerId});

  @override
  Future<void> linkPairCode(String code) =>
      _call('link_pair_code', {'code': code});

  @override
  Future<void> linkPairAccept() => _call('link_pair_accept');

  @override
  Future<void> linkPairCancel() => _call('link_pair_cancel');

  @override
  Future<void> linkUnpair(String peerId) =>
      _call('link_unpair', {'peerId': peerId});

  @override
  Future<String> linkSend(String peerId, List<String> paths) async {
    final data = _object(
      await _call('link_send', {'peerId': peerId, 'paths': paths}),
    );
    final id = data['id'];
    return id is String ? id : '';
  }

  @override
  Future<void> linkTransferCancel(String id) =>
      _call('link_transfer_cancel', {'id': id});

  @override
  Future<void> linkTransfersClear() => _call('link_transfers_clear');

  @override
  Future<LinkStatus> linkRename(String name) async =>
      LinkStatus.fromJson(_object(await _call('link_rename', {'name': name})));

  @override
  Future<void> linkReveal([String? id]) => _call('link_reveal', {'id': ?id});

  @override
  Future<LinkStatus> linkClipboardSet(bool enabled) async =>
      LinkStatus.fromJson(
        _object(await _call('link_clipboard_set', {'enabled': enabled})),
      );

  @override
  Future<void> clipboardRequestAccess() => _call('clipboard_request_access');

  @override
  Future<AwakeStatus> awakeStatus() async =>
      AwakeStatus.fromJson(_object(await _call('awake_status')));

  @override
  Future<AwakeStatus> setAwake(bool enabled) async => AwakeStatus.fromJson(
    _object(await _call('awake_set', {'enabled': enabled})),
  );

  @override
  Future<bool> verifySignature({
    required List<int> message,
    required List<int> signature,
    required List<int> publicKey,
  }) async =>
      _object(
        await _call('update_verify', {
          'message': base64Encode(message),
          'signature': base64Encode(signature),
          'publicKey': base64Encode(publicKey),
        }),
      )['valid'] ==
      true;
}
