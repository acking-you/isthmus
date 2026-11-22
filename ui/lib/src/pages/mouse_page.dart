import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/src/common/l10n_extension.dart';
import 'package:isthmus_ui/src/ffi/models.dart';
import 'package:isthmus_ui/src/pages/keyboard_page.dart'
    show DeviceHeader, StyleSegments;
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:isthmus_ui/src/shell/page_frame.dart';
import 'package:isthmus_ui/src/state/app_store.dart';
import 'package:isthmus_ui/src/theme/app_icons.dart';
import 'package:isthmus_ui/src/theme/app_theme.dart';
import 'package:isthmus_ui/src/theme/glass.dart';
import 'package:isthmus_ui/src/widgets/setting_rows.dart';
import 'package:isthmus_ui/src/widgets/surfaces.dart';

/// A change to one field of a profile, applied to whatever the profile is
/// when the write is sent, so two quick changes never undo each other.
typedef ProfileEdit = MouseProfile Function(MouseProfile profile);

/// How the wheel and the pointer behave: the style, the profile every mouse
/// gets, each mouse's own, and the trackpads Isthmus leaves alone.
class MousePage extends StatelessWidget {
  const MousePage({super.key});

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final store = AppScope.of(context).store;
    return PageFrame(
      title: l10n.navMouse,
      body: ListenableBuilder(
        listenable: store,
        builder: (context, _) {
          final mice = store.devicesOf(DeviceKind.mouse);
          final trackpads = store.devicesOf(DeviceKind.trackpad);
          return PageList(
            listKey: const Key('mouse-list'),
            children: [
              _StyleCard(store: store),
              const SizedBox(height: 16),
              GroupCard(
                key: const Key('mouse-defaults'),
                title: l10n.mouseDefaultsTitle,
                icon: AppIcons.mouse,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    _Quiet(l10n.mouseDefaultsHint),
                    const SizedBox(height: 8),
                    ProfileEditor(
                      keyPrefix: 'mouse-defaults',
                      profile: store.settings.mouseDefaults,
                      onEdit: (edit) => store.update(
                        (s) => s.withMouseDefaults(edit(s.mouseDefaults)),
                      ),
                    ),
                  ],
                ),
              ),
              SectionHeader(title: l10n.miceSection, count: mice.length),
              if (mice.isEmpty)
                EmptyNote(icon: AppIcons.mouse, text: l10n.miceNone)
              else
                for (final mouse in mice)
                  _MouseCard(store: store, device: mouse),
              if (trackpads.isNotEmpty) ...[
                SectionHeader(
                  title: l10n.trackpadsSection,
                  count: trackpads.length,
                ),
                for (final trackpad in trackpads)
                  ListCard(
                    key: Key('mouse-trackpad-${trackpad.id}'),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        DeviceHeader(device: trackpad, icon: AppIcons.trackpad),
                        const SizedBox(height: 8),
                        _Quiet(l10n.trackpadLeftAlone),
                      ],
                    ),
                  ),
              ],
            ],
          );
        },
      ),
    );
  }
}

/// A quiet line of help under a title.
class _Quiet extends StatelessWidget {
  const _Quiet(this.text);

  final String text;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Text(
      text,
      style: theme.textTheme.bodySmall?.copyWith(
        color: quietText(theme.colorScheme),
      ),
    );
  }
}

/// Windows | Mac | Custom for every mouse at once.
class _StyleCard extends StatelessWidget {
  const _StyleCard({required this.store});

  final AppStore store;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return GroupCard(
      title: l10n.mouseStyleTitle,
      icon: AppIcons.wheel,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          StyleSegments(
            key: const Key('mouse-style'),
            keyPrefix: 'mouse-style',
            selected: store.settings.mouseStyle,
            withCustom: true,
            onChanged: (preset) =>
                store.applyPreset(preset, PresetTarget.mouse),
          ),
          const SizedBox(height: 10),
          _Quiet(l10n.mouseStyleHint),
        ],
      ),
    );
  }
}

/// One mouse: the defaults, or a profile of its own.
class _MouseCard extends StatelessWidget {
  const _MouseCard({required this.store, required this.device});

  final AppStore store;
  final DeviceEntry device;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final id = device.id;
    final own = store.settings.mouseDevice(id);
    return ListCard(
      key: Key('mouse-device-$id'),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          DeviceHeader(device: device, icon: AppIcons.mouse),
          const SizedBox(height: 8),
          SwitchRow(
            switchKey: Key('mouse-use-defaults-$id'),
            title: l10n.mouseUseDefaults,
            value: own == null,
            // Leaving the defaults starts from them, so nothing moves until
            // the user changes something.
            onChanged: (useDefaults) => store.update(
              (s) => s.withMouseDevice(
                id,
                name: device.name,
                profile: useDefaults ? null : s.mouseDefaults,
              ),
            ),
          ),
          if (own != null) ...[
            const Divider(height: 20),
            ProfileEditor(
              keyPrefix: 'mouse-$id',
              profile: own.profile,
              onEdit: (edit) => store.update(
                (s) => s.withMouseDevice(
                  id,
                  name: device.name,
                  profile: edit(s.mouseProfileOf(id)),
                ),
              ),
            ),
          ],
        ],
      ),
    );
  }
}

/// Every setting of a [MouseProfile]. A number is enabled only while the
/// switch it tunes is on, as it counts only then.
class ProfileEditor extends StatelessWidget {
  const ProfileEditor({
    super.key,
    required this.keyPrefix,
    required this.profile,
    required this.onEdit,
  });

  /// Keys are `<keyPrefix>-direction`, `-linear`, `-lines`, `-pointer`,
  /// `-speed`, `-side` and `-zoom`.
  final String keyPrefix;
  final MouseProfile profile;
  final ValueChanged<ProfileEdit> onEdit;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final p = profile;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        ChoiceRow(
          label: l10n.wheelDirection,
          child: GlassSegmented<bool>(
            key: Key('$keyPrefix-direction'),
            expand: false,
            selected: p.reverseWheel,
            onChanged: (reverse) =>
                onEdit((m) => m.copyWith(reverseWheel: reverse)),
            segments: [
              GlassSegment(
                key: Key('$keyPrefix-direction-windows'),
                value: true,
                label: l10n.styleWindows,
              ),
              GlassSegment(
                key: Key('$keyPrefix-direction-mac'),
                value: false,
                label: l10n.styleMacOS,
              ),
            ],
          ),
        ),
        const SizedBox(height: 6),
        SwitchRow(
          switchKey: Key('$keyPrefix-linear'),
          title: l10n.linearWheel,
          value: p.linearWheel,
          onChanged: (on) => onEdit((m) => m.copyWith(linearWheel: on)),
        ),
        SliderRow(
          sliderKey: Key('$keyPrefix-lines'),
          label: l10n.linesPerNotch,
          value: p.linesPerNotch,
          min: MouseProfile.minLines,
          max: MouseProfile.maxLines,
          valueLabel: l10n.linesValue,
          onChanged: p.linearWheel
              ? (lines) => onEdit((m) => m.copyWith(linesPerNotch: lines))
              : null,
        ),
        const SizedBox(height: 6),
        ChoiceRow(
          label: l10n.pointerMode,
          child: GlassSegmented<PointerMode>(
            key: Key('$keyPrefix-pointer'),
            expand: false,
            selected: p.pointer,
            onChanged: (mode) => onEdit((m) => m.copyWith(pointer: mode)),
            segments: [
              GlassSegment(
                key: Key('$keyPrefix-pointer-windows'),
                value: PointerMode.windows,
                label: l10n.styleWindows,
              ),
              GlassSegment(
                key: Key('$keyPrefix-pointer-mac'),
                value: PointerMode.system,
                label: l10n.styleMacOS,
              ),
            ],
          ),
        ),
        SliderRow(
          sliderKey: Key('$keyPrefix-speed'),
          label: l10n.pointerSpeed,
          value: p.pointerSpeed,
          min: MouseProfile.minSpeed,
          max: MouseProfile.maxSpeed,
          valueLabel: l10n.pointerSpeedValue,
          marker: (MouseProfile.windowsSpeed, l10n.pointerSpeedWindowsDefault),
          onChanged: p.pointer == PointerMode.windows
              ? (speed) => onEdit((m) => m.copyWith(pointerSpeed: speed))
              : null,
        ),
        SwitchRow(
          switchKey: Key('$keyPrefix-side'),
          title: l10n.sideButtons,
          subtitle: l10n.sideButtonsHint,
          value: p.sideButtons,
          onChanged: (on) => onEdit((m) => m.copyWith(sideButtons: on)),
        ),
        SwitchRow(
          switchKey: Key('$keyPrefix-zoom'),
          title: l10n.ctrlWheelZoom,
          value: p.ctrlWheelZoom,
          onChanged: (on) => onEdit((m) => m.copyWith(ctrlWheelZoom: on)),
        ),
      ],
    );
  }
}
