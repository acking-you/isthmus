/// The JSON shapes `isthmus_call_json` answers with, as Dart values.
///
/// They mirror the serde forms of the Rust types in
/// `crates/isthmus-input/src/{settings,status,context,device}.rs` (camelCase).
/// Every reader is tolerant: a missing field takes the Rust default, and an
/// enum value this build does not know reads as the variant Rust's
/// `#[serde(other)]` picks, so a newer library never crashes an older app.
///
/// [Settings] is different from the rest: it is written back, so it keeps the
/// raw JSON map it was read from and every edit is made on a copy of that map.
/// Keys a newer library added survive a round trip through this app.
library;

/// A JSON object, or an empty one when [value] is anything else.
Map<String, Object?> _map(Object? value) =>
    value is Map ? value.cast<String, Object?>() : const <String, Object?>{};

/// A JSON array, or an empty one when [value] is anything else.
List<Object?> _list(Object? value) =>
    value is List ? value.cast<Object?>() : const <Object?>[];

String _string(Object? value, [String fallback = '']) =>
    value is String ? value : fallback;

bool _bool(Object? value, bool fallback) => value is bool ? value : fallback;

int _int(Object? value, int fallback) =>
    value is num ? value.toInt() : fallback;

/// A deep copy of a decoded JSON value, so an edit never reaches the map the
/// previous [Settings] still holds.
Object? _deepCopy(Object? value) => switch (value) {
  Map() => <String, Object?>{
    for (final MapEntry(:key, :value) in value.entries)
      '$key': _deepCopy(value),
  },
  List() => [for (final item in value) _deepCopy(item)],
  _ => value,
};

/// Whether two decoded JSON values are equal, member by member.
bool jsonEquals(Object? a, Object? b) {
  if (a is Map && b is Map) {
    if (a.length != b.length) return false;
    for (final MapEntry(:key, :value) in a.entries) {
      if (!b.containsKey(key) || !jsonEquals(value, b[key])) return false;
    }
    return true;
  }
  if (a is List && b is List) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (!jsonEquals(a[i], b[i])) return false;
    }
    return true;
  }
  return a == b;
}

// ---------------------------------------------------------------------------
// Settings enums.

/// How a keyboard behaves (`KeyboardStyle`). Unknown reads as [mac], which
/// changes nothing.
enum KeyboardStyle {
  windows,
  mac;

  static KeyboardStyle parse(Object? raw) =>
      raw == 'windows' ? KeyboardStyle.windows : KeyboardStyle.mac;
}

/// Which modifier row a keyboard has, when detection guesses wrong
/// (`LayoutChoice`). Unknown reads as [auto].
enum LayoutChoice {
  auto,
  mac,
  pc;

  static LayoutChoice parse(Object? raw) => switch (raw) {
    'mac' => LayoutChoice.mac,
    'pc' => LayoutChoice.pc,
    _ => LayoutChoice.auto,
  };
}

/// What the F1–F12 row sends (`FunctionKeys`). Unknown reads as [system].
enum FunctionKeys {
  standard,
  system;

  static FunctionKeys parse(Object? raw) =>
      raw == 'standard' ? FunctionKeys.standard : FunctionKeys.system;
}

/// How the pointer moves (`PointerMode`). Unknown reads as [system].
enum PointerMode {
  windows,
  system;

  static PointerMode parse(Object? raw) =>
      raw == 'windows' ? PointerMode.windows : PointerMode.system;
}

/// A preset name, or neither (`ProfileStyle`).
enum ProfileStyle { windows, mac, custom }

/// A one-click preset (`Preset`).
enum Preset { windows, mac }

/// What a preset is applied to (`PresetTarget`).
enum PresetTarget { all, keyboard, mouse }

/// How an app is treated (`AppClass`). Unknown reads as [standard], as in
/// Rust.
enum AppClass {
  standard,
  terminal,
  nativeCtrl,
  passthrough;

  static AppClass parse(Object? raw) => switch (raw) {
    'terminal' => AppClass.terminal,
    'nativeCtrl' => AppClass.nativeCtrl,
    'passthrough' => AppClass.passthrough,
    _ => AppClass.standard,
  };
}

/// The shortcut groups the event tap translates (`ShortcutGroups` fields).
/// [key] is the JSON field name.
enum ShortcutGroup {
  editing('editing'),
  navigation('navigation'),
  windows('windows'),
  startMenu('startMenu'),
  system('system'),
  inputSource('inputSource'),
  appFixes('appFixes'),
  terminal('terminal');

  const ShortcutGroup(this.key);

  final String key;
}

// ---------------------------------------------------------------------------
// Settings.

/// One keyboard's own choices (`KeyboardDeviceSettings`).
class KeyboardDeviceSettings {
  const KeyboardDeviceSettings({
    required this.id,
    this.name = '',
    this.style = KeyboardStyle.windows,
    this.layout = LayoutChoice.auto,
    this.functionKeys = FunctionKeys.standard,
  });

  factory KeyboardDeviceSettings.fromJson(Map<String, Object?> json) =>
      KeyboardDeviceSettings(
        id: _string(json['id']),
        name: _string(json['name']),
        // The Rust default for a missing field is Windows and Standard.
        style: json.containsKey('style')
            ? KeyboardStyle.parse(json['style'])
            : KeyboardStyle.windows,
        layout: LayoutChoice.parse(json['layout']),
        functionKeys: json.containsKey('functionKeys')
            ? FunctionKeys.parse(json['functionKeys'])
            : FunctionKeys.standard,
      );

  final String id;

  /// The product name when the entry was made, for a keyboard that is not
  /// connected now.
  final String name;
  final KeyboardStyle style;
  final LayoutChoice layout;
  final FunctionKeys functionKeys;
}

/// The user's class for one app (`AppRule`).
class AppRule {
  const AppRule({required this.bundleId, required this.appClass});

  factory AppRule.fromJson(Map<String, Object?> json) => AppRule(
    bundleId: _string(json['bundleId']),
    appClass: AppClass.parse(json['class']),
  );

  /// A bundle identifier, or a prefix ending in `*`.
  final String bundleId;
  final AppClass appClass;
}

/// What a wheel mouse does (`MouseProfile`).
class MouseProfile {
  const MouseProfile({
    required this.reverseWheel,
    required this.linearWheel,
    required this.linesPerNotch,
    required this.pointer,
    required this.pointerSpeed,
    required this.sideButtons,
    required this.ctrlWheelZoom,
  });

  /// The fewest and most lines one wheel notch may scroll.
  static const int minLines = 1, maxLines = 12;

  /// Windows' pointer speed scale (Control Panel `MouseSensitivity`).
  static const int minSpeed = 1, maxSpeed = 20;

  /// Windows' defaults: the sixth of eleven speed notches, three lines.
  static const int windowsSpeed = 10, windowsLines = 3;

  static const windows = MouseProfile(
    reverseWheel: true,
    linearWheel: true,
    linesPerNotch: windowsLines,
    pointer: PointerMode.windows,
    pointerSpeed: windowsSpeed,
    sideButtons: true,
    ctrlWheelZoom: true,
  );

  static const mac = MouseProfile(
    reverseWheel: false,
    linearWheel: false,
    linesPerNotch: windowsLines,
    pointer: PointerMode.system,
    pointerSpeed: windowsSpeed,
    sideButtons: false,
    ctrlWheelZoom: false,
  );

  /// Missing fields take the Windows preset's value, as Rust's `Default`.
  factory MouseProfile.fromJson(Map<String, Object?> json) => MouseProfile(
    reverseWheel: _bool(json['reverseWheel'], windows.reverseWheel),
    linearWheel: _bool(json['linearWheel'], windows.linearWheel),
    linesPerNotch: _int(
      json['linesPerNotch'],
      windowsLines,
    ).clamp(minLines, maxLines),
    pointer: json.containsKey('pointer')
        ? PointerMode.parse(json['pointer'])
        : windows.pointer,
    pointerSpeed: _int(
      json['pointerSpeed'],
      windowsSpeed,
    ).clamp(minSpeed, maxSpeed),
    sideButtons: _bool(json['sideButtons'], windows.sideButtons),
    ctrlWheelZoom: _bool(json['ctrlWheelZoom'], windows.ctrlWheelZoom),
  );

  final bool reverseWheel;
  final bool linearWheel;
  final int linesPerNotch;
  final PointerMode pointer;
  final int pointerSpeed;
  final bool sideButtons;
  final bool ctrlWheelZoom;

  /// Written into [into], so a key a newer library added stays.
  void writeInto(Map<String, Object?> into) {
    into['reverseWheel'] = reverseWheel;
    into['linearWheel'] = linearWheel;
    into['linesPerNotch'] = linesPerNotch;
    into['pointer'] = pointer.name;
    into['pointerSpeed'] = pointerSpeed;
    into['sideButtons'] = sideButtons;
    into['ctrlWheelZoom'] = ctrlWheelZoom;
  }

  MouseProfile copyWith({
    bool? reverseWheel,
    bool? linearWheel,
    int? linesPerNotch,
    PointerMode? pointer,
    int? pointerSpeed,
    bool? sideButtons,
    bool? ctrlWheelZoom,
  }) => MouseProfile(
    reverseWheel: reverseWheel ?? this.reverseWheel,
    linearWheel: linearWheel ?? this.linearWheel,
    linesPerNotch: linesPerNotch ?? this.linesPerNotch,
    pointer: pointer ?? this.pointer,
    pointerSpeed: pointerSpeed ?? this.pointerSpeed,
    sideButtons: sideButtons ?? this.sideButtons,
    ctrlWheelZoom: ctrlWheelZoom ?? this.ctrlWheelZoom,
  );

  /// Which preset these values are, as `MouseProfile::style` decides: the
  /// numbers count only when the switch they tune is on.
  ProfileStyle get style {
    final isWindows =
        reverseWheel &&
        linearWheel &&
        linesPerNotch == windowsLines &&
        pointer == PointerMode.windows &&
        pointerSpeed == windowsSpeed &&
        sideButtons &&
        ctrlWheelZoom;
    final isMac =
        !reverseWheel &&
        !linearWheel &&
        pointer == PointerMode.system &&
        !sideButtons &&
        !ctrlWheelZoom;
    if (isWindows) return ProfileStyle.windows;
    if (isMac) return ProfileStyle.mac;
    return ProfileStyle.custom;
  }

  @override
  bool operator ==(Object other) =>
      other is MouseProfile &&
      other.reverseWheel == reverseWheel &&
      other.linearWheel == linearWheel &&
      other.linesPerNotch == linesPerNotch &&
      other.pointer == pointer &&
      other.pointerSpeed == pointerSpeed &&
      other.sideButtons == sideButtons &&
      other.ctrlWheelZoom == ctrlWheelZoom;

  @override
  int get hashCode => Object.hash(
    reverseWheel,
    linearWheel,
    linesPerNotch,
    pointer,
    pointerSpeed,
    sideButtons,
    ctrlWheelZoom,
  );
}

/// One mouse's own profile (`MouseDeviceSettings`).
class MouseDeviceSettings {
  const MouseDeviceSettings({
    required this.id,
    required this.name,
    required this.profile,
  });

  factory MouseDeviceSettings.fromJson(Map<String, Object?> json) =>
      MouseDeviceSettings(
        id: _string(json['id']),
        name: _string(json['name']),
        profile: MouseProfile.fromJson(_map(json['profile'])),
      );

  final String id;
  final String name;
  final MouseProfile profile;
}

/// Everything Isthmus does to the keyboard and mouse (`Settings`), as the
/// raw JSON it was read from.
///
/// Readers decode on demand with Rust's defaults; every `with…` edit returns
/// a new [Settings] over a deep copy of [json] with only its own fields
/// changed, so whatever else the map holds (a newer library's keys) is sent
/// back untouched.
class Settings {
  Settings(Map<String, Object?> json)
    : json = Map.unmodifiable(_deepCopy(json) as Map<String, Object?>);

  /// What an empty settings file means: Windows habits, switched off.
  static final Settings initial = Settings(const {});

  /// The raw JSON, as read and as it will be written.
  final Map<String, Object?> json;

  Map<String, Object?> get _keyboard => _map(json['keyboard']);
  Map<String, Object?> get _mouse => _map(json['mouse']);

  /// The one switch: off puts back every system change.
  bool get enabled => _bool(json['enabled'], false);

  // --- Keyboard.

  /// The style of a keyboard with no entry of its own. Missing reads as
  /// Windows, the Rust default.
  KeyboardStyle get keyboardDefaultStyle =>
      _keyboard.containsKey('defaultStyle')
      ? KeyboardStyle.parse(_keyboard['defaultStyle'])
      : KeyboardStyle.windows;

  List<KeyboardDeviceSettings> get keyboardDevices => [
    for (final entry in _list(_keyboard['devices']))
      KeyboardDeviceSettings.fromJson(_map(entry)),
  ];

  /// The entry for keyboard [id], if it has one.
  KeyboardDeviceSettings? keyboardDevice(String id) {
    for (final device in keyboardDevices) {
      if (device.id == id) return device;
    }
    return null;
  }

  /// The style keyboard [id] gets: its own entry, else the default.
  KeyboardStyle keyboardStyleOf(String id) =>
      keyboardDevice(id)?.style ?? keyboardDefaultStyle;

  /// What keyboard [id]'s F-row sends: its own entry, else what the preset
  /// of the default style sets.
  FunctionKeys functionKeysOf(String id) =>
      keyboardDevice(id)?.functionKeys ??
      (keyboardDefaultStyle == KeyboardStyle.windows
          ? FunctionKeys.standard
          : FunctionKeys.system);

  /// The layout keyboard [id] was set to, [LayoutChoice.auto] by default.
  LayoutChoice layoutOf(String id) =>
      keyboardDevice(id)?.layout ?? LayoutChoice.auto;

  /// Whether the event tap translates [group]; missing reads as on.
  bool groupEnabled(ShortcutGroup group) =>
      _bool(_map(_keyboard['groups'])[group.key], true);

  List<AppRule> get appRules => [
    for (final entry in _list(_keyboard['appRules']))
      AppRule.fromJson(_map(entry)),
  ];

  /// Windows when every keyboard and every group is; Mac when every keyboard
  /// is; Custom otherwise, as `KeyboardSettings::style` decides.
  ProfileStyle get keyboardStyle {
    final styles = [
      keyboardDefaultStyle,
      for (final device in keyboardDevices) device.style,
    ];
    final allGroups = ShortcutGroup.values.every(groupEnabled);
    if (styles.every((s) => s == KeyboardStyle.windows) && allGroups) {
      return ProfileStyle.windows;
    }
    if (styles.every((s) => s == KeyboardStyle.mac)) return ProfileStyle.mac;
    return ProfileStyle.custom;
  }

  // --- Mouse.

  /// The profile of a wheel mouse with no entry of its own.
  MouseProfile get mouseDefaults => _mouse.containsKey('defaults')
      ? MouseProfile.fromJson(_map(_mouse['defaults']))
      : MouseProfile.windows;

  List<MouseDeviceSettings> get mouseDevices => [
    for (final entry in _list(_mouse['devices']))
      MouseDeviceSettings.fromJson(_map(entry)),
  ];

  /// The entry for mouse [id], if it has a profile of its own.
  MouseDeviceSettings? mouseDevice(String id) {
    for (final device in mouseDevices) {
      if (device.id == id) return device;
    }
    return null;
  }

  /// The profile mouse [id] gets: its own, else the defaults.
  MouseProfile mouseProfileOf(String id) =>
      mouseDevice(id)?.profile ?? mouseDefaults;

  /// The defaults' style, or Custom when a mouse of its own differs, as
  /// `MouseSettings::style` decides.
  ProfileStyle get mouseStyle {
    final first = mouseDefaults.style;
    return mouseDevices.every((d) => d.profile.style == first)
        ? first
        : ProfileStyle.custom;
  }

  // --- Edits. Each copies the map and changes only its own fields.

  Settings _edit(void Function(Map<String, Object?> root) change) {
    final root = _deepCopy(json) as Map<String, Object?>;
    change(root);
    return Settings(root);
  }

  /// The object at [key] in [parent], created when it is missing or not an
  /// object, so an edit always has somewhere to write.
  static Map<String, Object?> _child(Map<String, Object?> parent, String key) {
    final existing = parent[key];
    if (existing is Map<String, Object?>) return existing;
    final created = <String, Object?>{};
    parent[key] = created;
    return created;
  }

  static List<Object?> _childList(Map<String, Object?> parent, String key) {
    final existing = parent[key];
    if (existing is List<Object?>) return existing;
    final created = <Object?>[];
    parent[key] = created;
    return created;
  }

  Settings withEnabled(bool on) => _edit((root) => root['enabled'] = on);

  Settings withGroup(ShortcutGroup group, bool on) => _edit(
    (root) => _child(_child(root, 'keyboard'), 'groups')[group.key] = on,
  );

  /// Keyboard [id]'s entry with the given fields changed, created from what
  /// the keyboard gets now when it has none, so making an entry for one
  /// choice never changes another.
  Settings withKeyboardDevice(
    String id, {
    required String name,
    KeyboardStyle? style,
    LayoutChoice? layout,
    FunctionKeys? functionKeys,
  }) {
    final currentStyle = keyboardStyleOf(id);
    final currentKeys = functionKeysOf(id);
    final currentLayout = layoutOf(id);
    return _edit((root) {
      final devices = _childList(_child(root, 'keyboard'), 'devices');
      Map<String, Object?>? entry;
      for (final item in devices) {
        if (item is Map<String, Object?> && item['id'] == id) entry = item;
      }
      if (entry == null) {
        entry = <String, Object?>{
          'id': id,
          'name': name,
          'style': currentStyle.name,
          'layout': currentLayout.name,
          'functionKeys': currentKeys.name,
        };
        devices.add(entry);
      }
      if (name.isNotEmpty) entry['name'] = name;
      if (style != null) entry['style'] = style.name;
      if (layout != null) entry['layout'] = layout.name;
      if (functionKeys != null) entry['functionKeys'] = functionKeys.name;
    });
  }

  /// [rule] added, or replacing the rule for the same bundle id.
  Settings withAppRule(AppRule rule) => _edit((root) {
    final rules = _childList(_child(root, 'keyboard'), 'appRules');
    for (final item in rules) {
      if (item is Map<String, Object?> && item['bundleId'] == rule.bundleId) {
        item['class'] = rule.appClass.name;
        return;
      }
    }
    rules.add(<String, Object?>{
      'bundleId': rule.bundleId,
      'class': rule.appClass.name,
    });
  });

  Settings withoutAppRule(String bundleId) => _edit((root) {
    _childList(
      _child(root, 'keyboard'),
      'appRules',
    ).removeWhere((item) => item is Map && item['bundleId'] == bundleId);
  });

  Settings withMouseDefaults(MouseProfile profile) => _edit(
    (root) => profile.writeInto(_child(_child(root, 'mouse'), 'defaults')),
  );

  /// Mouse [id] with a profile of its own, or following the defaults again
  /// when [profile] is null.
  Settings withMouseDevice(
    String id, {
    required String name,
    MouseProfile? profile,
  }) => _edit((root) {
    final devices = _childList(_child(root, 'mouse'), 'devices');
    if (profile == null) {
      devices.removeWhere((item) => item is Map && item['id'] == id);
      return;
    }
    for (final item in devices) {
      if (item is Map<String, Object?> && item['id'] == id) {
        if (name.isNotEmpty) item['name'] = name;
        profile.writeInto(_child(item, 'profile'));
        return;
      }
    }
    final created = <String, Object?>{'id': id, 'name': name};
    profile.writeInto(_child(created, 'profile'));
    devices.add(created);
  });

  /// [preset] applied to [target], as `Settings::apply_preset` does, for the
  /// moment before Rust answers with the saved result.
  Settings withPreset(Preset preset, PresetTarget target) => _edit((root) {
    final keyboard = target != PresetTarget.mouse;
    final mouse = target != PresetTarget.keyboard;
    if (keyboard) {
      final section = _child(root, 'keyboard');
      final style = preset == Preset.windows ? 'windows' : 'mac';
      section['enabled'] = true;
      section['defaultStyle'] = style;
      for (final item in _childList(section, 'devices')) {
        if (item is Map<String, Object?>) {
          item['style'] = style;
          item['functionKeys'] = preset == Preset.windows
              ? 'standard'
              : 'system';
        }
      }
      if (preset == Preset.windows) {
        final groups = _child(section, 'groups');
        for (final group in ShortcutGroup.values) {
          groups[group.key] = true;
        }
      }
    }
    if (mouse) {
      final section = _child(root, 'mouse');
      section['enabled'] = true;
      (preset == Preset.windows ? MouseProfile.windows : MouseProfile.mac)
          .writeInto(_child(section, 'defaults'));
      section['devices'] = <Object?>[];
    }
    if (preset == Preset.windows) root['enabled'] = true;
  });

  @override
  bool operator ==(Object other) =>
      other is Settings && jsonEquals(other.json, json);

  @override
  int get hashCode => Object.hashAllUnordered(json.keys);
}

// ---------------------------------------------------------------------------
// Status.

/// A macOS privacy permission's state (`PermissionState`).
enum PermissionState {
  granted,
  denied,
  unknown;

  static PermissionState parse(Object? raw) => switch (raw) {
    'granted' => PermissionState.granted,
    'denied' => PermissionState.denied,
    _ => PermissionState.unknown,
  };
}

/// One event tap's state (`TapState`). Unknown reads as [failed].
enum TapState {
  running,
  off,
  needsPermission,
  failed;

  static TapState parse(Object? raw) => switch (raw) {
    'running' => TapState.running,
    'off' => TapState.off,
    'needsPermission' => TapState.needsPermission,
    _ => TapState.failed,
  };
}

/// How much of the HID event system Isthmus can reach (`HidBackend`).
enum HidBackend {
  full,
  limited,
  unavailable;

  static HidBackend parse(Object? raw) => switch (raw) {
    'full' => HidBackend.full,
    'limited' => HidBackend.limited,
    _ => HidBackend.unavailable,
  };
}

/// The mapping Windows-style keyboards have now (`HidProfile`).
enum HidProfile {
  windows,
  nativeCtrl,
  positions;

  static HidProfile parse(Object? raw) => switch (raw) {
    'windows' => HidProfile.windows,
    'nativeCtrl' => HidProfile.nativeCtrl,
    _ => HidProfile.positions,
  };
}

/// Per-app fixes inside the standard class (`AppTraits`).
class AppTraits {
  const AppTraits({
    this.browser = false,
    this.nativeSideButtons = false,
    this.vscode = false,
    this.office = false,
    this.finder = false,
  });

  factory AppTraits.fromJson(Map<String, Object?> json) => AppTraits(
    browser: _bool(json['browser'], false),
    nativeSideButtons: _bool(json['nativeSideButtons'], false),
    vscode: _bool(json['vscode'], false),
    office: _bool(json['office'], false),
    finder: _bool(json['finder'], false),
  );

  final bool browser;
  final bool nativeSideButtons;
  final bool vscode;
  final bool office;
  final bool finder;
}

/// An app as the UI shows it (`FrontApp`): the one in front, or one of the
/// running apps a rule can be made for.
class FrontApp {
  const FrontApp({
    required this.bundleId,
    required this.name,
    this.pid = 0,
    this.appClass = AppClass.standard,
    this.traits = const AppTraits(),
  });

  factory FrontApp.fromJson(Map<String, Object?> json) {
    final profile = _map(json['profile']);
    return FrontApp(
      bundleId: _string(json['bundleId']),
      name: _string(json['name']),
      pid: _int(json['pid'], 0),
      appClass: AppClass.parse(profile['class']),
      traits: AppTraits.fromJson(_map(profile['traits'])),
    );
  }

  final String bundleId;
  final String name;
  final int pid;
  final AppClass appClass;
  final AppTraits traits;

  /// The name to show: the app's own, else its bundle id.
  String get label => name.isNotEmpty ? name : bundleId;
}

/// What kind of thing an [Issue] is.
enum IssueKind {
  karabinerActive,
  systemModifierKeys,
  foreignPointerTool,
  relaunchNeeded,
  restorePending,

  /// `settings.json` could not be read, so Isthmus runs with the defaults,
  /// switched off; the next save writes a fresh file and keeps the
  /// unreadable one aside.
  settingsUnreadable,
  unknown;

  static IssueKind parse(Object? raw) => switch (raw) {
    'karabinerActive' => IssueKind.karabinerActive,
    'systemModifierKeys' => IssueKind.systemModifierKeys,
    'foreignPointerTool' => IssueKind.foreignPointerTool,
    'relaunchNeeded' => IssueKind.relaunchNeeded,
    'restorePending' => IssueKind.restorePending,
    'settingsUnreadable' => IssueKind.settingsUnreadable,
    _ => IssueKind.unknown,
  };
}

/// Something the user should know about (`Issue`, tagged by `kind`).
class Issue {
  const Issue(this.kind, {this.device = '', this.name = '', this.count = 0});

  factory Issue.fromJson(Map<String, Object?> json) => Issue(
    IssueKind.parse(json['kind']),
    device: _string(json['device']),
    name: _string(json['name']),
    count: _int(json['count'], 0),
  );

  final IssueKind kind;

  /// [IssueKind.systemModifierKeys]: the keyboard System Settings changes.
  final String device;

  /// [IssueKind.foreignPointerTool]: the other app.
  final String name;

  /// [IssueKind.restorePending]: how many changes wait to be put back.
  final int count;
}

/// The state of everything, for the overview (`Status`).
class Status {
  Status(this.json);

  /// What the app shows before Rust has answered once.
  static final Status initial = Status(const {});

  factory Status.fromJson(Map<String, Object?> json) => Status(json);

  /// The raw JSON, kept so two reads compare cheaply and exactly.
  final Map<String, Object?> json;

  String get version => _string(json['version']);
  bool get enabled => _bool(json['enabled'], false);

  /// Applying the settings now.
  bool get running => _bool(json['running'], false);

  /// Another Isthmus holds the instance lock, so this one changes nothing.
  bool get anotherInstance => _bool(json['anotherInstance'], false);

  PermissionState get accessibility =>
      PermissionState.parse(_map(json['permissions'])['accessibility']);

  /// Before the first answer the permission is unknown, which must not show
  /// the permission card on a Mac that has granted it.
  bool get knowsAccessibility => _map(json['permissions']).isNotEmpty;

  TapState get keyboardTap => json.containsKey('keyboardTap')
      ? TapState.parse(json['keyboardTap'])
      : TapState.off;

  TapState get mouseTap => json.containsKey('mouseTap')
      ? TapState.parse(json['mouseTap'])
      : TapState.off;

  bool get secureInput => _bool(json['secureInput'], false);

  HidBackend get hidBackend => _map(json['hid']).containsKey('backend')
      ? HidBackend.parse(_map(json['hid'])['backend'])
      : HidBackend.full;

  int get keyboardsMapped => _int(_map(json['hid'])['keyboardsMapped'], 0);
  int get miceTuned => _int(_map(json['hid'])['miceTuned'], 0);

  FrontApp? get frontApp {
    final raw = json['frontApp'];
    return raw is Map ? FrontApp.fromJson(_map(raw)) : null;
  }

  HidProfile get hidProfile => HidProfile.parse(json['hidProfile']);

  List<Issue> get issues => [
    for (final entry in _list(json['issues'])) Issue.fromJson(_map(entry)),
  ];

  @override
  bool operator ==(Object other) =>
      other is Status && jsonEquals(other.json, json);

  @override
  int get hashCode => Object.hashAllUnordered(json.keys);
}

// ---------------------------------------------------------------------------
// Devices.

/// What a device is to Isthmus (`DeviceKind`). [unknown] is a kind this build
/// cannot name; such a device is not listed.
enum DeviceKind {
  keyboard,
  mouse,
  trackpad,
  unknown;

  static DeviceKind parse(Object? raw) => switch (raw) {
    'keyboard' => DeviceKind.keyboard,
    'mouse' => DeviceKind.mouse,
    'trackpad' => DeviceKind.trackpad,
    _ => DeviceKind.unknown,
  };
}

/// Where a keyboard's modifiers sit (`Layout`).
enum Layout {
  mac,
  pc;

  static Layout parse(Object? raw) => raw == 'mac' ? Layout.mac : Layout.pc;
}

/// How sure layout detection is (`Confidence`).
enum Confidence {
  low,
  medium,
  high;

  static Confidence parse(Object? raw) => switch (raw) {
    'high' => Confidence.high,
    'medium' => Confidence.medium,
    _ => Confidence.low,
  };
}

/// A device for the keyboard and mouse pages (`DeviceEntry`, with `Device`
/// flattened into it).
class DeviceEntry {
  const DeviceEntry({
    required this.id,
    required this.kind,
    required this.name,
    this.manufacturer = '',
    this.transport = '',
    this.builtIn = false,
    this.detectedLayout = Layout.pc,
    this.layoutConfidence = Confidence.medium,
    this.viaKvm = false,
    this.connected = true,
    this.layout = Layout.pc,
    this.applied = false,
  });

  factory DeviceEntry.fromJson(Map<String, Object?> json) => DeviceEntry(
    id: _string(json['id']),
    kind: DeviceKind.parse(json['kind']),
    name: _string(json['name']),
    manufacturer: _string(json['manufacturer']),
    transport: _string(json['transport']),
    builtIn: _bool(json['builtIn'], false),
    detectedLayout: Layout.parse(json['detectedLayout']),
    layoutConfidence: Confidence.parse(json['layoutConfidence']),
    viaKvm: _bool(json['viaKvm'], false),
    connected: _bool(json['connected'], true),
    layout: Layout.parse(json['layout']),
    applied: _bool(json['applied'], false),
  );

  /// The stable device id settings entries are keyed by.
  final String id;
  final DeviceKind kind;
  final String name;
  final String manufacturer;

  /// `USB`, `Bluetooth`, `SPI`, …
  final String transport;
  final bool builtIn;
  final Layout detectedLayout;
  final Confidence layoutConfidence;

  /// A KVM switch's emulated device: the one behind it is unknown.
  final bool viaKvm;

  /// Connected now; a remembered device with settings is listed while away.
  final bool connected;

  /// The layout in effect: the user's choice, else the detected one.
  final Layout layout;

  /// Isthmus's mapping or pointer speed is on its services now.
  final bool applied;
}

// ---------------------------------------------------------------------------
// Everything else the ops take or answer.

/// What a permission request is for (`PermissionKind`).
enum PermissionKind { accessibility }

/// System Settings panes the UI links to (`SettingsPane`).
enum SettingsPane { accessibility, loginItems, keyboard, mouse, trackpad }

/// Launch at login, as `SMAppService.mainApp` reports it (`LoginItem`).
enum LoginItem {
  enabled,
  disabled,
  requiresApproval,
  unavailable;

  static LoginItem parse(Object? raw) => switch (raw) {
    'enabled' => LoginItem.enabled,
    'disabled' => LoginItem.disabled,
    'requiresApproval' => LoginItem.requiresApproval,
    _ => LoginItem.unavailable,
  };
}

/// What a change hint says went stale (`Change`). [unknown] is a kind this
/// build cannot name, read as "something changed".
enum ChangeKind {
  status,
  devices,
  settings,
  frontApp,
  unknown;

  static ChangeKind parse(Object? raw) => switch (raw) {
    'status' => ChangeKind.status,
    'devices' => ChangeKind.devices,
    'settings' => ChangeKind.settings,
    'frontApp' => ChangeKind.frontApp,
    _ => ChangeKind.unknown,
  };
}

/// A change hint from Rust: identity, not state. The receiver re-reads
/// through the normal calls, so a dropped or coalesced hint costs nothing.
class Change {
  const Change(this.kind, {this.seq = 0});

  factory Change.fromJson(Map<String, Object?> json) =>
      Change(ChangeKind.parse(json['kind']), seq: _int(json['seq'], 0));

  final ChangeKind kind;

  /// Rust's counter, increasing with each hint.
  final int seq;
}
