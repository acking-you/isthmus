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
  static const internal = 'INTERNAL';

  static const _known = {
    invalidArgument,
    notRunning,
    anotherInstance,
    permissionMissing,
    unsupported,
    io,
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

  /// `login_item_get`.
  Future<LoginItem> loginItem();

  /// `login_item_set`.
  Future<LoginItem> setLoginItem(bool enabled);

  /// `shutdown`: puts everything back and stops. The Runner's quit hook calls
  /// the library's own shutdown, so the app does not need this to quit.
  Future<void> shutdown();
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
  Future<LoginItem> loginItem() async =>
      LoginItem.parse(_object(await _call('login_item_get'))['state']);

  @override
  Future<LoginItem> setLoginItem(bool enabled) async => LoginItem.parse(
    _object(await _call('login_item_set', {'enabled': enabled}))['state'],
  );

  @override
  Future<void> shutdown() => _call('shutdown');
}
