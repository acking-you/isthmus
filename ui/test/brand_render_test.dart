/// Renders the brand into every file that carries it: the macOS app icon
/// set, the Windows executable and tray icons, the README logo and the
/// README posters (English and Chinese). Everything is drawn from
/// `BrandIconPainter` and the real app pages, so the files never drift from
/// the app. Opt-in, and it writes into the repository:
///
/// ```sh
/// ISTHMUS_RENDER_BRAND=1 \
/// ISTHMUS_UI_PREVIEW_CJK="/System/Library/Fonts/Supplemental/Arial Unicode.ttf" \
///   flutter test test/brand_render_test.dart
/// ```
library;

import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:isthmus_ui/src/state/link_store.dart';
import 'package:isthmus_ui/src/pages/devices_page.dart';
import 'package:isthmus_ui/src/theme/app_icons.dart';
import 'package:isthmus_ui/src/widgets/brand_mark.dart';
import 'package:material_ui/material_ui.dart';

import 'fake_isthmus_api.dart';
import 'support.dart';

final bool _render = Platform.environment['ISTHMUS_RENDER_BRAND'] == '1';
final String? _cjk = Platform.environment['ISTHMUS_UI_PREVIEW_CJK'];

/// Apple's grid: an 824-point tile on a 1024-point canvas.
const double _macMargin = 100 / 1024;

const Key _root = Key('brand-root');

Future<Uint8List> _png(WidgetTester tester, {double pixelRatio = 1}) async {
  final boundary = tester.renderObject<RenderRepaintBoundary>(
    find.byKey(_root),
  );
  late Uint8List bytes;
  await tester.runAsync(() async {
    final image = await boundary.toImage(pixelRatio: pixelRatio);
    final data = await image.toByteData(format: ui.ImageByteFormat.png);
    bytes = data!.buffer.asUint8List();
  });
  return bytes;
}

/// The icon art at [side] pixels.
Future<Uint8List> _icon(
  WidgetTester tester,
  int side, {
  double margin = 0,
  bool shadow = false,
}) async {
  tester.view.physicalSize = Size(side.toDouble() + 8, side.toDouble() + 8);
  tester.view.devicePixelRatio = 1;
  await tester.pumpWidget(
    Center(
      child: RepaintBoundary(
        key: _root,
        child: SizedBox.square(
          dimension: side.toDouble(),
          child: CustomPaint(
            painter: BrandIconPainter(margin: margin, shadow: shadow),
          ),
        ),
      ),
    ),
  );
  return _png(tester);
}

/// A Windows .ico holding [images] (side, PNG) as PNG entries, which every
/// Windows since Vista reads.
Uint8List _ico(List<(int, Uint8List)> images) {
  final header = BytesBuilder()
    ..add(_u16(0))
    ..add(_u16(1))
    ..add(_u16(images.length));
  var offset = 6 + 16 * images.length;
  final entries = BytesBuilder();
  for (final (side, png) in images) {
    entries
      ..addByte(side >= 256 ? 0 : side)
      ..addByte(side >= 256 ? 0 : side)
      ..addByte(0)
      ..addByte(0)
      ..add(_u16(1))
      ..add(_u16(32))
      ..add(_u32(png.length))
      ..add(_u32(offset));
    offset += png.length;
  }
  final body = BytesBuilder();
  for (final (_, png) in images) {
    body.add(png);
  }
  return (BytesBuilder()
        ..add(header.toBytes())
        ..add(entries.toBytes())
        ..add(body.toBytes()))
      .toBytes();
}

Uint8List _u16(int value) =>
    (ByteData(2)..setUint16(0, value, Endian.little)).buffer.asUint8List();

Uint8List _u32(int value) =>
    (ByteData(4)..setUint32(0, value, Endian.little)).buffer.asUint8List();

/// A happy Mac for the overview: everything on and granted.
FakeIsthmusApi _overviewApi() => FakeIsthmusApi(
  status: statusJson(enabled: true, keyboardsMapped: 2, miceTuned: 1),
  settings: const {'enabled': true},
  devices: [
    deviceJson(
      'keyboard:k8',
      'keyboard',
      name: 'Keychron K8',
      manufacturer: 'Keychron',
      hasModeSwitch: true,
    ),
    deviceJson(
      'keyboard:kvm',
      'keyboard',
      name: 'HKS201-M24',
      manufacturer: 'TESmart',
      viaKvm: true,
    ),
    deviceJson('mouse:mx', 'mouse', name: 'MX Master 3S'),
  ],
);

/// File sharing with a PC on the KVM cable: files waiting, one going out,
/// one just received.
FakeIsthmusApi _devicesApi() => FakeIsthmusApi(
  status: statusJson(enabled: true),
  settings: const {'enabled': true},
  link: linkJson(
    peers: [
      peerJson('0123456789abcdef', 'Desk PC'),
      peerJson(
        '2222333344445555',
        'MacBook Air',
        os: 'macos',
        online: false,
        routes: const [],
      ),
    ],
    transfers: [
      transferJson(
        't1',
        name: 'Holiday photos',
        total: 820000000,
        done: 512000000,
      ),
      transferJson(
        't2',
        direction: 'receive',
        state: 'done',
        name: 'Quarterly report.pdf',
        total: 4800000,
        done: 4800000,
        savedTo: '/Users/me/Downloads/Quarterly report.pdf',
      ),
    ],
  ),
);

/// A page of the app as an image, at twice the window's size.
Future<ui.Image> _page(
  WidgetTester tester,
  FakeIsthmusApi api,
  AppPage page,
  Locale locale, {
  void Function(LinkStore link)? prepare,
}) async {
  await pumpApp(tester, api, page: page, locale: locale);
  if (prepare != null) {
    final link = AppScope.of(tester.element(find.byType(DevicesPage)))
        .store
        .link;
    prepare(link);
    await settle(tester);
  }
  final boundary = tester.renderObject<RenderRepaintBoundary>(
    find.byKey(appRootKey),
  );
  late ui.Image image;
  await tester.runAsync(() async {
    image = await boundary.toImage(pixelRatio: 2);
  });
  await finish(tester);
  return image;
}

/// The words on the poster.
typedef _Copy = ({
  String tagline,
  String lead,
  List<(IconData, String)> points,
  String footer,
});

const _english = (
  tagline: 'Two systems, one pair of hands.',
  lead:
      'Windows habits on your Mac, and your files between the Mac and the PC.',
  points: [
    (
      AppIcons.keyboard,
      'Ctrl as Command, Home and End by line, Alt+Tab, and a Shift tap for 中/英',
    ),
    (
      AppIcons.mouse,
      'The Windows wheel and pointer, and scrolling with the middle button held',
    ),
    (
      AppIcons.devices,
      'Pair once with a code, drop files, send: over the KVM cable first, then Wi-Fi',
    ),
  ],
  footer: 'macOS 13+ · Windows 10+ · Rust + Flutter · MIT',
);

const _chinese = (
  tagline: '两套系统，一双手。',
  lead: '在 Mac 上延续 Windows 的手感，并在 Mac 与 PC 之间互传文件。',
  points: [
    (AppIcons.keyboard, 'Ctrl 当 Command、Home/End 按行、Alt+Tab，单按 Shift 切中英'),
    (AppIcons.mouse, 'Windows 的滚轮与指针手感，按住中键拖动即可滚动'),
    (AppIcons.devices, '配对一次，拖入即发：优先走 KVM 上的直连网线，其次 Wi-Fi'),
  ],
  footer: 'macOS 13+ · Windows 10+ · Rust + Flutter · MIT',
);

/// The poster: the mark, the name, the tagline and the three things Isthmus
/// does on the left; the overview and the devices page on the right.
class _Poster extends StatelessWidget {
  const _Poster({
    required this.copy,
    required this.overview,
    required this.devices,
  });

  final _Copy copy;
  final ui.Image overview;
  final ui.Image devices;

  static const ink = Color(0xFF0B1B33);
  static const quiet = Color(0xFF52627A);
  static const blue = Color(0xFF1565C0);

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: 1280,
      height: 720,
      child: DecoratedBox(
        decoration: const BoxDecoration(
          gradient: LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: [Color(0xFFF7FAFF), Color(0xFFE3EEFC)],
          ),
        ),
        child: Stack(
          clipBehavior: Clip.hardEdge,
          children: [
            // A wash of the brand blue behind the windows.
            Positioned(
              right: -160,
              top: 40,
              child: Container(
                width: 760,
                height: 760,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  gradient: RadialGradient(
                    colors: [
                      const Color(0xFF42A5F5).withValues(alpha: 0.30),
                      const Color(0xFF42A5F5).withValues(alpha: 0),
                    ],
                  ),
                ),
              ),
            ),
            Positioned(
              left: 628,
              top: 54,
              child: _Window(image: overview, width: 620),
            ),
            Positioned(
              left: 560,
              top: 318,
              child: _Window(image: devices, width: 556),
            ),
            Positioned(
              left: 72,
              top: 84,
              width: 430,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const SizedBox.square(
                    dimension: 92,
                    child: CustomPaint(
                      painter: BrandIconPainter(margin: 0.04, shadow: true),
                    ),
                  ),
                  const SizedBox(height: 22),
                  const Text(
                    'Isthmus',
                    style: TextStyle(
                      fontSize: 64,
                      fontWeight: FontWeight.w700,
                      letterSpacing: -1.2,
                      color: ink,
                      height: 1.0,
                    ),
                  ),
                  const SizedBox(height: 14),
                  Text(
                    copy.tagline,
                    style: const TextStyle(
                      fontSize: 26,
                      fontWeight: FontWeight.w600,
                      color: blue,
                      height: 1.2,
                    ),
                  ),
                  const SizedBox(height: 14),
                  Text(
                    copy.lead,
                    style: const TextStyle(
                      fontSize: 17,
                      color: quiet,
                      height: 1.45,
                    ),
                  ),
                  const SizedBox(height: 26),
                  for (final (icon, text) in copy.points)
                    Padding(
                      padding: const EdgeInsets.only(bottom: 14),
                      child: Row(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Container(
                            width: 34,
                            height: 34,
                            decoration: BoxDecoration(
                              color: const Color(0xFF1E88E5)
                                  .withValues(alpha: 0.12),
                              borderRadius: BorderRadius.circular(10),
                            ),
                            child: Icon(icon, size: 18, color: blue),
                          ),
                          const SizedBox(width: 14),
                          Expanded(
                            child: Padding(
                              padding: const EdgeInsets.only(top: 6),
                              child: Text(
                                text,
                                style: const TextStyle(
                                  fontSize: 15.5,
                                  color: ink,
                                  height: 1.35,
                                ),
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                ],
              ),
            ),
            Positioned(
              left: 76,
              bottom: 40,
              child: Text(
                copy.footer,
                style: const TextStyle(
                  fontSize: 13.5,
                  color: quiet,
                  letterSpacing: 0.2,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// A page of the app as a floating window: rounded, edged and shadowed.
class _Window extends StatelessWidget {
  const _Window({required this.image, required this.width});

  final ui.Image image;
  final double width;

  @override
  Widget build(BuildContext context) {
    final height = width * image.height / image.width;
    return Container(
      width: width,
      height: height,
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: const Color(0x1F0B1B33)),
        boxShadow: const [
          BoxShadow(
            color: Color(0x2E0B1B33),
            blurRadius: 48,
            offset: Offset(0, 24),
          ),
          BoxShadow(
            color: Color(0x140B1B33),
            blurRadius: 6,
            offset: Offset(0, 2),
          ),
        ],
      ),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(13),
        child: RawImage(image: image, fit: BoxFit.cover),
      ),
    );
  }
}

void main() {
  if (!_render) {
    test(
      'brand rendering is opt-in',
      () {},
      skip: 'set ISTHMUS_RENDER_BRAND=1',
    );
    return;
  }

  testWidgets('render the icons', (tester) async {
    addTearDown(tester.view.reset);
    // macOS: the sizes Contents.json names, in Apple's grid with a shadow.
    for (final side in [16, 32, 64, 128, 256, 512, 1024]) {
      final png = await _icon(
        tester,
        side,
        margin: _macMargin,
        shadow: side >= 64,
      );
      File('macos/Runner/Assets.xcassets/AppIcon.appiconset/app_icon_$side.png')
          .writeAsBytesSync(png);
    }
    // Windows: the executable's icon, nearly full bleed, every size the
    // shell asks for.
    final exe = <(int, Uint8List)>[
      for (final side in [16, 20, 24, 32, 40, 48, 64, 128, 256])
        (side, await _icon(tester, side, margin: 0.03)),
    ];
    File('windows/runner/resources/app_icon.ico').writeAsBytesSync(_ico(exe));
    // The Windows notification area: the tile filling its square.
    final tray = <(int, Uint8List)>[
      for (final side in [16, 20, 24, 32, 40, 48, 64])
        (side, await _icon(tester, side)),
    ];
    File('assets/tray/tray_icon.ico').writeAsBytesSync(_ico(tray));
    // The README's logo.
    File('../docs/assets/logo.png').writeAsBytesSync(
      await _icon(tester, 512, margin: _macMargin, shadow: true),
    );
  });

  for (final (locale, copy, suffix) in [
    (const Locale('en'), _english, ''),
    if (_cjk != null) (const Locale('zh'), _chinese, '.zh-CN'),
  ]) {
    testWidgets('render the poster ${locale.languageCode}', (tester) async {
      debugDisableShadows = false;
      try {
        await tester.runAsync(() => loadRealFonts(cjk: _cjk));
        final overview = await _page(
          tester,
          _overviewApi(),
          AppPage.overview,
          locale,
        );
        final devices = await _page(
          tester,
          _devicesApi(),
          AppPage.devices,
          locale,
          prepare: (link) => link.stage([
            '/Users/me/Documents/Design review.key',
            '/Users/me/Pictures/Trip',
          ]),
        );
        tester.view.physicalSize = const Size(1280, 720);
        tester.view.devicePixelRatio = 1;
        await tester.pumpWidget(
          Directionality(
            textDirection: TextDirection.ltr,
            child: DefaultTextStyle(
              style: const TextStyle(
                fontFamily: 'GoogleSansFlex',
                fontFamilyFallback: ['PingFang SC'],
              ),
              child: RepaintBoundary(
                key: _root,
                child: _Poster(
                  copy: copy,
                  overview: overview,
                  devices: devices,
                ),
              ),
            ),
          ),
        );
        await tester.pump();
        File('../docs/assets/poster$suffix.png')
            .writeAsBytesSync(await _png(tester, pixelRatio: 1.5));
        tester.view.reset();
      } finally {
        debugDisableShadows = true;
      }
    });
  }
}
