import 'package:flutter/foundation.dart';
import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/src/theme/metrics.dart';
import 'package:window_manager/window_manager.dart';

/// The native title bar is hidden (see `main.dart`), so the app draws one
/// continuous surface: the rail sits on the backdrop up to the window's edge,
/// and the strip above the page and the top of each page drag the window. A
/// Mac keeps its traffic lights in the rail's corner; Windows gets its
/// minimize, maximize and close buttons drawn here ([CaptionButtons]), in
/// the system's own glyphs, at the top right.
///
/// Gated on [defaultTargetPlatform] rather than `dart:io`, so `flutter test`
/// (forced to Android) never talks to a window that does not exist.
bool get isFramelessDesktop =>
    !kIsWeb &&
    (defaultTargetPlatform == TargetPlatform.macOS ||
        defaultTargetPlatform == TargetPlatform.windows);

/// The window as the app opens it, shared with the Windows integration
/// test so it runs the same window.
WindowOptions get isthmusWindowOptions => WindowOptions(
  size: const Size(980, 680),
  minimumSize: const Size(760, 560),
  center: true,
  backgroundColor: Colors.transparent,
  title: 'Isthmus',
  titleBarStyle: isFramelessDesktop
      ? TitleBarStyle.hidden
      : TitleBarStyle.normal,
  windowButtonVisibility: true,
);

/// Whether the app draws the window's caption buttons: Windows.
bool get hasCaptionButtons =>
    !kIsWeb && defaultTargetPlatform == TargetPlatform.windows;

/// Room the traffic lights take at the top of the rail.
double get trafficLightHeight =>
    !kIsWeb && defaultTargetPlatform == TargetPlatform.macOS ? 46 : 18;

/// The strip above the page: Windows' caption buttons need their height,
/// elsewhere the sheet's own inset is enough.
double get captionHeight => hasCaptionButtons ? kCaptionHeight : kSheetInset;

/// The height of Windows' caption buttons, as Windows 11 draws them.
const double kCaptionHeight = 32;

/// Empty chrome that drags the window, and zooms it on a double click as the
/// platform's own title bar would.
class WindowDragArea extends StatelessWidget {
  const WindowDragArea({super.key, this.child});

  final Widget? child;

  @override
  Widget build(BuildContext context) {
    final content = child ?? const SizedBox.expand();
    if (!isFramelessDesktop) return content;
    return DragToMoveArea(
      child: GestureDetector(
        behavior: HitTestBehavior.translucent,
        onDoubleTap: _toggleMaximize,
        child: content,
      ),
    );
  }
}

Future<void> _toggleMaximize() async {
  try {
    if (await windowManager.isMaximized()) {
      await windowManager.unmaximize();
    } else {
      await windowManager.maximize();
    }
  } catch (_) {
    // Best effort: a window that refuses stays as it is.
  }
}

/// Windows' minimize, maximize (or restore) and close buttons, drawn in the
/// app's surface with the glyphs Windows draws its own in (Segoe Fluent
/// Icons on Windows 11, Segoe MDL2 Assets on 10). Close closes the window,
/// which Isthmus answers by going to the notification area, as with the
/// native button.
class CaptionButtons extends StatefulWidget {
  const CaptionButtons({super.key});

  @override
  State<CaptionButtons> createState() => _CaptionButtonsState();
}

class _CaptionButtonsState extends State<CaptionButtons> with WindowListener {
  bool _maximized = false;

  @override
  void initState() {
    super.initState();
    windowManager.addListener(this);
    windowManager
        .isMaximized()
        .then((maximized) {
          if (mounted) setState(() => _maximized = maximized);
        })
        .catchError((Object _) {});
  }

  @override
  void dispose() {
    windowManager.removeListener(this);
    super.dispose();
  }

  @override
  void onWindowMaximize() => setState(() => _maximized = true);

  @override
  void onWindowUnmaximize() => setState(() => _maximized = false);

  @override
  Widget build(BuildContext context) {
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        _CaptionButton(
          key: const Key('caption-minimize'),
          glyph: '\uE921',
          onPressed: () => windowManager.minimize(),
        ),
        _CaptionButton(
          key: const Key('caption-maximize'),
          glyph: _maximized ? '\uE923' : '\uE922',
          onPressed: _toggleMaximize,
        ),
        _CaptionButton(
          key: const Key('caption-close'),
          glyph: '\uE8BB',
          close: true,
          onPressed: () => windowManager.close(),
        ),
      ],
    );
  }
}

/// One caption button: 46 by 32, a faint wash on hover, Windows' red for
/// close.
class _CaptionButton extends StatefulWidget {
  const _CaptionButton({
    super.key,
    required this.glyph,
    required this.onPressed,
    this.close = false,
  });

  final String glyph;
  final VoidCallback onPressed;
  final bool close;

  @override
  State<_CaptionButton> createState() => _CaptionButtonState();
}

class _CaptionButtonState extends State<_CaptionButton> {
  bool _hover = false;
  bool _down = false;

  /// Windows' own close-button red, and its pressed shade.
  static const _closeRed = Color(0xFFC42B1C);
  static const _closeRedDown = Color(0xFFC74F43);

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final background = switch ((widget.close, _hover, _down)) {
      (true, _, true) => _closeRedDown,
      (true, true, false) => _closeRed,
      (false, _, true) => scheme.onSurface.withValues(alpha: 0.12),
      (false, true, false) => scheme.onSurface.withValues(alpha: 0.07),
      (_, false, false) => Colors.transparent,
    };
    final foreground = widget.close && (_hover || _down)
        ? Colors.white
        : scheme.onSurface;
    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = _down = false),
      child: GestureDetector(
        onTapDown: (_) => setState(() => _down = true),
        onTapCancel: () => setState(() => _down = false),
        onTap: () {
          setState(() => _down = false);
          widget.onPressed();
        },
        child: Container(
          width: 46,
          height: kCaptionHeight,
          color: background,
          alignment: Alignment.center,
          child: Text(
            widget.glyph,
            style: TextStyle(
              fontFamily: 'Segoe Fluent Icons',
              fontFamilyFallback: const ['Segoe MDL2 Assets'],
              fontSize: 10,
              color: foreground,
            ),
          ),
        ),
      ),
    );
  }
}
