import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/src/theme/app_theme.dart';
import 'package:isthmus_ui/src/theme/fonts.dart';

/// One key as it is printed on a keycap: "Ctrl", "Home", "⌘", "⇧".
///
/// Geist Mono, so a Windows word and a Mac symbol line up on one baseline,
/// on a small raised cap with a darker lower lip: the eye reads it as a key
/// before it reads the label.
class Keycap extends StatelessWidget {
  const Keycap(this.label, {super.key});

  final String label;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final light = scheme.brightness == Brightness.light;
    return Container(
      constraints: const BoxConstraints(minWidth: 24),
      padding: const EdgeInsets.fromLTRB(7, 2, 7, 3),
      decoration: BoxDecoration(
        color: light ? Colors.white : const Color(0xFF2C2C2F),
        borderRadius: BorderRadius.circular(6),
        border: Border.all(
          color: light
              ? Colors.black.withValues(alpha: 0.12)
              : Colors.white.withValues(alpha: 0.1),
        ),
        boxShadow: [
          BoxShadow(
            color: light
                ? Colors.black.withValues(alpha: 0.12)
                : Colors.black.withValues(alpha: 0.5),
            offset: const Offset(0, 1.5),
          ),
        ],
      ),
      child: Text(
        label,
        textAlign: TextAlign.center,
        maxLines: 1,
        style: mono(
          fontSize: 12,
          fontWeight: FontWeight.w500,
          color: scheme.onSurface,
          height: 1.3,
        ),
      ),
    );
  }
}

/// A chord of [keys]: Windows chords joined by a quiet "+", as Windows
/// writes them; Mac chords ([mac]) set side by side, as macOS menus do.
class KeyChord extends StatelessWidget {
  const KeyChord(this.keys, {super.key, this.mac = false});

  final List<String> keys;
  final bool mac;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Wrap(
      spacing: mac ? 3 : 0,
      runSpacing: 4,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        for (var i = 0; i < keys.length; i++) ...[
          if (i > 0 && !mac)
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 3),
              child: Text(
                '+',
                style: mono(fontSize: 11, color: quietText(scheme)),
              ),
            ),
          Keycap(keys[i]),
        ],
      ],
    );
  }
}
