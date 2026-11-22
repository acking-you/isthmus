import 'dart:async';
import 'dart:io' show File, Platform, Process, ProcessStartMode, pid;
import 'dart:ui' show AppExitResponse, AppExitType;

import 'package:cupertino_ui/cupertino_ui.dart'
    show GlobalCupertinoLocalizations;
import 'package:flutter/services.dart' show ServicesBinding;
import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/l10n/app_localizations.dart';
import 'package:isthmus_ui/src/common/app_toast.dart';
import 'package:isthmus_ui/src/common/file_drop.dart';
import 'package:isthmus_ui/src/common/permission_card.dart';
import 'package:isthmus_ui/src/common/tray/tray_service.dart';
import 'package:isthmus_ui/src/ffi/isthmus_api.dart';
import 'package:isthmus_ui/src/ffi/isthmus_service.dart';
import 'package:isthmus_ui/src/pages/devices_page.dart';
import 'package:isthmus_ui/src/pages/keyboard_page.dart';
import 'package:isthmus_ui/src/pages/mouse_page.dart';
import 'package:isthmus_ui/src/pages/overview_page.dart';
import 'package:isthmus_ui/src/pages/settings_page.dart';
import 'package:isthmus_ui/src/shell/app_prefs.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:isthmus_ui/src/shell/app_shell.dart';
import 'package:isthmus_ui/src/state/app_store.dart';
import 'package:isthmus_ui/src/theme/app_theme.dart';
import 'package:isthmus_ui/src/theme/motion.dart';
import 'package:isthmus_ui/src/updates/update_controller.dart';
import 'package:isthmus_ui/src/updates/update_schedule.dart';
import 'package:toastification/toastification.dart' show ToastificationWrapper;
import 'package:window_manager/window_manager.dart';

/// The app. Owns the store and the preferences, and on a real Mac the
/// window's life in the menu bar: the tray item, showing the window when it
/// is needed, hiding it on close, and quitting through AppKit.
class IsthmusApp extends StatefulWidget {
  const IsthmusApp({
    super.key,
    required this.api,
    this.changes,
    this.prefs,
    this.initialPage,
    this.pollInterval = const Duration(seconds: 2),
    this.windowReady,
    this.updates,
  });

  final IsthmusApiClient api;

  /// Defaults to Rust's change hints; tests pass their own.
  final Stream<Change>? changes;

  /// Defaults to preferences kept in `shared_preferences`.
  final AppPrefs? prefs;

  /// Defaults to [AppPage.home].
  final AppPage? initialPage;

  /// How often the status is read while the window shows.
  final Duration? pollInterval;

  /// Completes once the native window is configured (`main.dart`). Null
  /// means there is no native window to run: a test. With it, the app runs
  /// the menu bar life; without it, the "window" is always visible.
  final Future<void>? windowReady;

  /// Defaults to the update flow against GitHub; tests pass their own.
  final UpdateController? updates;

  @override
  State<IsthmusApp> createState() => _IsthmusAppState();
}

class _IsthmusAppState extends State<IsthmusApp> with WindowListener {
  late final AppPrefs _prefs = widget.prefs ?? AppPrefs();
  late final AppStore _store = AppStore(
    api: widget.api,
    changes: widget.changes,
    pollInterval: widget.pollInterval,
    onFailure: _onFailure,
    onPermissionRequested: _desktop ? _showPermissionCard : null,
  );
  late final UpdateController _updates =
      widget.updates ?? UpdateController(quit: _quit, api: widget.api);
  UpdateSchedule? _updateSchedule;
  late AppPage _page = widget.initialPage ?? AppPage.home;
  AppLifecycleListener? _lifecycle;
  Future<void>? _quitting;
  bool _relaunching = false;

  bool get _desktop => widget.windowReady != null;

  @override
  void initState() {
    super.initState();
    _prefs.addListener(_onPrefs);
    _store.addListener(_refreshTray);
    _store.link.onReceived = _onReceived;
    _store.link.onPairingRequest = (_) => _onPairingRequest();
    if (_desktop) {
      _store.addListener(_relaunchIfMacOSAsks);
      _store.addListener(_hidePermissionCardOnceGranted);
      _startDesktop();
    } else {
      _store.setVisible(true);
    }
    unawaited(_boot());
  }

  Future<void> _boot() async {
    await (_prefs.load(), _store.start()).wait;
    final windowReady = widget.windowReady;
    if (windowReady == null || !mounted) return;
    await windowReady;
    // The window waits in the menu bar unless something needs the user: a
    // first run, a missing permission, or a service that did not start.
    if (!_prefs.onboarded ||
        _store.needsAccessibility ||
        _store.startError != null ||
        // Just relaunched to apply a grant: show that it worked.
        _prefs.relaunchedWithin(const Duration(minutes: 1))) {
      await _showWindow();
    }
    if (updatesSupported && mounted) {
      unawaited(_reportUpdate());
      _updateSchedule = UpdateSchedule(
        updates: _updates,
        prefs: _prefs,
        announce: (version) => showToast(
          null,
          lookupAppLocalizations(_prefs.activeLocale)
              .updateAnnounce('$version'),
          kind: ToastKind.info,
        ),
      )..start();
    }
  }

  /// Say what became of the update handed off before the last quit. The
  /// update reopens the app from the background, where it would otherwise
  /// wait unseen in the menu bar or the tray, so the window comes forward:
  /// otherwise the app seems to have vanished, and the update to have
  /// failed.
  Future<void> _reportUpdate() async {
    final outcome = await _updates.takeOutcome();
    if (outcome == null || !mounted) return;
    await _showWindow();
    final l10n = lookupAppLocalizations(_prefs.activeLocale);
    final version = '${outcome.to}';
    if (outcome.installed) {
      showToast(null, l10n.updateDone(version), kind: ToastKind.success);
    } else {
      showToast(
        null,
        l10n.updateNotDone(version),
        kind: ToastKind.error,
        description: l10n.updateNotDoneHint,
      );
    }
  }

  /// Puts the drag-to-authorize card under the Accessibility pane the
  /// request just opened.
  void _showPermissionCard() {
    final l10n = lookupAppLocalizations(_prefs.activeLocale);
    final bundle = File(Platform.resolvedExecutable).parent.parent.parent;
    final name = bundle.uri.pathSegments
        .where((segment) => segment.isNotEmpty)
        .last
        .replaceAll(RegExp(r'\.app$'), '');
    unawaited(
      PermissionCard.show(
        title: l10n.permissionCardTitle(name),
        subtitle: l10n.permissionCardSubtitle,
        appName: name,
      ),
    );
  }

  /// The card's job ends with the grant: the moment the service reads it,
  /// or the tap it allows comes up.
  void _hidePermissionCardOnceGranted() {
    if (PermissionCard.shown && !_store.needsAccessibility) {
      unawaited(PermissionCard.hide());
    }
  }

  /// macOS applies a new Accessibility grant to a running app only
  /// sometimes. Rust tries the tap again the moment the grant appears; when
  /// it is still refused with the grant in place (`relaunchNeeded`), the app
  /// restarts itself once instead of asking the user to. A second request
  /// within [AppPrefs.relaunchCooldown] means a restart does not help, and
  /// the overview's explanation is left to the user.
  void _relaunchIfMacOSAsks() {
    if (_relaunching || _quitting != null) return;
    final asked = _store.status.issues.any(
      (issue) => issue.kind == IssueKind.relaunchNeeded,
    );
    if (!asked || _prefs.relaunchedWithin(AppPrefs.relaunchCooldown)) return;
    _relaunching = true;
    unawaited(_relaunch());
  }

  /// Starts a helper that waits for this process to end and opens the app
  /// again, then quits through AppKit so Rust puts the system back first.
  Future<void> _relaunch() async {
    final bundle = File(Platform.resolvedExecutable).parent.parent.parent.path;
    if (!bundle.endsWith('.app')) {
      _relaunching = false;
      return;
    }
    await _prefs.markAccessibilityRelaunch();
    showToast(
      null,
      lookupAppLocalizations(_prefs.activeLocale).relaunchingForAccessibility,
      kind: ToastKind.success,
    );
    try {
      await Process.start('/bin/sh', [
        '-c',
        r'while /bin/kill -0 "$1" 2>/dev/null; do /bin/sleep 0.1; done; '
            r'/usr/bin/open "$2"',
        'isthmus-relaunch',
        '$pid',
        bundle,
      ], mode: ProcessStartMode.detached);
    } catch (error) {
      debugPrint('Could not start the relaunch helper: $error');
      _relaunching = false;
      return;
    }
    await Future<void>.delayed(const Duration(milliseconds: 1200));
    await _quit();
  }

  void _onPrefs() {
    if (!mounted) return;
    setState(() {});
    _refreshTray();
  }

  void _go(AppPage page) {
    if (page != _page) setState(() => _page = page);
  }

  /// Every failed call becomes one error toast, in the user's language: what
  /// did not happen, then Rust's sentence for why.
  void _onFailure(StoreAction action, IsthmusCallException error) {
    final l10n = lookupAppLocalizations(_prefs.activeLocale);
    final title = switch (action) {
      StoreAction.saveSettings => l10n.failSaveSettings,
      StoreAction.applyPreset => l10n.failApplyPreset,
      StoreAction.restore => l10n.failRestore,
      StoreAction.requestPermission => l10n.failPermission,
      StoreAction.openSettingsPane => l10n.failOpenPane,
      StoreAction.loginItem => l10n.failLoginItem,
      StoreAction.awake => l10n.failAwake,
      StoreAction.runningApps => l10n.failRunningApps,
      StoreAction.linkPair => l10n.failLinkPair,
      StoreAction.linkUnpair => l10n.failLinkUnpair,
      StoreAction.linkSend => l10n.failLinkSend,
      StoreAction.linkTransfer => l10n.failLinkTransfer,
      StoreAction.linkRename => l10n.failLinkRename,
      StoreAction.linkReveal => l10n.failLinkReveal,
    };
    showToast(null, title, kind: ToastKind.error, description: error.message);
  }

  /// Files arrived: say so, wherever the window is.
  void _onReceived(TransferEntry transfer) {
    final l10n = lookupAppLocalizations(_prefs.activeLocale);
    showToast(
      null,
      l10n.toastReceived(transfer.name, transfer.peerName),
      kind: ToastKind.success,
    );
  }

  /// Another device asked to pair: its code is on the Devices page, so the
  /// window comes up there, even from the menu bar.
  void _onPairingRequest() {
    _go(AppPage.devices);
    if (_desktop) unawaited(_showWindow());
  }

  /// Files dropped anywhere on the window wait on the Devices page for a
  /// device to send them to.
  void _onDrop(List<String> paths) {
    _store.link.stage(paths);
    _go(AppPage.devices);
  }

  // --- The menu bar life.

  void _startDesktop() {
    _lifecycle = AppLifecycleListener(
      // Command-Q and the Dock's Quit ask here. Rust puts the system back in
      // the Runner's applicationWillTerminate, which runs once this answers.
      onExitRequested: () async {
        _prepareToQuit();
        return AppExitResponse.exit;
      },
    );
    windowManager.addListener(this);
    unawaited(_startTray());
  }

  Future<void> _startTray() async {
    try {
      await windowManager.setPreventClose(true);
      await TrayService.instance.initialize(
        view: _trayView(),
        onOpen: () => unawaited(_showWindow()),
        onToggle: (on) => unawaited(_store.setEnabled(on)),
        onAwake: (on) => unawaited(_store.setAwake(on)),
        onQuit: () => unawaited(_quit()),
        onKeyboardMode: (id, windows) {
          final device = _store.devices.where((d) => d.id == id).firstOrNull;
          if (device == null) return;
          unawaited(
            _store.setKeyboardLayout(
              device,
              windows ? LayoutChoice.pc : LayoutChoice.mac,
            ),
          );
        },
      );
    } catch (error) {
      debugPrint('The menu bar item could not start: $error');
    }
  }

  void _refreshTray() {
    if (_desktop) unawaited(TrayService.instance.update(_trayView()));
  }

  /// The tray is not a widget, so its words come from the active locale.
  /// A PC has no Windows habits to switch, so its menu is open and quit.
  TrayView _trayView() {
    final l10n = lookupAppLocalizations(_prefs.activeLocale);
    final awakeLabel = _store.awake.supported ? l10n.trayKeepAwake : '';
    if (!hasInputPages) {
      return (
        status: l10n.appTitle,
        toggleLabel: '',
        enabled: false,
        awakeLabel: awakeLabel,
        awake: _store.awake.enabled,
        openLabel: l10n.trayOpen,
        quitLabel: l10n.trayQuit,
        keyboards: const [],
      );
    }
    final enabled = _store.settings.enabled;
    final status = _store.anotherInstance
        ? l10n.statusAnotherInstance
        : !enabled
        ? l10n.statusOff
        : _store.needsAccessibility
        ? l10n.statusNeedsPermission
        : l10n.statusOn;
    return (
      status: status,
      toggleLabel: l10n.trayToggle,
      enabled: enabled,
      awakeLabel: awakeLabel,
      awake: _store.awake.enabled,
      openLabel: l10n.trayOpen,
      quitLabel: l10n.trayQuit,
      keyboards: [
        for (final device in _store.devicesOf(DeviceKind.keyboard))
          if (device.hasModeSwitch && device.connected)
            (
              id: device.id,
              label: l10n.trayKeyboardWindowsMode(device.name),
              windows: device.layout == Layout.pc,
            ),
      ],
    );
  }

  /// Show the window, with a Dock icon while it shows.
  Future<void> _showWindow() async {
    try {
      await windowManager.setSkipTaskbar(false);
      if (await windowManager.isMinimized()) await windowManager.restore();
      await windowManager.show();
      await windowManager.focus();
    } catch (error) {
      debugPrint('Could not show the window: $error');
    }
    _store.setVisible(true);
    unawaited(_prefs.markOnboarded());
  }

  /// Hide the window and its Dock icon: Isthmus lives in the menu bar.
  Future<void> _hideWindow() async {
    _store.setVisible(false);
    try {
      await windowManager.hide();
      await windowManager.setSkipTaskbar(true);
    } catch (error) {
      debugPrint('Could not hide the window: $error');
    }
  }

  @override
  void onWindowClose() => unawaited(_hideWindow());

  // The Runner also brings the window back by itself (reopening the app, a
  // second copy handing over); focus is how Dart hears of it.
  @override
  void onWindowFocus() => _store.setVisible(true);

  /// Let go of the tray and the library's callback. Never throws: nothing
  /// may stand between a quit and AppKit's terminate.
  void _prepareToQuit() {
    _updateSchedule?.dispose();
    TrayService.instance.dispose();
    try {
      IsthmusService().dispose();
    } catch (error) {
      debugPrint('Releasing the Isthmus library failed: $error');
    }
  }

  /// Quit through AppKit, never `dart:io` `exit()`: only AppKit's terminate
  /// runs `applicationWillTerminate`, where Rust puts the keyboard and mouse
  /// back.
  Future<void> _quit() => _quitting ??= () async {
    // Windows has no quit hook into Rust, so file sharing says goodbye on
    // the network here; a Mac's Runner does it in applicationWillTerminate.
    if (!Platform.isMacOS) {
      try {
        await widget.api.shutdown().timeout(const Duration(seconds: 2));
      } catch (error) {
        debugPrint('Isthmus did not shut down cleanly: $error');
      }
    }
    _prepareToQuit();
    await ServicesBinding.instance.exitApplication(AppExitType.required);
  }();

  @override
  void dispose() {
    _prefs.removeListener(_onPrefs);
    _store.removeListener(_refreshTray);
    _store.removeListener(_relaunchIfMacOSAsks);
    _store.removeListener(_hidePermissionCardOnceGranted);
    unawaited(PermissionCard.hide());
    if (_desktop) {
      windowManager.removeListener(this);
      TrayService.instance.dispose();
    }
    _lifecycle?.dispose();
    _updateSchedule?.dispose();
    if (widget.updates == null) _updates.dispose();
    _store.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      onGenerateTitle: (context) => AppLocalizations.of(context)!.appTitle,
      debugShowCheckedModeBanner: false,
      locale: _prefs.locale,
      supportedLocales: AppLocalizations.supportedLocales,
      // Since Flutter 3.47 Material and Cupertino carry their own
      // localizations types, and the generated list still names
      // flutter_localizations' older ones, so Chinese would fall back to
      // English inside Material's own widgets without these two.
      localizationsDelegates: const [
        ...AppLocalizations.localizationsDelegates,
        GlobalMaterialLocalizations.delegate,
        GlobalCupertinoLocalizations.delegate,
      ],
      theme: buildAppTheme(Brightness.light),
      darkTheme: buildAppTheme(Brightness.dark),
      // Every colour is lerped between the themes, so following the system
      // into dark mode does not repaint the window in one frame.
      themeAnimationDuration: Motion.slow,
      themeAnimationCurve: Motion.move,
      // Inside MaterialApp, so a toast raised without a context still wears
      // the theme and speaks the language.
      builder: (context, child) =>
          ToastificationWrapper(child: child ?? const SizedBox.shrink()),
      home: AppScope(
        store: _store,
        prefs: _prefs,
        updates: _updates,
        page: _page,
        go: _go,
        child: FileDropZone(
          enabled: _desktop,
          onDrop: _onDrop,
          child: AppShell(
            page: _page,
            onSelect: _go,
            child: switch (_page) {
              AppPage.overview => const OverviewPage(),
              AppPage.keyboard => const KeyboardPage(),
              AppPage.mouse => const MousePage(),
              AppPage.devices => const DevicesPage(),
              AppPage.settings => const SettingsPage(),
            },
          ),
        ),
      ),
    );
  }
}
