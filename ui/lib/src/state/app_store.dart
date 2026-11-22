import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:isthmus_ui/src/ffi/isthmus_api.dart';
import 'package:isthmus_ui/src/ffi/isthmus_service.dart';
import 'package:isthmus_ui/src/state/link_store.dart';

/// What the store was doing when a call failed, so the app can say so in the
/// user's language without the store knowing any words.
enum StoreAction {
  saveSettings,
  applyPreset,
  restore,
  requestPermission,
  openSettingsPane,
  loginItem,
  awake,
  runningApps,
  linkPair,
  linkUnpair,
  linkSend,
  linkTransfer,
  linkRename,
  linkReveal,
  linkClipboard,
}

/// One settings change waiting its turn: how it looks right away, and how it
/// is sent once the writes before it have landed.
class _Write {
  _Write(this.action, this.optimistic, this.send, {this.needsSettings = false});

  final StoreAction action;

  /// The change applied to whatever the settings are at the moment, for the
  /// screen while the write is on its way.
  final Settings Function(Settings) optimistic;

  /// The call that makes the change, given the settings as the write before
  /// it left them. It answers the settings as Rust saved them.
  final Future<Settings> Function(Settings confirmed) send;

  /// Whether [send] builds on the settings it is given. Such a write is never
  /// sent before Rust's settings have been read: built on
  /// [Settings.initial] it would replace the user's whole file with the
  /// defaults. A preset and restore are ops Rust applies to its own copy, so
  /// they need no read.
  final bool needsSettings;
}

/// Everything the pages and the tray show, read once and kept fresh in one
/// place, and the one way to change it.
///
/// Freshness comes from Rust's change hints (a device arriving, the front app
/// changing, the CLI editing the settings) and, while the window shows, a
/// status read every [pollInterval]: Secure Input and the front app change
/// without a hint the moment they change.
///
/// Writes are serialized and optimistic. Each one shows at once, then waits
/// for the writes before it and is sent against the settings as they left
/// them, so two quick toggles never send a stale copy that undoes the first.
/// A failed write drops only itself from the screen and reports through
/// [onFailure]; the ones after it still go. Writes also wait for [start]'s
/// first read, and an edit of the settings is never sent until they have
/// been read: the tray's switch works from launch, and a click before the
/// read must not save the defaults over the user's file.
class AppStore extends ChangeNotifier {
  AppStore({
    required this.api,
    Stream<Change>? changes,
    this.pollInterval = const Duration(seconds: 2),
    this.onFailure,
    this.onPermissionRequested,
  }) : _changes = changes ?? IsthmusService.changeStream;

  final IsthmusApiClient api;
  final Stream<Change> _changes;

  /// How often the status is read while the window shows; null never.
  final Duration? pollInterval;

  /// Told about every failed call; the app turns it into a toast.
  void Function(StoreAction action, IsthmusCallException error)? onFailure;

  /// Told once System Settings is open at the Accessibility pane; the app
  /// shows its drag-to-authorize card there.
  void Function()? onPermissionRequested;

  /// File sharing: its own snapshot and calls, fed by the same hints.
  late final LinkStore link = LinkStore(
    api: api,
    onFailure: (action, error) => onFailure?.call(action, error),
  );

  /// The last status Rust answered.
  Status status = Status.initial;

  /// The settings as Rust last saved them.
  Settings _confirmed = Settings.initial;

  /// Whether [_confirmed] holds settings Rust answered, rather than
  /// [Settings.initial] standing in until a read lands.
  bool _settingsRead = false;
  final List<_Write> _pending = [];

  /// Every keyboard, mouse and trackpad Rust knows, connected or remembered.
  List<DeviceEntry> devices = const [];

  /// Launch at login.
  LoginItem loginItem = LoginItem.unavailable;

  /// Keeping the computer awake while its screen sleeps.
  AwakeStatus awake = AwakeStatus.unknown;

  /// Whether the first read finished, so a page can tell "nothing yet" from
  /// "nothing at all".
  bool loaded = false;

  /// Why `start` failed, when it did; the overview shows it.
  IsthmusCallException? startError;

  bool _started = false;
  bool _disposed = false;
  bool _visible = false;
  Timer? _poll;
  Timer? _debounce;
  final Set<ChangeKind> _stale = {};
  StreamSubscription<Change>? _subscription;
  Future<void> _writes = Future<void>.value();
  final Completer<bool> _firstRead = Completer<bool>();

  /// The settings on screen: what Rust saved, with the writes still on their
  /// way laid over it.
  Settings get settings => _pending.fold(
    _confirmed,
    (settings, write) => write.optimistic(settings),
  );

  /// Whether a write is still on its way.
  bool get writing => _pending.isNotEmpty;

  /// Another Isthmus holds the keyboard and mouse, so this one changes
  /// nothing.
  bool get anotherInstance =>
      status.anotherInstance || startError?.code == ErrorCode.anotherInstance;

  /// Accessibility is known and not granted.
  bool get needsAccessibility =>
      status.knowsAccessibility &&
      status.accessibility != PermissionState.granted;

  /// Completes once [start]'s first read is in, with whether every read
  /// succeeded.
  Future<bool> get ready => _firstRead.future;

  List<DeviceEntry> devicesOf(DeviceKind kind) => [
    for (final device in devices)
      if (device.kind == kind) device,
  ];

  /// Start the service, read everything once, and listen. Safe to call more
  /// than once.
  Future<void> start() async {
    if (_started) return;
    _started = true;
    // Set before the first await, so a write made while the service starts
    // queues behind the first read instead of racing it.
    final booted = Completer<void>();
    _writes = _writes.then((_) => booted.future);
    try {
      _subscription = _changes.listen(_onChange);
      var ok = true;
      try {
        status = await api.start();
      } catch (error) {
        startError = _asCallException(error);
        ok = false;
      }
      final reads = await Future.wait([
        if (startError != null) _loadStatus(),
        _loadSettings(),
        _loadDevices(),
        _loadLoginItem(),
        _loadAwake(),
      ]);
      ok = ok && reads.every((read) => read);
      if (_disposed) return;
      // Not part of `ok`: file sharing that did not start says so on its
      // own page and never holds up the keyboard and mouse.
      unawaited(link.refresh());
      loaded = true;
      _notify();
      if (!_firstRead.isCompleted) _firstRead.complete(ok);
      _syncPolling();
    } finally {
      booted.complete();
    }
  }

  /// The window shows or hides: poll the status only while someone can see
  /// it, and read it at once when it comes back.
  void setVisible(bool visible) {
    if (_visible == visible) return;
    _visible = visible;
    _syncPolling();
    if (visible && _started) {
      unawaited(refreshStatus());
      unawaited(link.refresh());
    }
  }

  void _syncPolling() {
    _poll?.cancel();
    _poll = null;
    final interval = pollInterval;
    if (!_visible || !_started || _disposed || interval == null) return;
    _poll = Timer.periodic(interval, (_) => unawaited(refreshStatus()));
  }

  /// Read the status again; listeners hear only of a real change.
  Future<void> refreshStatus() => _loadStatus();

  // --- Change hints.

  /// Gathers a burst of hints into one read per kind: a replug sends several
  /// within a few milliseconds, and reading each would only flicker.
  void _onChange(Change change) {
    if (_disposed) return;
    _stale.add(change.kind);
    _debounce?.cancel();
    _debounce = Timer(const Duration(milliseconds: 120), _reloadStale);
  }

  void _reloadStale() {
    if (_disposed) return;
    final kinds = Set.of(_stale);
    _stale.clear();
    final all = kinds.contains(ChangeKind.unknown);
    if (all ||
        kinds.contains(ChangeKind.status) ||
        kinds.contains(ChangeKind.frontApp)) {
      unawaited(_loadStatus());
    }
    if (all || kinds.contains(ChangeKind.devices)) unawaited(_loadDevices());
    if (all || kinds.contains(ChangeKind.link)) unawaited(link.refresh());
    // After the writes in flight, so a hint caused by one of them never
    // replaces the screen with an older copy.
    if (all || kinds.contains(ChangeKind.settings)) {
      _writes = _writes.then((_) => _loadSettings());
    }
  }

  // --- Reads. Each answers whether it worked and never throws.

  Future<bool> _loadStatus() async {
    try {
      final next = await api.status();
      if (_disposed) return true;
      if (next != status) {
        status = next;
        _notify();
      }
      return true;
    } catch (_) {
      // A failed status read leaves the last one on screen; the next poll or
      // hint tries again.
      return false;
    }
  }

  Future<bool> _loadSettings() async {
    try {
      await _readSettings();
      return true;
    } catch (_) {
      return false;
    }
  }

  /// Reads the settings into [_confirmed]. Throws what the call threw, so a
  /// write that needs them can report why they could not be read. Kept even
  /// after [dispose], so a write already on its way still builds on them.
  Future<void> _readSettings() async {
    final next = await api.settings();
    _settingsRead = true;
    if (next != _confirmed) {
      _confirmed = next;
      _notify();
    }
  }

  Future<bool> _loadDevices() async {
    try {
      final next = await api.devices();
      if (_disposed) return true;
      devices = next;
      _notify();
      return true;
    } catch (_) {
      return false;
    }
  }

  Future<bool> _loadLoginItem() async {
    try {
      final next = await api.loginItem();
      if (_disposed) return true;
      loginItem = next;
      _notify();
      return true;
    } catch (_) {
      return false;
    }
  }

  Future<bool> _loadAwake() async {
    try {
      final next = await api.awakeStatus();
      if (_disposed) return true;
      awake = next;
      _notify();
      return true;
    } catch (_) {
      // Not offered, rather than a failed launch: the switch hides.
      return true;
    }
  }

  // --- Writes.

  /// Change the settings with [edit]; answers whether Rust saved it.
  Future<bool> update(Settings Function(Settings) edit) => _enqueue(
    _Write(
      StoreAction.saveSettings,
      edit,
      (confirmed) => api.saveSettings(edit(confirmed)),
      needsSettings: true,
    ),
  );

  /// The one switch.
  Future<bool> setEnabled(bool on) => update((s) => s.withEnabled(on));

  /// A preset on the keyboard, the mouse, or both.
  Future<bool> applyPreset(Preset preset, PresetTarget target) => _enqueue(
    _Write(
      StoreAction.applyPreset,
      (s) => s.withPreset(preset, target),
      (_) => api.applyPreset(preset, target),
    ),
  );

  /// Switch off and put everything back now.
  Future<bool> restore() => _enqueue(
    _Write(
      StoreAction.restore,
      (s) => s.withEnabled(false),
      (_) => api.restore(),
    ),
  );

  Future<bool> _enqueue(_Write write) {
    _pending.add(write);
    _notify();
    final done = Completer<bool>();
    _writes = _writes.then((_) async {
      var ok = false;
      try {
        // Not read yet (the first read failed): read now, and send nothing
        // if that fails too rather than an edit of the defaults.
        if (write.needsSettings && !_settingsRead) await _readSettings();
        final saved = await write.send(_confirmed);
        // Every write answers the whole file as Rust saved it.
        _confirmed = saved;
        _settingsRead = true;
        ok = true;
      } catch (error) {
        if (!_disposed) onFailure?.call(write.action, _asCallException(error));
      } finally {
        _pending.remove(write);
        _notify();
        done.complete(ok);
      }
      // The switch and the presets change what is running.
      if (ok) await _loadStatus();
    });
    return done.future;
  }

  // --- Everything else.

  /// Ask macOS for Accessibility; the answer is the new status.
  Future<void> requestPermission() async {
    try {
      final next = await api.requestPermission(PermissionKind.accessibility);
      if (_disposed) return;
      status = next;
      _notify();
      if (needsAccessibility) onPermissionRequested?.call();
    } catch (error) {
      onFailure?.call(StoreAction.requestPermission, _asCallException(error));
    }
  }

  /// Lift [deviceId]'s mapping so the app can read which key it sends
  /// beside the space bar. True once lifted.
  Future<bool> beginLayoutProbe(String deviceId) async {
    try {
      await api.beginLayoutProbe(deviceId);
      return true;
    } catch (error) {
      onFailure?.call(StoreAction.saveSettings, _asCallException(error));
      return false;
    }
  }

  /// Map the probed keyboard again; best effort (Rust ends a forgotten
  /// probe by itself).
  Future<void> endLayoutProbe() async {
    try {
      await api.endLayoutProbe();
    } catch (_) {}
  }

  /// Set a keyboard's layout, which for a keyboard with a mode switch is
  /// the mode it is in.
  Future<bool> setKeyboardLayout(DeviceEntry device, LayoutChoice layout) =>
      update(
        (s) =>
            s.withKeyboardDevice(device.id, name: device.name, layout: layout),
      );

  Future<void> openSettingsPane(SettingsPane pane) async {
    try {
      await api.openSettingsPane(pane);
    } catch (error) {
      onFailure?.call(StoreAction.openSettingsPane, _asCallException(error));
    }
  }

  Future<void> setLoginItem(bool enabled) async {
    try {
      final next = await api.setLoginItem(enabled);
      if (_disposed) return;
      loginItem = next;
      _notify();
    } catch (error) {
      onFailure?.call(StoreAction.loginItem, _asCallException(error));
      await _loadLoginItem();
    }
  }

  /// Turns keeping the computer awake on or off. The switch moves at once;
  /// a refusal says why and shows what the library holds.
  Future<void> setAwake(bool enabled) async {
    awake = AwakeStatus(
      enabled: enabled,
      active: awake.active,
      supported: awake.supported,
    );
    _notify();
    try {
      final next = await api.setAwake(enabled);
      if (_disposed) return;
      awake = next;
      _notify();
    } catch (error) {
      onFailure?.call(StoreAction.awake, _asCallException(error));
      await _loadAwake();
    }
  }

  /// The apps running now, for making a rule; empty when the call fails.
  Future<List<FrontApp>> runningApps() async {
    try {
      return await api.runningApps();
    } catch (error) {
      onFailure?.call(StoreAction.runningApps, _asCallException(error));
      return const [];
    }
  }

  /// Any failure as a call failure: a library that would not load is still
  /// something the user is told about, with code INTERNAL.
  static IsthmusCallException _asCallException(Object error) =>
      error is IsthmusCallException
      ? error
      : IsthmusCallException('$error', code: ErrorCode.internal);

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _poll?.cancel();
    _debounce?.cancel();
    unawaited(_subscription?.cancel());
    link.dispose();
    super.dispose();
  }
}
