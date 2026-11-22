import 'package:flutter/foundation.dart';
import 'package:flutter/painting.dart';

/// The typefaces, after POCOX and pb-mapper: Google Sans Flex for prose and
/// Geist Mono for keys, both bundled (SIL OFL, see `assets/fonts/OFL-*.txt`),
/// with Segoe UI for prose on Windows. None of them carries a CJK glyph, so
/// Chinese always resolves down [uiCjkFallback].
///
/// Gated on [defaultTargetPlatform] rather than `dart:io`, so `flutter test`
/// (forced to Android) takes one branch on every host.

/// The bundled prose face. Latin only.
const String appFontFamily = 'GoogleSansFlex';

/// The bundled key face. Latin only.
const String monoFontFamily = 'GeistMono';

/// The bundled Noto Sans SC subset (see `assets/fonts/README.md`).
const String bundledCjkFamily = 'NotoSansSC';

bool get _windows => !kIsWeb && defaultTargetPlatform == TargetPlatform.windows;

/// The UI face: Google Sans Flex everywhere but Windows.
///
/// Flutter draws text with grayscale antialiasing rather than ClearType, and
/// at 13 to 15 px on a 100 to 125 % display a bundled face's joins smear.
/// Segoe UI is hinted for exactly those sizes and ships with every Windows.
String get uiFontFamily => _windows ? 'Segoe UI' : appFontFamily;

/// The CJK chain behind [uiFontFamily].
///
/// macOS leads with PingFang SC, which has every weight the theme asks for.
/// Windows leads with the bundled subset: Microsoft YaHei has no 500 or
/// 600, so a Medium line would render half Regular, half Bold-ish. A
/// character outside the subset still finds YaHei.
List<String> get uiCjkFallback => _windows
    ? const [
        bundledCjkFamily,
        'Microsoft YaHei UI',
        'Microsoft YaHei',
        // Always present: a stripped install without Segoe UI still lands
        // Latin on a real face instead of tofu.
        appFontFamily,
      ]
    : const ['PingFang SC', 'Hiragino Sans GB', bundledCjkFamily];

/// Fallback behind [monoFontFamily]: the system's own monospaced faces,
/// then the CJK chain, since a bundle id or an app name can carry Chinese.
List<String> get monoFallback => [
  'SF Mono',
  'Menlo',
  'Cascadia Mono',
  'Consolas',
  ...uiCjkFallback,
];

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
