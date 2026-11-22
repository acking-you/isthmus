// Draws the menu bar icon: the isthmus glyph (two shores joined by a narrow
// neck of land) as a macOS template image, black plus alpha, at 18 × 18 and
// 36 × 36 pixels. macOS tints a template to match the menu bar, so the art is
// shape only. The geometry is `isthmusGlyph` in
// lib/src/widgets/brand_mark.dart; the two change together.
//
// Run from ui/: dart run tool/gen_tray_icon.dart
import 'dart:io';
import 'dart:typed_data';

/// Writes `assets/tray/tray_icon.png` and `tray_icon@2x.png`.
void main() {
  for (final (scale, suffix) in [(1, ''), (2, '@2x')]) {
    final png = encodePng(18 * scale, 18 * scale, renderGlyph(scale));
    File('assets/tray/tray_icon$suffix.png').writeAsBytesSync(png);
  }
}

/// Whether the point (x, y), in the 18-unit box, is land.
bool inside(double x, double y) {
  bool disc(double cx, double cy, double r) =>
      (x - cx) * (x - cx) + (y - cy) * (y - cy) <= r * r;
  if (disc(4.6, 10.0, 3.8) || disc(13.6, 8.0, 3.4)) return true;
  if (x < 4.6 || x > 13.6) return false;
  // The neck's curves have evenly spaced control points in x, so x is linear
  // in t and each edge is a function of x.
  final t = (x - 4.6) / 9;
  double quad(double a, double b, double c) =>
      (1 - t) * (1 - t) * a + 2 * t * (1 - t) * b + t * t * c;
  final top = quad(7.4, 10.4, 5.4);
  final bottom = quad(12.6, 7.6, 10.6);
  return y >= top && y <= bottom;
}

/// The alpha of every pixel at [scale], 8 × 8 samples each for smooth edges.
Uint8List renderGlyph(int scale) {
  final size = 18 * scale;
  const samples = 8;
  final alpha = Uint8List(size * size);
  for (var py = 0; py < size; py++) {
    for (var px = 0; px < size; px++) {
      var hits = 0;
      for (var sy = 0; sy < samples; sy++) {
        for (var sx = 0; sx < samples; sx++) {
          final x = (px + (sx + 0.5) / samples) / scale;
          final y = (py + (sy + 0.5) / samples) / scale;
          if (inside(x, y)) hits++;
        }
      }
      alpha[py * size + px] = (hits * 255 / (samples * samples)).round();
    }
  }
  return alpha;
}

/// A minimal RGBA PNG: black everywhere, [alpha] per pixel.
Uint8List encodePng(int width, int height, Uint8List alpha) {
  final raw = BytesBuilder();
  for (var y = 0; y < height; y++) {
    raw.addByte(0); // Filter: none.
    for (var x = 0; x < width; x++) {
      raw.add([0, 0, 0, alpha[y * width + x]]);
    }
  }
  final header = ByteData(13)
    ..setUint32(0, width)
    ..setUint32(4, height)
    ..setUint8(8, 8) // Bit depth.
    ..setUint8(9, 6) // Colour type: RGBA.
    ..setUint8(10, 0)
    ..setUint8(11, 0)
    ..setUint8(12, 0);
  final out = BytesBuilder()
    ..add([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
    ..add(_chunk('IHDR', header.buffer.asUint8List()))
    ..add(_chunk('IDAT', Uint8List.fromList(zlib.encode(raw.toBytes()))))
    ..add(_chunk('IEND', Uint8List(0)));
  return out.toBytes();
}

Uint8List _chunk(String type, Uint8List data) {
  final typeBytes = type.codeUnits;
  final length = ByteData(4)..setUint32(0, data.length);
  final crc = ByteData(4)..setUint32(0, _crc32([...typeBytes, ...data]));
  return Uint8List.fromList([
    ...length.buffer.asUint8List(),
    ...typeBytes,
    ...data,
    ...crc.buffer.asUint8List(),
  ]);
}

int _crc32(List<int> bytes) {
  var crc = 0xFFFFFFFF;
  for (final byte in bytes) {
    crc ^= byte;
    for (var k = 0; k < 8; k++) {
      crc = (crc & 1) != 0 ? (crc >> 1) ^ 0xEDB88320 : crc >> 1;
    }
  }
  return crc ^ 0xFFFFFFFF;
}
