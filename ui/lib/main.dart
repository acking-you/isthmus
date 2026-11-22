import 'dart:async';
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart' show rootBundle;
import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/src/app.dart';
import 'package:isthmus_ui/src/ffi/isthmus_api.dart';
import 'package:isthmus_ui/src/ffi/isthmus_service.dart';
import 'package:window_manager/window_manager.dart';

/// The bundled faces are under the SIL Open Font License, which asks that the
/// notice travel with the font. Registering them puts the text in the standard
/// licence listing beside every package's.
void _registerFontLicenses() {
  for (final (families, file) in [
    (['GoogleSansFlex'], 'assets/fonts/OFL-GoogleSansFlex.txt'),
    (['GeistMono'], 'assets/fonts/OFL-GeistMono.txt'),
    (['NotoSansSC'], 'assets/fonts/OFL-NotoSansSC.txt'),
  ]) {
    LicenseRegistry.addLicense(() async* {
      yield LicenseEntryWithLineBreaks(
        families,
        await rootBundle.loadString(file),
      );
    });
  }
}

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  _registerFontLicenses();
  await windowManager.ensureInitialized();
  // The native title bar is hidden so the app draws one continuous surface:
  // a Mac keeps its traffic lights in the rail's corner, and Windows gets
  // its caption buttons drawn by the app (window_chrome.dart). On a Mac the
  // window starts hidden (MainFlutterWindow.swift) and is configured here
  // without being shown: the app decides whether a launch needs it.
  final windowReady = windowManager.waitUntilReadyToShow(
    WindowOptions(
      size: const Size(980, 680),
      minimumSize: const Size(760, 560),
      center: true,
      backgroundColor: Colors.transparent,
      title: 'Isthmus',
      titleBarStyle: Platform.isMacOS || Platform.isWindows
          ? TitleBarStyle.hidden
          : TitleBarStyle.normal,
      windowButtonVisibility: true,
    ),
  );
  try {
    IsthmusService().initChangeEvents();
  } catch (error) {
    // A library that will not load fails every call too; the overview says
    // so, and the window opens to show it.
    debugPrint('Isthmus change hints are unavailable: $error');
  }
  runApp(IsthmusApp(api: IsthmusApi(), windowReady: windowReady));
}
