import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/l10n/app_localizations.dart';
import 'package:isthmus_ui/src/common/l10n_extension.dart';
import 'package:isthmus_ui/src/ffi/models.dart';
import 'package:isthmus_ui/src/pages/overview_page.dart'
    show classHint, styleLabel;
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:isthmus_ui/src/shell/page_frame.dart';
import 'package:isthmus_ui/src/state/app_store.dart';
import 'package:isthmus_ui/src/state/shortcut_catalog.dart';
import 'package:isthmus_ui/src/theme/app_icons.dart';
import 'package:isthmus_ui/src/theme/app_theme.dart';
import 'package:isthmus_ui/src/theme/fonts.dart';
import 'package:isthmus_ui/src/theme/glass.dart';
import 'package:isthmus_ui/src/theme/metrics.dart';
import 'package:isthmus_ui/src/theme/motion.dart';
import 'package:isthmus_ui/src/widgets/keycap.dart';
import 'package:isthmus_ui/src/widgets/setting_rows.dart';
import 'package:isthmus_ui/src/widgets/status.dart';
import 'package:isthmus_ui/src/widgets/surfaces.dart';

/// Which keys act as they do on Windows: the style for every keyboard, each
/// keyboard's own facts (layout, F-keys), the shortcut groups, and the apps
/// that need Ctrl left alone.
class KeyboardPage extends StatelessWidget {
  const KeyboardPage({super.key});

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final store = AppScope.of(context).store;
    return PageFrame(
      title: l10n.navKeyboard,
      body: ListenableBuilder(
        listenable: store,
        builder: (context, _) {
          final keyboards = store.devicesOf(DeviceKind.keyboard);
          final rules = store.settings.appRules;
          return PageList(
            listKey: const Key('keyboard-list'),
            children: [
              _StyleCard(store: store),
              SectionHeader(
                title: l10n.keyboardsSection,
                count: keyboards.length,
              ),
              if (keyboards.isEmpty)
                EmptyNote(icon: AppIcons.keyboard, text: l10n.keyboardNone)
              else
                for (final keyboard in keyboards)
                  _KeyboardCard(store: store, device: keyboard),
              SectionHeader(title: l10n.shortcutsSection),
              _Hint(l10n.shortcutsHint),
              for (final card in shortcutCatalog)
                _ShortcutCardView(store: store, card: card),
              SectionHeader(
                title: l10n.appRulesSection,
                count: rules.length,
                action: TextButton.icon(
                  key: const Key('keyboard-add-app'),
                  onPressed: () => _addApp(context, store),
                  icon: const Icon(AppIcons.add, size: AppIconSize.inline),
                  label: Text(l10n.appRulesAdd),
                ),
              ),
              _Hint(l10n.appRulesHint),
              _FrontAppLine(app: store.status.frontApp),
              if (rules.isEmpty)
                EmptyNote(icon: AppIcons.windows, text: l10n.appRulesNone)
              else
                for (final rule in rules) _RuleCard(store: store, rule: rule),
            ],
          );
        },
      ),
    );
  }
}

/// A quiet sentence under a section header.
class _Hint extends StatelessWidget {
  const _Hint(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(4, 0, 4, 12),
      child: Text(
        text,
        style: theme.textTheme.bodySmall?.copyWith(
          color: quietText(theme.colorScheme),
          fontSize: 12.5,
        ),
      ),
    );
  }
}

/// Windows or Mac for every keyboard at once; Custom shows, unchosen, when
/// the keyboards or groups are mixed.
class _StyleCard extends StatelessWidget {
  const _StyleCard({required this.store});

  final AppStore store;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final style = store.settings.keyboardStyle;
    return GroupCard(
      title: l10n.keyboardStyleTitle,
      icon: AppIcons.keyboard,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          StyleSegments(
            key: const Key('keyboard-style'),
            keyPrefix: 'keyboard-style',
            selected: style,
            withCustom: false,
            onChanged: (preset) =>
                store.applyPreset(preset, PresetTarget.keyboard),
          ),
          const SizedBox(height: 10),
          Text(
            l10n.keyboardStyleHint,
            style: theme.textTheme.bodySmall?.copyWith(
              color: quietText(theme.colorScheme),
            ),
          ),
        ],
      ),
    );
  }
}

/// Windows | Mac (| Custom) as a segmented control that applies a preset.
/// Custom is never chosen by a click: it is what mixed values are called,
/// shown selected (and only shown, unless [withCustom]) when they are.
class StyleSegments extends StatelessWidget {
  const StyleSegments({
    super.key,
    required this.keyPrefix,
    required this.selected,
    required this.onChanged,
    required this.withCustom,
  });

  final String keyPrefix;
  final ProfileStyle selected;
  final ValueChanged<Preset> onChanged;

  /// Show Custom even when the values match a preset.
  final bool withCustom;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return GlassSegmented<ProfileStyle>(
      expand: false,
      selected: selected,
      onChanged: (style) {
        switch (style) {
          case ProfileStyle.windows:
            onChanged(Preset.windows);
          case ProfileStyle.mac:
            onChanged(Preset.mac);
          case ProfileStyle.custom:
            // Disabled: mixed values are named, never chosen.
            break;
        }
      },
      segments: [
        GlassSegment(
          key: Key('$keyPrefix-windows'),
          value: ProfileStyle.windows,
          label: styleLabel(l10n, ProfileStyle.windows),
        ),
        GlassSegment(
          key: Key('$keyPrefix-mac'),
          value: ProfileStyle.mac,
          label: styleLabel(l10n, ProfileStyle.mac),
        ),
        if (withCustom || selected == ProfileStyle.custom)
          GlassSegment(
            key: Key('$keyPrefix-custom'),
            value: ProfileStyle.custom,
            label: styleLabel(l10n, ProfileStyle.custom),
            enabled: false,
          ),
      ],
    );
  }
}

/// A device's name, where it comes from, and whether it is here.
class DeviceHeader extends StatelessWidget {
  const DeviceHeader({super.key, required this.device, required this.icon});

  final DeviceEntry device;
  final IconData icon;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final origin = [
      if (device.builtIn) l10n.deviceBuiltIn,
      if (device.manufacturer.isNotEmpty) device.manufacturer,
      if (device.transport.isNotEmpty) device.transport,
    ].join(' · ');
    return Row(
      children: [
        Icon(icon, size: AppIconSize.button, color: scheme.onSurfaceVariant),
        const SizedBox(width: 12),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                device.name.isEmpty ? device.id : device.name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: theme.textTheme.titleSmall?.copyWith(fontSize: 14),
              ),
              if (origin.isNotEmpty)
                Text(
                  origin,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.bodySmall?.copyWith(
                    color: quietText(scheme),
                  ),
                ),
            ],
          ),
        ),
        const SizedBox(width: 8),
        StatusChip(
          color: device.connected ? successColor(scheme) : scheme.outline,
          label: device.connected
              ? l10n.deviceConnected
              : l10n.deviceDisconnected,
          filled: false,
        ),
      ],
    );
  }
}

String _layoutName(AppLocalizations l10n, Layout layout) => switch (layout) {
  Layout.mac => l10n.layoutMac,
  Layout.pc => l10n.layoutPc,
};

String _confidenceName(AppLocalizations l10n, Confidence c) => switch (c) {
  Confidence.low => l10n.confidenceLow,
  Confidence.medium => l10n.confidenceMedium,
  Confidence.high => l10n.confidenceHigh,
};

/// One keyboard: its layout (detected, or set), its style, and its F-row.
class _KeyboardCard extends StatelessWidget {
  const _KeyboardCard({required this.store, required this.device});

  final AppStore store;
  final DeviceEntry device;

  Future<bool> _edit({
    KeyboardStyle? style,
    LayoutChoice? layout,
    FunctionKeys? functionKeys,
  }) => store.update(
    (s) => s.withKeyboardDevice(
      device.id,
      name: device.name,
      style: style,
      layout: layout,
      functionKeys: functionKeys,
    ),
  );

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final settings = store.settings;
    final id = device.id;
    return ListCard(
      key: Key('keyboard-device-$id'),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          DeviceHeader(device: device, icon: AppIcons.keyboard),
          if (device.viaKvm) ...[
            const SizedBox(height: 10),
            Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Icon(
                  AppIcons.kvm,
                  size: AppIconSize.inline,
                  color: cautionColor(scheme),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    l10n.deviceKvmHint,
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: scheme.onSurfaceVariant,
                    ),
                  ),
                ),
              ],
            ),
          ],
          const SizedBox(height: 12),
          ChoiceRow(
            label: l10n.keyboardLayout,
            child: GlassSegmented<LayoutChoice>(
              key: Key('keyboard-layout-$id'),
              expand: false,
              selected: settings.layoutOf(id),
              onChanged: (layout) => _edit(layout: layout),
              segments: [
                for (final choice in LayoutChoice.values)
                  GlassSegment(
                    key: Key('keyboard-layout-$id-${choice.name}'),
                    value: choice,
                    label: switch (choice) {
                      LayoutChoice.auto => l10n.layoutAuto,
                      LayoutChoice.mac => l10n.layoutMac,
                      LayoutChoice.pc => l10n.layoutPc,
                    },
                  ),
              ],
            ),
          ),
          Padding(
            padding: const EdgeInsets.only(top: 4, bottom: 6),
            child: Text(
              l10n.layoutDetected(
                _layoutName(l10n, device.detectedLayout),
                _confidenceName(l10n, device.layoutConfidence),
              ),
              style: theme.textTheme.bodySmall?.copyWith(
                color: quietText(scheme),
              ),
            ),
          ),
          SwitchRow(
            switchKey: Key('keyboard-style-$id'),
            title: l10n.keyboardWindowsStyle,
            value: settings.keyboardStyleOf(id) == KeyboardStyle.windows,
            onChanged: (on) =>
                _edit(style: on ? KeyboardStyle.windows : KeyboardStyle.mac),
          ),
          SwitchRow(
            switchKey: Key('keyboard-fkeys-$id'),
            title: l10n.keyboardFunctionKeys,
            subtitle: l10n.keyboardFunctionKeysHint,
            value: settings.functionKeysOf(id) == FunctionKeys.standard,
            onChanged: (on) => _edit(
              functionKeys: on ? FunctionKeys.standard : FunctionKeys.system,
            ),
          ),
        ],
      ),
    );
  }
}

/// One shortcut group: its switch, and its examples behind a disclosure, so
/// at rest the page shows names and values.
class _ShortcutCardView extends StatefulWidget {
  const _ShortcutCardView({required this.store, required this.card});

  final AppStore store;
  final ShortcutCard card;

  @override
  State<_ShortcutCardView> createState() => _ShortcutCardViewState();
}

class _ShortcutCardViewState extends State<_ShortcutCardView> {
  bool _open = false;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final card = widget.card;
    final group = card.group;
    final store = widget.store;
    final examples = _open
        ? Padding(
            padding: const EdgeInsets.fromLTRB(30, 0, 4, 10),
            child: Column(
              children: [
                for (final example in card.examples)
                  _ExampleRow(example: example),
              ],
            ),
          )
        : const SizedBox(width: double.infinity);
    return ListCard(
      key: Key('keyboard-card-${card.id}'),
      padding: const EdgeInsets.fromLTRB(16, 14, 12, 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Icon(
                card.icon,
                size: AppIconSize.button,
                color: scheme.onSurfaceVariant,
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      card.title(l10n),
                      style: theme.textTheme.titleSmall?.copyWith(fontSize: 14),
                    ),
                    const SizedBox(height: 2),
                    Text(
                      card.summary(l10n),
                      style: theme.textTheme.bodySmall?.copyWith(
                        color: quietText(scheme),
                      ),
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 12),
              if (group == null)
                Text(
                  l10n.alwaysOn,
                  style: theme.textTheme.bodySmall?.copyWith(
                    color: scheme.onSurfaceVariant,
                    fontWeight: FontWeight.w500,
                  ),
                )
              else
                Switch(
                  key: Key('keyboard-group-${card.id}'),
                  value: store.settings.groupEnabled(group),
                  onChanged: (on) =>
                      store.update((s) => s.withGroup(group, on)),
                ),
            ],
          ),
          Align(
            alignment: AlignmentDirectional.centerStart,
            child: Padding(
              padding: const EdgeInsets.only(left: 18),
              child: TextButton.icon(
                key: Key('keyboard-card-${card.id}-expand'),
                style: TextButton.styleFrom(
                  foregroundColor: scheme.onSurfaceVariant,
                  // A button's text style replaces the inherited one, so it
                  // carries the theme's face rather than a bare size.
                  textStyle: theme.textTheme.bodySmall?.copyWith(
                    fontSize: 12.5,
                    fontWeight: FontWeight.w500,
                  ),
                ),
                onPressed: () => setState(() => _open = !_open),
                icon: Icon(
                  _open ? AppIcons.chevronUp : AppIcons.chevronDown,
                  size: AppIconSize.inline,
                ),
                label: Text(_open ? l10n.hideExamples : l10n.showExamples),
              ),
            ),
          ),
          // A zero-length AnimatedSize finishes inside its own layout, which
          // Flutter forbids, so reduced motion swaps it for the plain child.
          if (Motion.reduced(context))
            examples
          else
            AnimatedSize(
              duration: Motion.medium,
              curve: Motion.move,
              alignment: Alignment.topCenter,
              child: examples,
            ),
        ],
      ),
    );
  }
}

/// A Windows chord, what it becomes on the Mac, and what it does.
class _ExampleRow extends StatelessWidget {
  const _ExampleRow({required this.example});

  final ShortcutExample example;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final mac = example.mac;
    final app = example.app;
    final does = app == null
        ? example.does(l10n)
        : '${app(l10n)} · ${example.does(l10n)}';
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 5),
      child: Row(
        children: [
          Expanded(flex: 5, child: KeyChord(example.windows)),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 6),
            child: Icon(
              AppIcons.chevronRight,
              size: AppIconSize.inline,
              color: onSurfaceMuted(scheme),
            ),
          ),
          Expanded(
            flex: 4,
            child: mac == null
                ? const SizedBox.shrink()
                : KeyChord(mac, mac: true),
          ),
          const SizedBox(width: 8),
          Expanded(
            flex: 5,
            child: Text(
              does,
              style: theme.textTheme.bodySmall?.copyWith(
                color: scheme.onSurfaceVariant,
                fontSize: 12.5,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// The app in front now, and how it is treated.
class _FrontAppLine extends StatelessWidget {
  const _FrontAppLine({required this.app});

  final FrontApp? app;

  @override
  Widget build(BuildContext context) {
    final app = this.app;
    if (app == null) return const SizedBox.shrink();
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    return Padding(
      padding: const EdgeInsets.fromLTRB(4, 0, 4, 12),
      child: Row(
        children: [
          StatusDot(color: signalColor(scheme), size: 7),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              '${l10n.appRulesFront(app.label)} · ${classHint(l10n, app.appClass)}',
              key: const Key('keyboard-front-app'),
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: theme.textTheme.bodySmall?.copyWith(
                color: scheme.onSurfaceVariant,
                fontSize: 12.5,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// What an app class is called in the rule menu.
String className(AppLocalizations l10n, AppClass appClass) =>
    switch (appClass) {
      AppClass.standard => l10n.classStandard,
      AppClass.terminal => l10n.classTerminal,
      AppClass.nativeCtrl => l10n.classNativeCtrl,
      AppClass.passthrough => l10n.classPassthrough,
    };

/// One of the user's rules: the bundle id, its class, and a way to drop it.
class _RuleCard extends StatelessWidget {
  const _RuleCard({required this.store, required this.rule});

  final AppStore store;
  final AppRule rule;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final id = rule.bundleId;
    return ListCard(
      key: Key('keyboard-rule-$id'),
      padding: const EdgeInsets.fromLTRB(16, 8, 8, 8),
      child: Row(
        children: [
          Expanded(
            child: Text(
              id,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: mono(fontSize: 12.5, color: scheme.onSurface),
            ),
          ),
          const SizedBox(width: 8),
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 12),
            decoration: BoxDecoration(
              color: fieldFill(scheme),
              borderRadius: BorderRadius.circular(kControlRadius),
            ),
            child: DropdownButtonHideUnderline(
              child: DropdownButton<AppClass>(
                key: Key('keyboard-rule-$id-class'),
                value: rule.appClass,
                isDense: true,
                borderRadius: BorderRadius.circular(kMenuRadius),
                style: theme.textTheme.bodyMedium?.copyWith(fontSize: 13),
                icon: const Padding(
                  padding: EdgeInsets.only(left: 6),
                  child: Icon(AppIcons.chevronDown, size: AppIconSize.inline),
                ),
                items: [
                  for (final appClass in AppClass.values)
                    DropdownMenuItem(
                      value: appClass,
                      child: Text(className(l10n, appClass)),
                    ),
                ],
                onChanged: (appClass) {
                  if (appClass == null || appClass == rule.appClass) return;
                  store.update(
                    (s) => s.withAppRule(
                      AppRule(bundleId: id, appClass: appClass),
                    ),
                  );
                },
              ),
            ),
          ),
          IconButton(
            key: Key('keyboard-rule-$id-remove'),
            tooltip: l10n.appRulesRemove,
            onPressed: () => store.update((s) => s.withoutAppRule(id)),
            icon: const Icon(AppIcons.delete),
          ),
        ],
      ),
    );
  }
}

/// Pick a running app and give it a rule. The rule starts as the class the
/// app has now, so adding it changes nothing until the user picks another.
Future<void> _addApp(BuildContext context, AppStore store) async {
  final taken = {for (final rule in store.settings.appRules) rule.bundleId};
  final apps = store.runningApps();
  final picked = await showDialog<FrontApp>(
    context: context,
    builder: (c) => _AddAppDialog(apps: apps, taken: taken),
  );
  if (picked == null) return;
  await store.update(
    (s) => s.withAppRule(
      AppRule(bundleId: picked.bundleId, appClass: picked.appClass),
    ),
  );
}

/// The running apps without a rule yet, by name; picking one closes the
/// dialog with it.
class _AddAppDialog extends StatelessWidget {
  const _AddAppDialog({required this.apps, required this.taken});

  final Future<List<FrontApp>> apps;
  final Set<String> taken;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final size = MediaQuery.sizeOf(context);
    return AlertDialog(
      title: Text(l10n.addAppTitle),
      contentPadding: const EdgeInsets.fromLTRB(12, 16, 12, 8),
      content: SizedBox(
        width: 420,
        height: size.height * 0.5,
        child: FutureBuilder<List<FrontApp>>(
          future: apps,
          builder: (context, snapshot) {
            final list =
                [
                  for (final app in snapshot.data ?? const <FrontApp>[])
                    if (app.bundleId.isNotEmpty &&
                        !taken.contains(app.bundleId))
                      app,
                ]..sort(
                  (a, b) =>
                      a.label.toLowerCase().compareTo(b.label.toLowerCase()),
                );
            if (snapshot.connectionState != ConnectionState.done) {
              return const Center(child: CircularProgressIndicator());
            }
            if (list.isEmpty) {
              return Center(
                child: Text(
                  l10n.addAppEmpty,
                  style: theme.textTheme.bodyMedium?.copyWith(
                    color: theme.colorScheme.onSurfaceVariant,
                  ),
                ),
              );
            }
            return ListView(
              children: [
                for (final app in list)
                  ListTile(
                    key: Key('add-app-${app.bundleId}'),
                    title: Text(app.label),
                    subtitle: Text(
                      '${app.bundleId} · ${className(l10n, app.appClass)}',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                    onTap: () => Navigator.of(context).pop(app),
                  ),
              ],
            );
          },
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l10n.cancel),
        ),
      ],
    );
  }
}
