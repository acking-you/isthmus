import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/l10n/app_localizations.dart';
import 'package:isthmus_ui/src/common/l10n_extension.dart';
import 'package:isthmus_ui/src/ffi/models.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:isthmus_ui/src/shell/page_frame.dart';
import 'package:isthmus_ui/src/state/app_store.dart';
import 'package:isthmus_ui/src/theme/app_icons.dart';
import 'package:isthmus_ui/src/theme/app_theme.dart';
import 'package:isthmus_ui/src/theme/metrics.dart';
import 'package:isthmus_ui/src/widgets/status.dart';
import 'package:isthmus_ui/src/widgets/surfaces.dart';

/// The one question this page answers: are Windows habits on, and is
/// anything in the way? The switch and the one-click preset come first; what
/// blocks them (another instance, a missing permission) sits right under
/// them; the details of what is running come last.
class OverviewPage extends StatelessWidget {
  const OverviewPage({super.key});

  @override
  Widget build(BuildContext context) {
    final store = AppScope.of(context).store;
    return PageFrame(
      title: context.l10n.navOverview,
      body: ListenableBuilder(
        listenable: store,
        builder: (context, _) {
          final startError = store.startError;
          final issues = store.status.issues;
          return PageList(
            listKey: const Key('overview-list'),
            children: [
              if (store.anotherInstance) ...[
                const _AnotherInstanceCard(),
                const SizedBox(height: 16),
              ] else if (startError != null) ...[
                NoticeCard(
                  key: const Key('overview-start-failed'),
                  color: Theme.of(context).colorScheme.error,
                  icon: AppIcons.error,
                  title: context.l10n.startFailedTitle,
                  body: startError.message,
                ),
                const SizedBox(height: 16),
              ],
              _HeroCard(store: store),
              if (store.needsAccessibility) ...[
                const SizedBox(height: 16),
                _PermissionCard(store: store),
              ],
              const SizedBox(height: 16),
              _StatusCard(status: store.status),
              if (issues.isNotEmpty) ...[
                const SizedBox(height: 16),
                _IssuesCard(store: store, issues: issues),
              ],
            ],
          );
        },
      ),
    );
  }
}

/// The name of a preset's style, as the hero and the style controls say it.
String styleLabel(AppLocalizations l10n, ProfileStyle style) => switch (style) {
  ProfileStyle.windows => l10n.styleWindows,
  ProfileStyle.mac => l10n.styleMac,
  ProfileStyle.custom => l10n.styleCustom,
};

/// The one switch, and the one click that makes everything Windows.
class _HeroCard extends StatelessWidget {
  const _HeroCard({required this.store});

  final AppStore store;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final settings = store.settings;
    final keyboard = settings.keyboardStyle;
    final mouse = settings.mouseStyle;
    final allWindows =
        keyboard == ProfileStyle.windows && mouse == ProfileStyle.windows;
    final go = AppScope.of(context).go;
    return RaisedSurface(
      radius: kCardRadius,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(22, 20, 18, 20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Expanded(
                  child: Text(
                    l10n.heroTitle,
                    style: theme.textTheme.headlineSmall,
                  ),
                ),
                Transform.scale(
                  scale: 1.15,
                  child: Switch(
                    key: const Key('overview-enabled'),
                    value: settings.enabled,
                    onChanged: store.anotherInstance
                        ? null
                        : (on) => store.setEnabled(on),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 6),
            Text(
              settings.enabled ? l10n.heroOn : l10n.heroOff,
              style: theme.textTheme.bodyMedium?.copyWith(
                color: scheme.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: 14),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                _StyleChip(
                  key: const Key('overview-keyboard-style'),
                  icon: AppIcons.keyboard,
                  label: l10n.heroKeyboardStyle(styleLabel(l10n, keyboard)),
                  onTap: () => go(AppPage.keyboard),
                ),
                _StyleChip(
                  key: const Key('overview-mouse-style'),
                  icon: AppIcons.mouse,
                  label: l10n.heroMouseStyle(styleLabel(l10n, mouse)),
                  onTap: () => go(AppPage.mouse),
                ),
              ],
            ),
            if (!allWindows) ...[
              const SizedBox(height: 16),
              FilledButton.icon(
                key: const Key('overview-apply-windows'),
                onPressed: store.anotherInstance
                    ? null
                    : () => store.applyPreset(Preset.windows, PresetTarget.all),
                icon: const Icon(
                  AppIcons.applyWindows,
                  size: AppIconSize.button,
                ),
                label: Text(l10n.heroApplyWindows),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

/// A style as a quiet pill that opens its page.
class _StyleChip extends StatelessWidget {
  const _StyleChip({
    super.key,
    required this.icon,
    required this.label,
    required this.onTap,
  });

  final IconData icon;
  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Material(
      color: softWash(scheme),
      shape: const StadiumBorder(),
      child: InkWell(
        mouseCursor: clickable,
        customBorder: const StadiumBorder(),
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(10, 6, 12, 6),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                icon,
                size: AppIconSize.inline,
                color: scheme.onSurfaceVariant,
              ),
              const SizedBox(width: 6),
              Text(
                label,
                style: TextStyle(
                  fontSize: 12.5,
                  fontWeight: FontWeight.w500,
                  color: scheme.onSurface,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Accessibility is missing: what still works, what does not, and the way
/// to grant it.
class _PermissionCard extends StatelessWidget {
  const _PermissionCard({required this.store});

  final AppStore store;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return NoticeCard(
      key: const Key('overview-permission'),
      color: cautionColor(Theme.of(context).colorScheme),
      icon: AppIcons.permission,
      title: l10n.permissionTitle,
      body: l10n.permissionBody,
      footnote: l10n.permissionPaneNote,
      actions: [
        FilledButton(
          key: const Key('overview-grant'),
          onPressed: store.requestPermission,
          child: Text(l10n.permissionGrant),
        ),
        OutlinedButton.icon(
          key: const Key('overview-open-accessibility'),
          onPressed: () => store.openSettingsPane(SettingsPane.accessibility),
          icon: const Icon(AppIcons.external, size: AppIconSize.inline),
          label: Text(l10n.permissionOpenSettings),
        ),
      ],
    );
  }
}

/// Another Isthmus holds the keyboard and mouse: this one changes nothing,
/// and the user should know before flipping switches that do nothing.
class _AnotherInstanceCard extends StatelessWidget {
  const _AnotherInstanceCard();

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return NoticeCard(
      key: const Key('overview-another-instance'),
      color: cautionColor(Theme.of(context).colorScheme),
      icon: AppIcons.warning,
      title: l10n.anotherInstanceTitle,
      body: l10n.anotherInstanceBody,
    );
  }
}

/// What is running now, row by row.
class _StatusCard extends StatelessWidget {
  const _StatusCard({required this.status});

  final Status status;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final front = status.frontApp;
    final hid = switch (status.hidBackend) {
      HidBackend.full => null,
      HidBackend.limited => l10n.hidLimited,
      HidBackend.unavailable => l10n.hidUnavailable,
    };
    return GroupCard(
      key: const Key('overview-status'),
      title: l10n.statusTitle,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _StatusRow(
            label: l10n.statusKeyboardTap,
            value: TapChip(
              key: const Key('overview-keyboard-tap'),
              state: status.keyboardTap,
            ),
          ),
          _StatusRow(
            label: l10n.statusMouseTap,
            value: TapChip(
              key: const Key('overview-mouse-tap'),
              state: status.mouseTap,
            ),
          ),
          _StatusRow(
            label: l10n.statusSecureInput,
            value: StatusChip(
              key: const Key('overview-secure-input'),
              color: status.secureInput ? cautionColor(scheme) : scheme.outline,
              label: status.secureInput
                  ? l10n.secureInputOn
                  : l10n.secureInputOff,
            ),
            note: status.secureInput ? l10n.secureInputHint : null,
          ),
          _StatusRow(
            label: l10n.statusFrontApp,
            value: Text(
              front?.label ?? l10n.statusNoFrontApp,
              key: const Key('overview-front-app'),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              textAlign: TextAlign.end,
              style: theme.textTheme.bodyMedium?.copyWith(
                fontWeight: FontWeight.w500,
              ),
            ),
            note: front == null ? null : classHint(l10n, front.appClass),
          ),
          _StatusRow(
            label: l10n.statusKeyboardsMapped,
            value: _Count(status.keyboardsMapped),
          ),
          _StatusRow(
            label: l10n.statusMiceTuned,
            value: _Count(status.miceTuned),
            last: hid == null,
          ),
          if (hid != null)
            Padding(
              padding: const EdgeInsets.only(top: 10),
              child: Text(
                hid,
                style: theme.textTheme.bodySmall?.copyWith(
                  color: cautionColor(scheme),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// What an app's class means for Ctrl, in one line.
String classHint(AppLocalizations l10n, AppClass appClass) =>
    switch (appClass) {
      AppClass.standard => l10n.classStandardHint,
      AppClass.terminal => l10n.classTerminalHint,
      AppClass.nativeCtrl => l10n.classNativeCtrlHint,
      AppClass.passthrough => l10n.classPassthroughHint,
    };

/// A label, its value at the end, and an optional line under both.
class _StatusRow extends StatelessWidget {
  const _StatusRow({
    required this.label,
    required this.value,
    this.note,
    this.last = false,
  });

  final String label;
  final Widget value;
  final String? note;
  final bool last;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final note = this.note;
    return Container(
      padding: const EdgeInsets.symmetric(vertical: 9),
      decoration: BoxDecoration(
        border: last
            ? null
            : Border(
                bottom: BorderSide(color: theme.colorScheme.outlineVariant),
              ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Text(label, style: theme.textTheme.bodyMedium),
              const SizedBox(width: 16),
              Expanded(
                child: Align(alignment: Alignment.centerRight, child: value),
              ),
            ],
          ),
          if (note != null) ...[
            const SizedBox(height: 4),
            Text(
              note,
              style: theme.textTheme.bodySmall?.copyWith(
                color: quietText(theme.colorScheme),
              ),
            ),
          ],
        ],
      ),
    );
  }
}

/// A count at the end of a status row.
class _Count extends StatelessWidget {
  const _Count(this.count);

  final int count;

  @override
  Widget build(BuildContext context) => Text(
    '$count',
    style: Theme.of(context).textTheme.bodyMedium
        ?.copyWith(fontWeight: FontWeight.w600),
  );
}

/// Everything the user should know about, each in a sentence.
class _IssuesCard extends StatelessWidget {
  const _IssuesCard({required this.store, required this.issues});

  final AppStore store;
  final List<Issue> issues;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    return GroupCard(
      key: const Key('overview-issues'),
      title: l10n.issuesTitle,
      icon: AppIcons.info,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final (index, issue) in issues.indexed)
            Padding(
              padding: EdgeInsets.only(top: index == 0 ? 0 : 12),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Padding(
                    padding: const EdgeInsets.only(top: 7),
                    child: StatusDot(color: cautionColor(scheme), size: 7),
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          issueText(l10n, issue),
                          style: theme.textTheme.bodyMedium?.copyWith(
                            fontSize: 13.5,
                          ),
                        ),
                        if (issue.kind == IssueKind.systemModifierKeys)
                          Padding(
                            padding: const EdgeInsets.only(top: 6),
                            child: OutlinedButton.icon(
                              onPressed: () =>
                                  store.openSettingsPane(SettingsPane.keyboard),
                              icon: const Icon(
                                AppIcons.external,
                                size: AppIconSize.inline,
                              ),
                              label: Text(l10n.issueOpenKeyboardSettings),
                            ),
                          ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
        ],
      ),
    );
  }
}

/// An issue, explained.
String issueText(AppLocalizations l10n, Issue issue) => switch (issue.kind) {
  IssueKind.karabinerActive => l10n.issueKarabiner,
  IssueKind.systemModifierKeys => l10n.issueSystemModifierKeys(issue.device),
  IssueKind.foreignPointerTool => l10n.issueForeignPointerTool(issue.name),
  IssueKind.relaunchNeeded => l10n.issueRelaunchNeeded,
  IssueKind.restorePending => l10n.issueRestorePending(issue.count),
  IssueKind.settingsUnreadable => l10n.issueSettingsUnreadable,
  IssueKind.unknown => l10n.issueUnknown,
};
