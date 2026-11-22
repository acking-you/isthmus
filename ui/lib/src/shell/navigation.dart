import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/src/common/l10n_extension.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:isthmus_ui/src/shell/window_chrome.dart';
import 'package:isthmus_ui/src/state/app_store.dart';
import 'package:isthmus_ui/src/theme/app_icons.dart';
import 'package:isthmus_ui/src/theme/app_theme.dart';
import 'package:isthmus_ui/src/theme/glass.dart';
import 'package:isthmus_ui/src/theme/metrics.dart';
import 'package:isthmus_ui/src/theme/motion.dart';
import 'package:isthmus_ui/src/widgets/brand_mark.dart';
import 'package:isthmus_ui/src/widgets/status.dart';

/// The rail's width. The window's minimum (760) leaves the page 564 beside it.
const double kRailWidth = 196;

const double _rowHeight = 38;
const double _rowGap = 2;

/// The pages as a rail on the backdrop: the selection is a pane of glass
/// that glides, with a little spring, to the chosen row. At its foot, the one
/// fact every page depends on: whether Windows habits are on.
class NavRail extends StatelessWidget {
  const NavRail({super.key, required this.page, required this.onSelect});

  final AppPage page;
  final ValueChanged<AppPage> onSelect;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final step = MediaQuery.textScalerOf(context).scale(_rowHeight);
    final pages = AppPage.available;
    final index = pages.indexOf(page).clamp(0, pages.length - 1);
    return Material(
      type: MaterialType.transparency,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // The window's corner: the traffic lights live here, and it drags.
          SizedBox(height: trafficLightHeight, child: const WindowDragArea()),
          WindowDragArea(
            child: Padding(
              padding: const EdgeInsets.fromLTRB(20, 0, 12, 18),
              child: Row(
                children: [
                  const BrandMark(size: 26),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(
                      context.l10n.appTitle,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(
                        fontSize: 15,
                        fontWeight: FontWeight.w600,
                        letterSpacing: -0.1,
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Stack(
              children: [
                AnimatedPositioned(
                  duration: Motion.of(
                    context,
                    const Duration(milliseconds: 520),
                  ),
                  curve: Motion.spring,
                  top: index * (step + _rowGap),
                  left: 0,
                  right: 0,
                  height: step,
                  child: const IgnorePointer(
                    child: Glass(
                      depth: GlassDepth.chrome,
                      borderRadius: BorderRadius.all(Radius.circular(999)),
                      child: SizedBox.expand(),
                    ),
                  ),
                ),
                Column(
                  children: [
                    for (final entry in pages) ...[
                      SizedBox(
                        height: step,
                        child: _RailRow(
                          page: entry,
                          selected: entry == page,
                          onTap: () => onSelect(entry),
                          hover: backdropHover(scheme),
                        ),
                      ),
                      const SizedBox(height: _rowGap),
                    ],
                  ],
                ),
              ],
            ),
          ),
          const Expanded(child: WindowDragArea()),
          // Windows habits are a Mac's; a PC's rail has no switch to show.
          if (hasInputPages)
            Padding(
              padding: const EdgeInsets.fromLTRB(10, 0, 10, 12),
              child: _StatusFoot(onTap: () => onSelect(AppPage.overview)),
            ),
        ],
      ),
    );
  }
}

/// One page in the rail: its icon and name, a pointing hand, no tap on
/// the page already shown.
class _RailRow extends StatelessWidget {
  const _RailRow({
    required this.page,
    required this.selected,
    required this.onTap,
    required this.hover,
  });

  final AppPage page;
  final bool selected;
  final VoidCallback onTap;
  final Color hover;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final ink = selected ? scheme.onSurface : scheme.onSurfaceVariant;
    return Semantics(
      selected: selected,
      button: true,
      child: InkWell(
        key: Key('nav-${page.name}'),
        mouseCursor: clickable,
        hoverColor: hover,
        borderRadius: BorderRadius.circular(999),
        onTap: selected ? null : onTap,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 14),
          child: Row(
            children: [
              Icon(page.icon, size: AppIconSize.nav, color: ink),
              const SizedBox(width: 11),
              Expanded(
                child: Text(
                  page.label(context),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(
                    fontSize: 13.5,
                    fontWeight: selected ? FontWeight.w600 : FontWeight.w400,
                    color: ink,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The state of the one switch at the foot of the rail; opens the overview.
class _StatusFoot extends StatelessWidget {
  const _StatusFoot({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final store = AppScope.of(context).store;
    return ListenableBuilder(
      listenable: store,
      builder: (context, _) {
        final (color, label) = railStatus(context, store);
        return Glass(
          depth: GlassDepth.chrome,
          borderRadius: BorderRadius.circular(kRowRadius),
          child: InkWell(
            key: const Key('rail-status'),
            mouseCursor: clickable,
            borderRadius: BorderRadius.circular(kRowRadius),
            onTap: onTap,
            child: Padding(
              padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
              child: StatusChip(color: color, label: label, filled: false),
            ),
          ),
        );
      },
    );
  }
}

/// The one line that sums Isthmus up, with its colour: what is in the way
/// first, then on or off. The tray's first line says the same.
(Color, String) railStatus(BuildContext context, AppStore store) {
  final scheme = Theme.of(context).colorScheme;
  final l10n = context.l10n;
  if (store.anotherInstance) {
    return (cautionColor(scheme), l10n.statusAnotherInstance);
  }
  if (!store.settings.enabled) return (scheme.outline, l10n.statusOff);
  if (store.needsAccessibility) {
    return (cautionColor(scheme), l10n.statusNeedsPermission);
  }
  return (successColor(scheme), l10n.statusOn);
}
