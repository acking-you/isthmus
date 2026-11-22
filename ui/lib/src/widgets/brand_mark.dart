import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/src/theme/app_theme.dart';

/// The isthmus glyph in an 18 × 18 box: two shores joined by a narrow neck
/// of land. `tool/gen_tray_icon.dart` rasterizes the same geometry into the
/// menu bar icon; the two change together.
Path isthmusGlyph(double size) {
  final s = size / 18;
  final shores = Path()
    ..addOval(Rect.fromCircle(center: Offset(4.6 * s, 10 * s), radius: 3.8 * s))
    ..addOval(
      Rect.fromCircle(center: Offset(13.6 * s, 8 * s), radius: 3.4 * s),
    );
  // The neck: two quadratic curves pinched toward the line between the
  // shores, so the land narrows in the middle and widens into each side.
  final neck = Path()
    ..moveTo(4.6 * s, 7.4 * s)
    ..quadraticBezierTo(9.1 * s, 10.4 * s, 13.6 * s, 5.4 * s)
    ..lineTo(13.6 * s, 10.6 * s)
    ..quadraticBezierTo(9.1 * s, 7.6 * s, 4.6 * s, 12.6 * s)
    ..close();
  return Path.combine(PathOperation.union, shores, neck);
}

/// The app's mark: the isthmus glyph in white on the logo's blue tile. The
/// one place the brand gradient appears, because it is about the app itself.
class BrandMark extends StatelessWidget {
  const BrandMark({super.key, this.size = 26});

  final double size;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        gradient: brandGradient(scheme),
        borderRadius: BorderRadius.circular(size * 0.28),
        boxShadow: [
          BoxShadow(
            color: const Color(0xFF1976D2).withValues(alpha: 0.25),
            blurRadius: 6,
            offset: const Offset(0, 2),
          ),
        ],
      ),
      child: CustomPaint(painter: _GlyphPainter()),
    );
  }
}

/// The glyph in white, centred in the tile with a margin.
class _GlyphPainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    final glyph = size.shortestSide * 0.78;
    final inset = (size.shortestSide - glyph) / 2;
    canvas
      ..save()
      ..translate(inset, inset)
      ..drawPath(isthmusGlyph(glyph), Paint()..color = Colors.white)
      ..restore();
  }

  @override
  bool shouldRepaint(_GlyphPainter old) => false;
}
