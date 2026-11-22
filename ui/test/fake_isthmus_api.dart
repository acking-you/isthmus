import 'dart:async';

import 'package:isthmus_ui/src/ffi/isthmus_api.dart';

/// A status as Rust would send it, with the fields a test cares about.
Map<String, Object?> statusJson({
  String accessibility = 'granted',
  bool enabled = false,
  bool anotherInstance = false,
  String keyboardTap = 'running',
  String mouseTap = 'running',
  bool secureInput = false,
  Map<String, Object?>? frontApp,
  List<Map<String, Object?>> issues = const [],
  String backend = 'full',
  int keyboardsMapped = 1,
  int miceTuned = 1,
}) => {
  'version': '0.1.0',
  'enabled': enabled,
  'running': enabled,
  'anotherInstance': anotherInstance,
  'permissions': {'accessibility': accessibility},
  'keyboardTap': keyboardTap,
  'mouseTap': mouseTap,
  'secureInput': secureInput,
  'hid': {
    'backend': backend,
    'keyboardsMapped': keyboardsMapped,
    'miceTuned': miceTuned,
  },
  'frontApp': frontApp,
  'hidProfile': 'windows',
  'issues': issues,
};

/// A front app as Rust would send it.
Map<String, Object?> appJson(
  String bundleId,
  String name, {
  String appClass = 'standard',
}) => {
  'bundleId': bundleId,
  'name': name,
  'pid': 42,
  'profile': {'class': appClass, 'traits': <String, Object?>{}},
};

/// A device entry as Rust would send it.
Map<String, Object?> deviceJson(
  String id,
  String kind, {
  String name = 'Device',
  String manufacturer = 'Maker',
  bool connected = true,
  bool viaKvm = false,
  String detectedLayout = 'pc',
  String confidence = 'medium',
  bool hasModeSwitch = false,
}) => {
  'id': id,
  'kind': kind,
  'name': name,
  'manufacturer': manufacturer,
  'vendorId': 1,
  'productId': 2,
  'transport': 'USB',
  'builtIn': false,
  'services': [1],
  'detectedLayout': detectedLayout,
  'layoutConfidence': confidence,
  'viaKvm': viaKvm,
  'hasModeSwitch': hasModeSwitch,
  'connected': connected,
  'layout': detectedLayout,
  'applied': true,
};

/// File sharing's snapshot as Rust would send it.
Map<String, Object?> linkJson({
  String name = 'Studio Mac',
  List<Map<String, Object?>> peers = const [],
  Map<String, Object?>? pairing,
  List<Map<String, Object?>> transfers = const [],
}) => {
  'me': {
    'id': 'fedcba9876543210',
    'name': name,
    'os': 'macos',
    'port': 47821,
    'paths': [
      {'interface': 'en8', 'address': '169.254.149.93', 'kind': 'direct'},
      {'interface': 'en1', 'address': '192.168.21.37', 'kind': 'lan'},
    ],
  },
  'receiveDir': '/Users/someone/Downloads',
  'peers': peers,
  'pairing': pairing,
  'transfers': transfers,
};

/// Another device as Rust would send it.
Map<String, Object?> peerJson(
  String id,
  String name, {
  bool paired = true,
  bool online = true,
  String os = 'windows',
  String? connection = 'direct',
  List<String> routes = const ['direct', 'lan'],
}) => {
  'id': id,
  'name': name,
  'os': os,
  'paired': paired,
  'online': online,
  'connection': paired && online && connection != null
      ? {'kind': connection, 'interface': 'en8', 'address': '169.254.1.2'}
      : null,
  'routes': routes,
};

/// A transfer as Rust would send it.
Map<String, Object?> transferJson(
  String id, {
  String direction = 'send',
  String state = 'running',
  String name = 'Project',
  int total = 2000000,
  int done = 500000,
  String? failure,
  String? savedTo,
}) => {
  'id': id,
  'peerId': '0123456789abcdef',
  'peerName': 'Desk PC',
  'direction': direction,
  'name': name,
  'files': 3,
  'totalBytes': total,
  'doneBytes': done,
  'bytesPerSecond': 1500000,
  'state': state,
  'path': 'direct',
  'failure': failure,
  'savedTo': savedTo,
  'startedAt': 0,
};

/// An [IsthmusApiClient] that answers from memory instead of over FFI.
/// Every method is implemented: the interface makes a forgotten one a
/// compile error rather than a call that escapes to the FFI.
class FakeIsthmusApi implements IsthmusApiClient {
  FakeIsthmusApi({
    Map<String, Object?>? status,
    Map<String, Object?> settings = const {},
    List<Map<String, Object?>> devices = const [],
    List<Map<String, Object?>> runningApps = const [],
    this.login = LoginItem.disabled,
    Map<String, Object?>? link,
  }) : link = link ?? linkJson(),
       current = Status(status ?? statusJson()),
       stored = Settings(settings),
       devicesNow = [for (final d in devices) DeviceEntry.fromJson(d)],
       apps = [for (final a in runningApps) FrontApp.fromJson(a)];

  /// What `status` answers.
  Status current;

  /// The settings as "saved".
  Settings stored;

  /// What `devices` answers.
  List<DeviceEntry> devicesNow;

  /// What `running_apps` answers.
  List<FrontApp> apps;
  LoginItem login;

  /// What `link_status` answers, as JSON a test may edit.
  Map<String, Object?> link;

  /// Every call, in order, so a test asserts what the view asked for rather
  /// than only what it drew. Ops with arguments carry them: `preset_apply
  /// windows all`, `permission_request accessibility`.
  final List<String> calls = [];

  /// The settings each `settings_set` was sent, in order.
  final List<Map<String, Object?>> saved = [];

  /// Ops that fail, with the code they fail with, until removed.
  final Map<String, String> failing = {};

  /// When set, every write waits for it, so a test can see the optimistic
  /// state and queue writes behind each other.
  Completer<void>? writeGate;

  /// When set, `start` waits for it, so a test can act while the service is
  /// still starting, as the tray lets the user do.
  Completer<void>? startGate;

  /// What `permission_request` turns the permission into.
  String grantsTo = 'granted';

  void _check(String op) {
    calls.add(op);
    final code = failing[op.split(' ').first];
    if (code != null) throw IsthmusCallException('$op refused', code: code);
  }

  Future<void> _gate() async {
    final gate = writeGate;
    if (gate != null) await gate.future;
  }

  @override
  Future<Status> start() async {
    final gate = startGate;
    if (gate != null) await gate.future;
    _check('start');
    return current;
  }

  @override
  Future<Status> status() async {
    _check('status');
    return current;
  }

  @override
  Future<Settings> settings() async {
    _check('settings_get');
    return stored;
  }

  @override
  Future<Settings> saveSettings(Settings next) async {
    await _gate();
    _check('settings_set');
    saved.add(next.json);
    stored = Settings(next.json);
    return stored;
  }

  @override
  Future<Settings> applyPreset(Preset preset, PresetTarget target) async {
    await _gate();
    _check('preset_apply ${preset.name} ${target.name}');
    stored = stored.withPreset(preset, target);
    return stored;
  }

  @override
  Future<List<DeviceEntry>> devices() async {
    _check('devices');
    return devicesNow;
  }

  @override
  Future<List<FrontApp>> runningApps() async {
    _check('running_apps');
    return apps;
  }

  @override
  Future<Status> requestPermission(PermissionKind kind) async {
    _check('permission_request ${kind.name}');
    current = Status({
      ...current.json,
      'permissions': {'accessibility': grantsTo},
    });
    return current;
  }

  @override
  Future<void> openSettingsPane(SettingsPane pane) async {
    _check('open_settings_pane ${pane.name}');
  }

  @override
  Future<Settings> restore() async {
    await _gate();
    _check('restore');
    stored = stored.withEnabled(false);
    return stored;
  }

  /// The keyboard whose mapping is lifted now, if any.
  String? probing;

  @override
  Future<void> beginLayoutProbe(String deviceId) async {
    _check('layout_probe_begin $deviceId');
    probing = deviceId;
  }

  @override
  Future<void> endLayoutProbe() async {
    _check('layout_probe_end');
    probing = null;
  }

  @override
  Future<LoginItem> loginItem() async {
    _check('login_item_get');
    return login;
  }

  @override
  Future<LoginItem> setLoginItem(bool enabled) async {
    _check('login_item_set $enabled');
    login = enabled ? LoginItem.enabled : LoginItem.disabled;
    return login;
  }

  @override
  Future<void> shutdown() async {
    _check('shutdown');
  }

  @override
  Future<LinkStatus> linkStatus() async {
    _check('link_status');
    return LinkStatus.fromJson(link);
  }

  @override
  Future<void> linkPair(String peerId) async => _check('link_pair $peerId');

  @override
  Future<void> linkPairCode(String code) async =>
      _check('link_pair_code $code');

  @override
  Future<void> linkPairCancel() async => _check('link_pair_cancel');

  @override
  Future<void> linkUnpair(String peerId) async => _check('link_unpair $peerId');

  @override
  Future<String> linkSend(String peerId, List<String> paths) async {
    _check('link_send $peerId ${paths.join('|')}');
    return 'sent-1';
  }

  @override
  Future<void> linkTransferCancel(String id) async =>
      _check('link_transfer_cancel $id');

  @override
  Future<void> linkTransfersClear() async => _check('link_transfers_clear');

  @override
  Future<LinkStatus> linkRename(String name) async {
    _check('link_rename $name');
    link = {
      ...link,
      'me': {...(link['me'] as Map<String, Object?>), 'name': name},
    };
    return LinkStatus.fromJson(link);
  }

  @override
  Future<void> linkReveal([String? id]) async =>
      _check('link_reveal ${id ?? '-'}');

  /// Keep-awake as the library reports it.
  AwakeStatus awake = const AwakeStatus(
    enabled: false,
    active: false,
    supported: true,
  );

  @override
  Future<AwakeStatus> awakeStatus() async {
    _check('awake_status');
    return awake;
  }

  @override
  Future<AwakeStatus> setAwake(bool enabled) async {
    _check('awake_set $enabled');
    return awake = AwakeStatus(
      enabled: enabled,
      active: enabled,
      supported: true,
    );
  }

  /// Signatures `update_verify` accepts: the message's text.
  final Set<String> validSignatures = {};

  @override
  Future<bool> verifySignature({
    required List<int> message,
    required List<int> signature,
    required List<int> publicKey,
  }) async {
    _check('update_verify');
    return validSignatures.contains(String.fromCharCodes(signature));
  }
}

/// A Mac with something on every card: a missing permission, Secure Input,
/// issues, keyboards through a KVM, a mouse of its own, a trackpad, a rule.
FakeIsthmusApi busyApi() => FakeIsthmusApi(
  link: linkJson(
    peers: [
      peerJson('0123456789abcdef', 'Desk PC'),
      peerJson(
        '2222333344445555',
        'MacBook Air',
        os: 'macos',
        online: false,
        routes: const [],
      ),
      peerJson(
        '6666777788889999',
        'Studio PC',
        paired: false,
        routes: const ['lan'],
      ),
    ],
    pairing: {
      'role': 'incoming',
      'peerId': '6666777788889999',
      'peerName': 'Studio PC',
      'peerOs': 'windows',
      'code': '482913',
      'stage': 'code',
    },
    transfers: [
      transferJson(
        't1',
        name: 'Quarterly report.pdf',
        total: 48000000,
        done: 31000000,
      ),
      transferJson(
        't2',
        direction: 'receive',
        state: 'done',
        name: 'Holiday photos',
        total: 820000000,
        done: 820000000,
        savedTo: '/Users/someone/Downloads/Holiday photos',
      ),
      transferJson('t3', state: 'retrying', name: 'build.zip'),
    ],
  ),
  status: statusJson(
    accessibility: 'denied',
    secureInput: true,
    keyboardTap: 'needsPermission',
    mouseTap: 'failed',
    backend: 'limited',
    frontApp: appJson(
      'com.jetbrains.intellij',
      'IntelliJ IDEA Ultimate Edition',
      appClass: 'nativeCtrl',
    ),
    issues: [
      {'kind': 'karabinerActive'},
      {'kind': 'systemModifierKeys', 'device': 'Keychron K8 Pro Wireless'},
      {'kind': 'foreignPointerTool', 'name': 'Logitech G HUB'},
      {'kind': 'relaunchNeeded'},
      {'kind': 'restorePending', 'count': 3},
    ],
  ),
  settings: {
    'enabled': true,
    'keyboard': {
      'groups': {'editing': false},
      'appRules': [
        {
          'bundleId': 'com.parallels.desktop.console.with.a.long.id',
          'class': 'passthrough',
        },
      ],
    },
    'mouse': {
      'devices': [
        {
          'id': 'mouse:1',
          'name': 'MX Master 3S',
          'profile': {'linesPerNotch': 5},
        },
      ],
    },
  },
  devices: [
    deviceJson(
      'kbd:1',
      'keyboard',
      name: 'HKS201 KVM Switch Emulated Keyboard With A Long Name',
      manufacturer: 'TESmart',
      viaKvm: true,
      confidence: 'low',
    ),
    deviceJson(
      'kbd:2',
      'keyboard',
      name: 'Apple Internal Keyboard / Trackpad',
      detectedLayout: 'mac',
      confidence: 'high',
      connected: false,
    ),
    deviceJson(
      'mouse:1',
      'mouse',
      name: 'MX Master 3S',
      manufacturer: 'Logitech',
    ),
    deviceJson('tp:1', 'trackpad', name: 'Magic Trackpad'),
  ],
  login: LoginItem.requiresApproval,
);
