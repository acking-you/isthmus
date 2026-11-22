import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/src/common/l10n_extension.dart';
import 'package:isthmus_ui/src/ffi/models.dart';
import 'package:isthmus_ui/src/theme/app_theme.dart';

/// A small solid dot with a soft glow of its own colour.
class StatusDot extends StatelessWidget {
  const StatusDot({super.key, required this.color, this.size = 8});

  final Color color;
  final double size;

  @override
  Widget build(BuildContext context) => Container(
    width: size,
    height: size,
    decoration: BoxDecoration(
      color: color,
      shape: BoxShape.circle,
      boxShadow: [
        BoxShadow(
          color: color.withValues(alpha: 0.45),
          blurRadius: 4,
          spreadRadius: 0.3,
        ),
      ],
    ),
  );
}

/// A dot and a word. [filled] sets it in a faint pill of its own colour, for
/// a card where a bare dot and text would float without weight.
class StatusChip extends StatelessWidget {
  const StatusChip({
    super.key,
    required this.color,
    required this.label,
    this.filled = true,
  });

  final Color color;
  final String label;
  final bool filled;

  @override
  Widget build(BuildContext context) {
    final row = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        StatusDot(color: color),
        const SizedBox(width: 6),
        Flexible(
          child: Text(
            label,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontSize: 12,
              fontWeight: FontWeight.w500,
              color: Theme.of(context).colorScheme.onSurfaceVariant,
            ),
          ),
        ),
      ],
    );
    if (!filled) return row;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 4),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.13),
        borderRadius: BorderRadius.circular(20),
      ),
      child: row,
    );
  }
}

/// An event tap's state: green running, grey off, amber waiting for
/// Accessibility, red failed.
class TapChip extends StatelessWidget {
  const TapChip({super.key, required this.state});

  final TapState state;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final l10n = context.l10n;
    final (color, label) = switch (state) {
      TapState.running => (successColor(scheme), l10n.tapRunning),
      TapState.off => (scheme.outline, l10n.tapOff),
      TapState.needsPermission => (
        cautionColor(scheme),
        l10n.tapNeedsPermission,
      ),
      TapState.failed => (scheme.error, l10n.tapFailed),
    };
    return StatusChip(color: color, label: label);
  }
}
