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

/// The app icon's art: the brand tile with the glyph, drawn from the same
/// geometry as [BrandMark] so the icon, the rail and the poster never drift.
///
/// [margin] is the share of the canvas left around the tile on each side:
/// macOS icons sit in a transparent margin with a soft shadow (Apple's grid:
/// an 824-point tile on a 1024-point canvas), while Windows and tray icons
/// fill their canvas. `test/brand_render_test.dart` rasterizes it into every
/// icon file.
class BrandIconPainter extends CustomPainter {
  const BrandIconPainter({this.margin = 0, this.shadow = false});

  /// The empty border on each side, as a share of the canvas.
  final double margin;

  /// Whether to drop the soft shadow macOS icons carry under the tile.
  final bool shadow;

  /// The light and the dark end of the brand gradient on the icon.
  static const Color light = Color(0xFF42A5F5);
  static const Color dark = Color(0xFF1565C0);

  @override
  void paint(Canvas canvas, Size size) {
    final side = size.shortestSide;
    final inset = side * margin;
    final tile = Rect.fromLTWH(
      inset,
      inset,
      side - 2 * inset,
      side - 2 * inset,
    );
    final radius = Radius.circular(tile.width * 0.225);
    final shape = RSuperellipse.fromRectAndRadius(tile, radius);
    if (shadow) {
      canvas.drawRSuperellipse(
        RSuperellipse.fromRectAndRadius(
          tile.translate(0, tile.height * 0.012),
          radius,
        ),
        Paint()
          ..color = const Color(0x40000000)
          ..maskFilter = MaskFilter.blur(BlurStyle.normal, tile.width * 0.014),
      );
    }
    canvas.drawRSuperellipse(
      shape,
      Paint()
        ..shader = const LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: [light, Color(0xFF1E88E5), dark],
          stops: [0, 0.5, 1],
        ).createShader(tile),
    );
    // A soft light from above, as on a lit tile.
    canvas.drawRSuperellipse(
      shape,
      Paint()
        ..shader = LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.center,
          colors: [
            Colors.white.withValues(alpha: 0.18),
            Colors.white.withValues(alpha: 0),
          ],
        ).createShader(tile),
    );
    final glyphSide = tile.width * 0.66;
    final glyph = isthmusGlyph(glyphSide).shift(
      Offset(
        tile.left + (tile.width - glyphSide) / 2,
        tile.top + (tile.height - glyphSide) / 2,
      ),
    );
    canvas
      ..drawPath(
        glyph.shift(Offset(0, tile.height * 0.012)),
        Paint()
          ..color = const Color(0x3D0D47A1)
          ..maskFilter = MaskFilter.blur(BlurStyle.normal, tile.width * 0.012),
      )
      ..drawPath(glyph, Paint()..color = Colors.white);
  }

  @override
  bool shouldRepaint(BrandIconPainter old) =>
      old.margin != margin || old.shadow != shadow;
}
