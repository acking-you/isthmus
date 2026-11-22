import 'package:flutter/foundation.dart';
import 'package:material_ui/material_ui.dart';
import 'package:window_manager/window_manager.dart';

/// The native title bar is hidden (see `main.dart`), so the app draws one
/// continuous surface: the rail sits on the backdrop up to the window's edge,
/// the traffic lights stay in its corner, and the top of each page drags the
/// window.
///
/// Gated on [defaultTargetPlatform] rather than `dart:io`, so `flutter test`
/// (forced to Android) never talks to a window that does not exist.
bool get isFramelessDesktop =>
    !kIsWeb && defaultTargetPlatform == TargetPlatform.macOS;

/// Room the traffic lights take at the top of the rail.
double get trafficLightHeight => isFramelessDesktop ? 46 : 18;

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
