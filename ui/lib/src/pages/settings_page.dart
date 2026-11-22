import 'dart:async';

import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/src/common/app_toast.dart';
import 'package:isthmus_ui/src/common/l10n_extension.dart';
import 'package:isthmus_ui/src/ffi/models.dart';
import 'package:isthmus_ui/src/shell/app_prefs.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:isthmus_ui/src/shell/page_frame.dart';
import 'package:isthmus_ui/src/state/app_store.dart';
import 'package:isthmus_ui/src/theme/app_icons.dart';
import 'package:isthmus_ui/src/theme/glass.dart';
import 'package:isthmus_ui/src/updates/update_controller.dart';
import 'package:isthmus_ui/src/updates/update_dialog.dart';
import 'package:isthmus_ui/src/widgets/brand_mark.dart';
import 'package:isthmus_ui/src/widgets/setting_rows.dart';
import 'package:isthmus_ui/src/widgets/surfaces.dart';
import 'package:package_info_plus/package_info_plus.dart';

/// The app itself: opening at login, keeping the computer awake, its
/// language, putting everything back and the System Settings panes it
/// depends on (on a Mac), and what it is, with its updates.
class SettingsPage extends StatelessWidget {
  const SettingsPage({super.key});

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final scope = AppScope.of(context);
    return PageFrame(
      title: l10n.navSettings,
      body: ListenableBuilder(
        listenable: Listenable.merge([scope.store, scope.prefs]),
        builder: (context, _) => PageList(
          listKey: const Key('settings-list'),
          children: [
            _GeneralCard(store: scope.store, prefs: scope.prefs),
            const SizedBox(height: 16),
            if (hasInputPages) ...[
              _RestoreCard(store: scope.store),
              const SizedBox(height: 16),
              _PanesCard(store: scope.store),
              const SizedBox(height: 16),
            ],
            _AboutCard(
              store: scope.store,
              prefs: scope.prefs,
              updates: scope.updates,
            ),
            const SizedBox(height: 16),
            GroupCard(
              key: const Key('settings-coming-next'),
              title: l10n.comingNextTitle,
              icon: AppIcons.comingNext,
              child: Text(
                l10n.comingNextBody,
                style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                  color: Theme.of(context).colorScheme.onSurfaceVariant,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// Opening at login, keeping the computer awake, and the language: the app's
/// own choices.
class _GeneralCard extends StatelessWidget {
  const _GeneralCard({required this.store, required this.prefs});

  final AppStore store;
  final AppPrefs prefs;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final login = store.loginItem;
    final note = switch (login) {
      LoginItem.requiresApproval => l10n.loginApproval,
      LoginItem.unavailable => l10n.loginUnavailable,
      LoginItem.enabled || LoginItem.disabled => null,
    };
    return GroupCard(
      title: l10n.settingsGeneral,
      icon: AppIcons.settings,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SwitchRow(
            switchKey: const Key('settings-login'),
            title: l10n.launchAtLogin,
            subtitle: note,
            value: login == LoginItem.enabled,
            onChanged: login == LoginItem.unavailable
                ? null
                : (on) => store.setLoginItem(on),
          ),
          if (store.awake.supported)
            SwitchRow(
              switchKey: const Key('settings-awake'),
              title: l10n.keepAwakeTitle,
              subtitle: store.awake.error == null
                  ? l10n.keepAwakeHint
                  : l10n.keepAwakeRefused(store.awake.error!),
              value: store.awake.enabled,
              onChanged: (on) => store.setAwake(on),
            ),
          if (login == LoginItem.requiresApproval)
            Align(
              alignment: AlignmentDirectional.centerStart,
              child: Padding(
                padding: const EdgeInsets.only(top: 4, bottom: 6),
                child: OutlinedButton.icon(
                  key: const Key('settings-login-approve'),
                  onPressed: () =>
                      store.openSettingsPane(SettingsPane.loginItems),
                  icon: const Icon(AppIcons.external, size: AppIconSize.inline),
                  label: Text(l10n.loginApprovalOpen),
                ),
              ),
            ),
          const SizedBox(height: 10),
          ChoiceRow(
            label: l10n.language,
            child: GlassSegmented<String?>(
              key: const Key('settings-language'),
              expand: false,
              selected: prefs.locale?.languageCode,
              onChanged: (code) =>
                  prefs.setLocale(code == null ? null : Locale(code)),
              segments: [
                GlassSegment(
                  key: const Key('settings-language-system'),
                  value: null,
                  label: l10n.languageSystem,
                ),
                const GlassSegment(
                  key: Key('settings-language-en'),
                  value: 'en',
                  label: 'English',
                ),
                const GlassSegment(
                  key: Key('settings-language-zh'),
                  value: 'zh',
                  label: '中文',
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// Switch off and put every system change back, after asking.
class _RestoreCard extends StatelessWidget {
  const _RestoreCard({required this.store});

  final AppStore store;

  Future<void> _restore(BuildContext context) async {
    final l10n = context.l10n;
    final yes = await confirmDestructive(
      context,
      title: l10n.restoreConfirmTitle,
      body: l10n.restoreConfirmBody,
      action: l10n.restoreConfirmAction,
    );
    if (!yes) return;
    final done = await store.restore();
    if (done && context.mounted) {
      showToast(context, l10n.restoreDone, kind: ToastKind.success);
    }
  }

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    return GroupCard(
      title: l10n.restoreTitle,
      icon: AppIcons.restore,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            l10n.restoreBody,
            style: theme.textTheme.bodyMedium?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: 12),
          OutlinedButton.icon(
            key: const Key('settings-restore'),
            style: OutlinedButton.styleFrom(
              foregroundColor: theme.colorScheme.error,
            ),
            onPressed: () => unawaited(_restore(context)),
            icon: const Icon(AppIcons.restore, size: AppIconSize.inline),
            label: Text(l10n.restoreAction),
          ),
        ],
      ),
    );
  }
}

/// The System Settings panes Isthmus depends on, one click away.
class _PanesCard extends StatelessWidget {
  const _PanesCard({required this.store});

  final AppStore store;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return GroupCard(
      title: l10n.systemSettingsTitle,
      icon: AppIcons.external,
      padding: const EdgeInsets.fromLTRB(18, 16, 10, 10),
      child: Column(
        children: [
          for (final (pane, label, icon) in [
            (
              SettingsPane.accessibility,
              l10n.paneAccessibility,
              AppIcons.permission,
            ),
            (SettingsPane.keyboard, l10n.paneKeyboard, AppIcons.keyboard),
            (SettingsPane.mouse, l10n.paneMouse, AppIcons.mouse),
          ])
            ListTile(
              key: Key('settings-pane-${pane.name}'),
              dense: true,
              contentPadding: const EdgeInsets.symmetric(horizontal: 6),
              leading: Icon(icon, size: AppIconSize.button),
              title: Text(label),
              trailing: const Icon(AppIcons.external, size: AppIconSize.inline),
              onTap: () => store.openSettingsPane(pane),
            ),
        ],
      ),
    );
  }
}

/// What this is: the app's version and the library's, and whether a newer
/// release is out.
class _AboutCard extends StatefulWidget {
  const _AboutCard({
    required this.store,
    required this.prefs,
    required this.updates,
  });

  final AppStore store;
  final AppPrefs prefs;
  final UpdateController updates;

  @override
  State<_AboutCard> createState() => _AboutCardState();
}

class _AboutCardState extends State<_AboutCard> {
  String? _version;

  @override
  void initState() {
    super.initState();
    PackageInfo.fromPlatform()
        .then((info) {
          if (mounted) setState(() => _version = info.version);
        })
        .catchError((Object _) {
          // No version to show (a test, a broken bundle): say "unknown".
        });
  }

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final library = widget.store.status.version;
    final muted = theme.textTheme.bodyMedium?.copyWith(
      color: scheme.onSurfaceVariant,
    );
    Widget row(String label, String value, Key key) => Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Row(
        children: [
          Expanded(child: Text(label, style: theme.textTheme.bodyMedium)),
          Text(value, key: key, style: muted),
        ],
      ),
    );
    return GroupCard(
      title: l10n.aboutTitle,
      icon: AppIcons.info,
      trailing: const BrandMark(size: 22),
      child: Column(
        children: [
          row(
            l10n.aboutApp,
            _version ?? l10n.aboutUnknownVersion,
            const Key('settings-app-version'),
          ),
          row(
            l10n.aboutLibrary,
            library.isEmpty ? l10n.aboutUnknownVersion : library,
            const Key('settings-library-version'),
          ),
          // The app ships for macOS and Windows, which both update; shown
          // on any system, so tests on any host see it.
          _UpdateRow(updates: widget.updates, prefs: widget.prefs),
        ],
      ),
    );
  }
}

/// Look for a newer release, or install the one a check found, and whether
/// to look on their own.
class _UpdateRow extends StatelessWidget {
  const _UpdateRow({required this.updates, required this.prefs});

  final UpdateController updates;
  final AppPrefs prefs;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return ListenableBuilder(
      listenable: Listenable.merge([updates, prefs]),
      builder: (context, _) {
        final release = updates.published;
        final offer = updates.newer && release != null;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const SizedBox(height: 10),
            Align(
              alignment: AlignmentDirectional.centerStart,
              child: offer
                  ? FilledButton.tonalIcon(
                      key: const Key('settings-update'),
                      onPressed: () => showUpdateDialog(context, updates),
                      icon: const Icon(
                        AppIcons.update,
                        size: AppIconSize.inline,
                      ),
                      label: Text(l10n.updateTo('${release.version}')),
                    )
                  : OutlinedButton.icon(
                      key: const Key('settings-check-updates'),
                      onPressed: updates.busy
                          ? null
                          : () => showUpdateDialog(context, updates),
                      icon: const Icon(
                        AppIcons.update,
                        size: AppIconSize.inline,
                      ),
                      label: Text(l10n.checkForUpdates),
                    ),
            ),
            const SizedBox(height: 6),
            SwitchRow(
              switchKey: const Key('settings-auto-update'),
              title: l10n.autoCheckUpdates,
              subtitle: l10n.autoCheckUpdatesHint,
              value: prefs.autoCheckUpdates,
              onChanged: prefs.setAutoCheckUpdates,
            ),
          ],
        );
      },
    );
  }
}
