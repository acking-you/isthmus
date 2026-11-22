import 'package:flutter/painting.dart';

/// The typefaces, after POCOX's design system: Google Sans Flex for prose and
/// Geist Mono for keys, both bundled (SIL OFL, see `assets/fonts/OFL-*.txt`).
/// Neither carries a single CJK glyph, so Chinese always resolves down
/// [uiCjkFallback]. Isthmus runs on macOS only, where PingFang SC has every
/// weight the theme asks for, so no CJK face is bundled.

/// The bundled prose face. Latin only.
const String uiFontFamily = 'GoogleSansFlex';

/// The bundled key face. Latin only.
const String monoFontFamily = 'GeistMono';

/// The CJK chain behind [uiFontFamily]: PingFang SC ships with macOS in six
/// real weights, so a Medium line never renders half Regular.
const List<String> uiCjkFallback = ['PingFang SC', 'Hiragino Sans GB'];

/// Fallback behind [monoFontFamily]: macOS's own monospaced faces, then the
/// CJK chain, since a bundle id or an app name can carry Chinese.
const List<String> monoFallback = ['SF Mono', 'Menlo', ...uiCjkFallback];

/// A monospaced style for key labels and bundle identifiers.
TextStyle mono({
  double? fontSize,
  FontWeight? fontWeight,
  Color? color,
  double? height,
}) => TextStyle(
  fontFamily: monoFontFamily,
  fontFamilyFallback: monoFallback,
  fontSize: fontSize,
  fontWeight: fontWeight,
  color: color,
  height: height,
);
