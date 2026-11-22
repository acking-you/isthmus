import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/src/theme/app_theme.dart';
import 'package:isthmus_ui/src/theme/metrics.dart';

/// A name, an optional line under it, and a switch at the end. At rest a
/// form shows names and values, so the line under it is short.
class SwitchRow extends StatelessWidget {
  const SwitchRow({
    super.key,
    required this.switchKey,
    required this.title,
    required this.value,
    required this.onChanged,
    this.subtitle,
  });

  /// The switch's own key, so a test taps the control rather than the row.
  final Key switchKey;
  final String title;
  final String? subtitle;
  final bool value;

  /// Null shows the switch disabled.
  final ValueChanged<bool>? onChanged;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final subtitle = this.subtitle;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(title, style: theme.textTheme.bodyMedium),
                if (subtitle != null)
                  Text(
                    subtitle,
                    style: theme.textTheme.bodySmall?.copyWith(
                      color: quietText(theme.colorScheme),
                    ),
                  ),
              ],
            ),
          ),
          const SizedBox(width: 12),
          Switch(key: switchKey, value: value, onChanged: onChanged),
        ],
      ),
    );
  }
}

/// A label with its control at the end, or beneath it when the row is too
/// narrow for both.
class ChoiceRow extends StatelessWidget {
  const ChoiceRow({super.key, required this.label, required this.child});

  final String label;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final text = Text(label, style: Theme.of(context).textTheme.bodyMedium);
    return LayoutBuilder(
      builder: (context, box) => box.maxWidth >= 440
          ? Row(
              children: [
                Expanded(child: text),
                const SizedBox(width: 12),
                child,
              ],
            )
          : Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                text,
                const SizedBox(height: 8),
                SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: child,
                ),
              ],
            ),
    );
  }
}

/// The slider's horizontal inset, and the track's height: with both fixed the
/// position of a value on the track is known, so a marker can sit under it.
const double _sliderInset = 12;
const double _trackHeight = 4;

/// A whole-number slider with its value written beside the label, written
/// back only when the drag ends (a drag is one change, not twenty), and an
/// optional [marker] under the track at a value worth finding again.
class SliderRow extends StatefulWidget {
  const SliderRow({
    super.key,
    required this.sliderKey,
    required this.label,
    required this.value,
    required this.min,
    required this.max,
    required this.valueLabel,
    required this.onChanged,
    this.marker,
  });

  final Key sliderKey;
  final String label;
  final int value;
  final int min;
  final int max;

  /// How a value is written: "3 lines".
  final String Function(int value) valueLabel;

  /// Null shows the slider disabled.
  final ValueChanged<int>? onChanged;

  /// A value and its name, marked under the track: Windows' default speed.
  final (int, String)? marker;

  @override
  State<SliderRow> createState() => _SliderRowState();
}

class _SliderRowState extends State<SliderRow> {
  /// The value under the thumb while it is dragged.
  double? _live;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final onChanged = widget.onChanged;
    final shown = (_live ?? widget.value.toDouble())
        .clamp(widget.min.toDouble(), widget.max.toDouble())
        .toDouble();
    final marker = widget.marker;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Expanded(
                child: Text(widget.label, style: theme.textTheme.bodyMedium),
              ),
              Text(
                widget.valueLabel(shown.round()),
                style: theme.textTheme.bodyMedium?.copyWith(
                  color: onChanged == null
                      ? onSurfaceMuted(scheme)
                      : scheme.onSurfaceVariant,
                  fontWeight: FontWeight.w500,
                ),
              ),
            ],
          ),
          SliderTheme(
            data: SliderTheme.of(context).copyWith(
              trackHeight: _trackHeight,
              trackShape: const RoundedRectSliderTrackShape(),
            ),
            child: Slider(
              key: widget.sliderKey,
              padding: const EdgeInsets.symmetric(
                horizontal: _sliderInset,
                vertical: 10,
              ),
              value: shown,
              min: widget.min.toDouble(),
              max: widget.max.toDouble(),
              divisions: widget.max - widget.min,
              mouseCursor: clickable,
              onChanged: onChanged == null
                  ? null
                  : (value) => setState(() => _live = value),
              onChangeEnd: onChanged == null
                  ? null
                  : (value) {
                      setState(() => _live = null);
                      if (value.round() != widget.value) {
                        onChanged(value.round());
                      }
                    },
            ),
          ),
          if (marker != null)
            _Marker(
              fraction: (marker.$1 - widget.min) / (widget.max - widget.min),
              label: marker.$2,
            ),
        ],
      ),
    );
  }
}

/// A tick and a word under the slider's track at [fraction] of its length.
class _Marker extends StatelessWidget {
  const _Marker({required this.fraction, required this.label});

  final double fraction;
  final String label;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final color = quietText(theme.colorScheme);
    return LayoutBuilder(
      builder: (context, box) {
        // Where the thumb sits for this value: the rounded track keeps half
        // its height clear at each end.
        final span = box.maxWidth - 2 * _sliderInset - _trackHeight;
        final x = _sliderInset + _trackHeight / 2 + fraction * span;
        return SizedBox(
          height: 30,
          child: Stack(
            clipBehavior: Clip.none,
            children: [
              Positioned(
                left: x - 1,
                top: 0,
                child: Container(width: 2, height: 6, color: color),
              ),
              Positioned(
                left: x,
                top: 8,
                child: FractionalTranslation(
                  translation: const Offset(-0.5, 0),
                  child: Text(
                    label,
                    maxLines: 1,
                    style: theme.textTheme.bodySmall?.copyWith(color: color),
                  ),
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}
