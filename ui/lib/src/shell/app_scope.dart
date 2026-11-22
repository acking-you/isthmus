import 'package:flutter/foundation.dart';
import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/src/common/l10n_extension.dart';
import 'package:isthmus_ui/src/shell/app_prefs.dart';
import 'package:isthmus_ui/src/state/app_store.dart';
import 'package:isthmus_ui/src/theme/app_icons.dart';
import 'package:isthmus_ui/src/updates/update_controller.dart';

/// Whether this system has the keyboard and mouse pages: a Mac does, a PC
/// does not. Keyed on [defaultTargetPlatform], like the window chrome, so
/// `flutter test` (forced to Android) sees every page on any host.
bool get hasInputPages =>
    defaultTargetPlatform != TargetPlatform.windows &&
    defaultTargetPlatform != TargetPlatform.linux;

/// The places the app has. Everything else opens over one of them.
enum AppPage {
  overview,
  keyboard,
  mouse,
  devices,
  settings;

  /// The pages this system shows: the keyboard and mouse are a Mac's, so a
  /// PC has file sharing and the settings alone.
  static List<AppPage> get available =>
      hasInputPages ? values : const [AppPage.devices, AppPage.settings];

  /// Where the window opens.
  static AppPage get home => hasInputPages ? AppPage.overview : AppPage.devices;

  IconData get icon => switch (this) {
    AppPage.overview => AppIcons.overview,
    AppPage.keyboard => AppIcons.keyboard,
    AppPage.mouse => AppIcons.mouse,
    AppPage.devices => AppIcons.devices,
    AppPage.settings => AppIcons.settings,
  };

  /// The rail's label and the page's title.
  String label(BuildContext context) {
    final l10n = context.l10n;
    return switch (this) {
      AppPage.overview => l10n.navOverview,
      AppPage.keyboard => l10n.navKeyboard,
      AppPage.mouse => l10n.navMouse,
      AppPage.devices => l10n.navDevices,
      AppPage.settings => l10n.navSettings,
    };
  }

  static AppPage? parse(String? name) {
    for (final page in values) {
      if (page.name == name) return page;
    }
    return null;
  }
}

/// What every page reaches for: the data, the preferences, and the way to
/// another page.
class AppScope extends InheritedWidget {
  const AppScope({
    super.key,
    required this.store,
    required this.prefs,
    required this.updates,
    required this.page,
    required this.go,
    required super.child,
  });

  final AppStore store;
  final AppPrefs prefs;

  /// Checking for, and installing, a newer release.
  final UpdateController updates;

  /// The page on screen.
  final AppPage page;

  /// Show another page.
  final ValueChanged<AppPage> go;

  static AppScope of(BuildContext context) {
    final scope = context.dependOnInheritedWidgetOfExactType<AppScope>();
    assert(scope != null, 'No AppScope above this context.');
    return scope!;
  }

  @override
  bool updateShouldNotify(AppScope old) =>
      old.store != store ||
      old.prefs != prefs ||
      old.updates != updates ||
      old.page != page;
}
