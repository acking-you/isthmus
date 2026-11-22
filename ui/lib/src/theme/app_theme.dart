import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/src/theme/app_icons.dart';
import 'package:isthmus_ui/src/theme/fonts.dart';
import 'package:isthmus_ui/src/theme/glass.dart';
import 'package:isthmus_ui/src/theme/metrics.dart';
import 'package:isthmus_ui/src/theme/motion.dart';

/// Calm graphite neutrals, ink controls and one blue "signal".
///
/// The design follows POCOX, as pb-mapper does: the neutrals carry no blue or
/// violet cast (a tint is what the eye keeps adapting to), the page is a soft
/// off-white rather than pure white, and controls are drawn in ink so colour
/// stays free for meaning. Green is working, amber is waiting on something,
/// red is failed, grey is off; the blue marks only what is live right now,
/// such as the app in front.
///
/// `onSurfaceVariant` stays above 4.5:1 for small secondary text on every
/// ground; `outline` is a border colour and must not carry text.
ColorScheme appColorScheme(Brightness brightness) {
  final light = brightness == Brightness.light;
  const ink = Color(0xFF1D1D1F);
  const paper = Color(0xFFE6E6E6);
  return ColorScheme.fromSeed(
    seedColor: const Color(0xFF1976D2),
    brightness: brightness,
  ).copyWith(
    primary: light ? ink : paper,
    onPrimary: light ? Colors.white : const Color(0xFF19191B),
    primaryContainer: light ? const Color(0xFFE6E6E4) : const Color(0xFF333336),
    onPrimaryContainer: light ? ink : paper,
    secondary: light ? const Color(0xFF4A4A4D) : const Color(0xFFBDBDC0),
    onSecondary: light ? Colors.white : const Color(0xFF19191B),
    secondaryContainer: light
        ? const Color(0xFFE6E6E4)
        : const Color(0xFF333336),
    onSecondaryContainer: light ? ink : paper,
    tertiary: light ? const Color(0xFF1B6FC8) : const Color(0xFF7DB6F2),
    onTertiary: light ? Colors.white : const Color(0xFF0B2A4D),
    tertiaryContainer: light
        ? const Color(0xFFE5EFFA)
        : const Color(0xFF1C2A3B),
    onTertiaryContainer: light
        ? const Color(0xFF124A8C)
        : const Color(0xFFD2E5FA),
    surface: light ? const Color(0xFFFBFBFA) : const Color(0xFF19191B),
    surfaceBright: light ? Colors.white : const Color(0xFF27272A),
    surfaceDim: light ? const Color(0xFFEDEDEB) : const Color(0xFF141416),
    onSurface: light ? ink : paper,
    onSurfaceVariant: light ? const Color(0xFF4A4A4D) : const Color(0xFFBDBDC0),
    surfaceContainerLowest: light ? Colors.white : const Color(0xFF141416),
    surfaceContainerLow: light
        ? const Color(0xFFF4F4F2)
        : const Color(0xFF202022),
    surfaceContainer: light ? const Color(0xFFEDEDEB) : const Color(0xFF262628),
    surfaceContainerHigh: light
        ? const Color(0xFFE6E6E4)
        : const Color(0xFF2E2E31),
    surfaceContainerHighest: light
        ? const Color(0xFFDEDEDC)
        : const Color(0xFF38383B),
    outline: light ? const Color(0xFFC8C8C6) : const Color(0xFF4C4C50),
    outlineVariant: light ? const Color(0xFFE4E4E2) : const Color(0xFF313134),
    inverseSurface: light ? const Color(0xFF2B2B2E) : const Color(0xFFE6E6E6),
    onInverseSurface: light ? const Color(0xFFF2F2F2) : ink,
    inversePrimary: light ? const Color(0xFFA9CBF2) : const Color(0xFF1B6FC8),
    // No elevation tint: a raised surface is told apart by tone and shadow.
    surfaceTint: Colors.transparent,
    error: light ? const Color(0xFFB3261E) : const Color(0xFFF2B8B5),
    onError: light ? Colors.white : const Color(0xFF601410),
    errorContainer: light ? const Color(0xFFFCE8E6) : const Color(0xFF3F1F1D),
    onErrorContainer: light ? const Color(0xFF8C1D18) : const Color(0xFFF9DEDC),
  );
}

/// The logo's blues as a gradient, for the few moments that are about the app
/// itself: the brand tile in the rail. Never a control's fill.
LinearGradient brandGradient(ColorScheme scheme) => LinearGradient(
  begin: Alignment.topLeft,
  end: Alignment.bottomRight,
  colors: scheme.brightness == Brightness.light
      ? const [Color(0xFF1976D2), Color(0xFF42A5F5)]
      : const [Color(0xFF2B7FD6), Color(0xFF5AB0F2)],
);

/// The one signal: what is live right now, such as the app in front.
Color signalColor(ColorScheme scheme) => scheme.tertiary;

/// Working: an event tap running, a device carrying Isthmus's mapping.
Color successColor(ColorScheme scheme) => scheme.brightness == Brightness.light
    ? const Color(0xFF1A7F37)
    : const Color(0xFF57C27A);

/// Waiting on something: a permission, Secure Input, another instance.
Color cautionColor(ColorScheme scheme) => scheme.brightness == Brightness.light
    ? const Color(0xFF9A5B00)
    : const Color(0xFFE0A647);

/// Ink for quiet glyphs: placeholders, idle icons. Text that must be read
/// uses `onSurfaceVariant`.
Color onSurfaceMuted(ColorScheme scheme) =>
    scheme.onSurface.withValues(alpha: 0.55);

/// Text a step quieter than `onSurfaceVariant`: help under a control, a
/// footnote. Still above 4.5:1 on cards.
Color quietText(ColorScheme scheme) => scheme.brightness == Brightness.light
    ? const Color(0xFF6E6E73)
    : const Color(0xFF8E8E93);

/// A faint neutral wash: a highlighted option, a chip that is "on" without
/// being a status.
Color softWash(ColorScheme scheme, {double strength = 1}) =>
    scheme.onSurface.withValues(
      alpha: (scheme.brightness == Brightness.light ? 0.06 : 0.09) * strength,
    );

/// The well of a text field: a step darker than both the page and cards.
Color fieldFill(ColorScheme scheme) => scheme.brightness == Brightness.light
    ? const Color(0xFFEAEAE8)
    : const Color(0xFF29292C);

/// The ground of what floats over the page: popovers, menus, dialogs.
Color floatingSurface(ColorScheme scheme) =>
    scheme.brightness == Brightness.light
    ? Colors.white
    : scheme.surfaceContainerHigh;

/// The hairline that outlines floating surfaces.
Color floatingEdge(ColorScheme scheme) => scheme.brightness == Brightness.light
    ? Colors.black.withValues(alpha: 0.05)
    : Colors.white.withValues(alpha: 0.07);

/// The window's own ground behind the rail and the content sheet.
Color backdropBase(ColorScheme scheme) => scheme.brightness == Brightness.light
    ? const Color(0xFFEAEAE8)
    : const Color(0xFF0F0F10);

/// Soft light on [backdropBase]: brighter at the top, never a hue.
List<({Alignment center, double radius, Color color})> backdropGlows(
  ColorScheme scheme,
) => scheme.brightness == Brightness.light
    ? const [
        (center: Alignment(-1.0, -1.15), radius: 1.2, color: Color(0x73FFFFFF)),
        (center: Alignment(1.1, -0.9), radius: 0.9, color: Color(0x40FFFFFF)),
      ]
    : const [
        (center: Alignment(-1.0, -1.15), radius: 1.2, color: Color(0x0FFFFFFF)),
      ];

/// The pointer's wash over a row on the backdrop: toward glass, not grey.
Color backdropHover(ColorScheme scheme) => scheme.brightness == Brightness.light
    ? Colors.white.withValues(alpha: 0.5)
    : Colors.white.withValues(alpha: 0.05);

/// The content sheet: white for a page you read, grouped grey for a page of
/// cards so the white cards lift off it.
Color sheetColor(ColorScheme scheme, {bool grouped = false}) =>
    scheme.brightness == Brightness.light
    ? (grouped ? const Color(0xFFF4F4F2) : scheme.surface)
    : (grouped ? const Color(0xFF151517) : scheme.surface);

/// The hairline that keeps the sheet's edge crisp against the backdrop.
Color sheetEdge(ColorScheme scheme) => scheme.brightness == Brightness.light
    ? Colors.black.withValues(alpha: 0.06)
    : Colors.white.withValues(alpha: 0.07);

/// The sheet's lift off the backdrop.
List<BoxShadow> sheetShadows(ColorScheme scheme) =>
    scheme.brightness == Brightness.light
    ? [
        BoxShadow(
          color: Colors.black.withValues(alpha: 0.05),
          blurRadius: 2,
          offset: const Offset(0, 1),
        ),
        BoxShadow(
          color: Colors.black.withValues(alpha: 0.07),
          blurRadius: 30,
          offset: const Offset(0, 10),
        ),
      ]
    : [
        BoxShadow(
          color: Colors.black.withValues(alpha: 0.5),
          blurRadius: 30,
          offset: const Offset(0, 12),
        ),
      ];

/// A white card on a grouped sheet.
Color raisedCardColor(ColorScheme scheme) =>
    scheme.brightness == Brightness.light
    ? Colors.white
    : const Color(0xFF212124);

/// The hairline around a [raisedCardColor] card.
Color raisedCardEdge(ColorScheme scheme) =>
    scheme.brightness == Brightness.light
    ? Colors.black.withValues(alpha: 0.05)
    : Colors.white.withValues(alpha: 0.06);

/// The soft lift under a [raisedCardColor] card.
List<BoxShadow> raisedCardShadows(ColorScheme scheme) =>
    scheme.brightness == Brightness.light
    ? [
        BoxShadow(
          color: Colors.black.withValues(alpha: 0.04),
          blurRadius: 2,
          offset: const Offset(0, 1),
        ),
        BoxShadow(
          color: Colors.black.withValues(alpha: 0.045),
          blurRadius: 16,
          offset: const Offset(0, 4),
        ),
      ]
    : [
        BoxShadow(
          color: Colors.black.withValues(alpha: 0.35),
          blurRadius: 12,
          offset: const Offset(0, 4),
        ),
      ];

/// The type scale on Material's roles: 11 (captions), 12 (secondary), 13
/// (controls and rows), 14 (reading) do nearly all the work, titles above.
/// Material's tracking is tuned for Roboto and spreads Google Sans Flex and
/// CJK apart, so every role carries its own.
TextTheme _textTheme(ColorScheme scheme) {
  TextStyle t(
    double size,
    double height, [
    FontWeight w = FontWeight.w400,
    double tracking = 0,
  ]) => TextStyle(
    fontSize: size,
    height: height,
    fontWeight: w,
    letterSpacing: tracking,
  );

  return TextTheme(
    displayLarge: t(48, 1.12, FontWeight.w400, -0.5),
    displayMedium: t(40, 1.15, FontWeight.w400, -0.4),
    displaySmall: t(34, 1.18, FontWeight.w400, -0.3),
    headlineLarge: t(30, 1.2, FontWeight.w600, -0.3),
    headlineMedium: t(26, 1.22, FontWeight.w600, -0.2),
    headlineSmall: t(22, 1.25, FontWeight.w600, -0.1),
    titleLarge: t(19, 1.3, FontWeight.w600, -0.1),
    titleMedium: t(16, 1.35, FontWeight.w600),
    titleSmall: t(13, 1.4, FontWeight.w600),
    bodyLarge: t(15, 1.6),
    bodyMedium: t(14, 1.5),
    bodySmall: t(12, 1.45, FontWeight.w400, 0.1),
  ).apply(
    fontFamily: uiFontFamily,
    fontFamilyFallback: uiCjkFallback,
    bodyColor: scheme.onSurface,
    displayColor: scheme.onSurface,
  );
}

/// The shared Material 3 theme for [scheme]: every button a pill, every icon
/// button a circle, panels generously rounded and lifted by tone rather than
/// outline, tuned for a pointer (hover instead of ripples, a pointing hand
/// over anything clickable).
ThemeData _base(ColorScheme scheme) {
  final light = scheme.brightness == Brightness.light;
  TextStyle ui(
    double size, {
    FontWeight weight = FontWeight.w500,
    Color? color,
  }) => TextStyle(
    fontSize: size,
    fontWeight: weight,
    color: color,
    letterSpacing: 0.1,
    fontFamily: uiFontFamily,
    fontFamilyFallback: uiCjkFallback,
  );
  const pill = StadiumBorder();
  final floatShadow = Colors.black.withValues(alpha: light ? 0.14 : 0.5);
  final menuShape = RoundedRectangleBorder(
    borderRadius: BorderRadius.circular(kMenuRadius),
    side: BorderSide(color: floatingEdge(scheme)),
  );
  final fieldRadius = BorderRadius.circular(kFieldRadius);
  const density = EdgeInsets.symmetric(horizontal: 16);
  return ThemeData(
    colorScheme: scheme,
    useMaterial3: true,
    fontFamily: uiFontFamily,
    fontFamilyFallback: uiCjkFallback,
    textTheme: _textTheme(scheme),
    scaffoldBackgroundColor: scheme.surface,
    canvasColor: scheme.surface,
    pageTransitionsTheme: appPageTransitions,
    visualDensity: VisualDensity.compact,
    splashFactory: NoSplash.splashFactory,
    hoverColor: scheme.onSurface.withValues(alpha: 0.05),
    highlightColor: scheme.onSurface.withValues(alpha: 0.08),
    focusColor: scheme.onSurface.withValues(alpha: 0.08),
    dividerTheme: DividerThemeData(
      color: scheme.outlineVariant,
      thickness: 1,
      space: 1,
    ),
    // Buttons are lit capsules: ink for the filled action, a pane of chrome
    // glass for the secondary one, each with a sheen ([LitFace]) and a soft
    // shadow that rises on hover and settles when pressed.
    filledButtonTheme: FilledButtonThemeData(
      style:
          FilledButton.styleFrom(
            minimumSize: const Size(36, 38),
            padding: density,
            shape: pill,
            textStyle: ui(13.5, weight: FontWeight.w600),
          ).copyWith(
            mouseCursor: clickable,
            elevation: WidgetStateProperty.resolveWith(
              (states) =>
                  states.contains(WidgetState.disabled) ||
                      states.contains(WidgetState.pressed)
                  ? 0
                  : states.contains(WidgetState.hovered)
                  ? 3
                  : 1,
            ),
            shadowColor: WidgetStatePropertyAll(
              light ? Colors.black.withValues(alpha: 0.38) : Colors.black,
            ),
            backgroundBuilder: (context, states, child) =>
                LitFace(states: states, child: child),
          ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style:
          OutlinedButton.styleFrom(
            minimumSize: const Size(36, 38),
            padding: density,
            foregroundColor: scheme.onSurface,
            shape: pill,
            textStyle: ui(13.5),
          ).copyWith(
            mouseCursor: clickable,
            backgroundColor: WidgetStateProperty.resolveWith((states) {
              if (states.contains(WidgetState.disabled)) {
                return Colors.transparent;
              }
              final strong =
                  states.contains(WidgetState.hovered) ||
                  states.contains(WidgetState.pressed);
              return light
                  ? Colors.white.withValues(alpha: strong ? 0.95 : 0.7)
                  : Colors.white.withValues(alpha: strong ? 0.12 : 0.07);
            }),
            side: WidgetStateProperty.resolveWith(
              (states) => BorderSide(
                color: states.contains(WidgetState.disabled)
                    ? scheme.outlineVariant
                    : light
                    ? Colors.black.withValues(alpha: 0.09)
                    : Colors.white.withValues(alpha: 0.12),
              ),
            ),
            elevation: WidgetStateProperty.resolveWith(
              (states) =>
                  states.contains(WidgetState.hovered) &&
                      !states.contains(WidgetState.pressed)
                  ? 2
                  : 0,
            ),
            shadowColor: WidgetStatePropertyAll(
              light ? Colors.black.withValues(alpha: 0.22) : Colors.black,
            ),
            overlayColor: WidgetStateProperty.resolveWith(
              (states) => states.contains(WidgetState.pressed)
                  ? scheme.onSurface.withValues(alpha: 0.06)
                  : Colors.transparent,
            ),
            backgroundBuilder: (context, states, child) =>
                LitFace(states: states, glass: true, child: child),
          ),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(
        minimumSize: const Size(36, 36),
        padding: const EdgeInsets.symmetric(horizontal: 12),
        foregroundColor: scheme.onSurface,
        shape: pill,
        textStyle: ui(13.5),
      ).copyWith(mouseCursor: clickable),
    ),
    iconButtonTheme: IconButtonThemeData(
      style: IconButton.styleFrom(
        minimumSize: const Size(34, 34),
        iconSize: AppIconSize.button,
        foregroundColor: scheme.onSurfaceVariant,
        shape: const CircleBorder(),
      ).copyWith(mouseCursor: clickable),
    ),
    // Fields are soft filled wells with no outline at rest; focus draws a
    // firm ink ring with a faint signal glow.
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      isDense: true,
      fillColor: fieldFill(scheme),
      hoverColor: scheme.onSurface.withValues(alpha: 0.03),
      border: FieldBorder(borderRadius: fieldRadius),
      enabledBorder: FieldBorder(borderRadius: fieldRadius),
      disabledBorder: FieldBorder(borderRadius: fieldRadius),
      focusedBorder: FieldBorder(
        borderRadius: fieldRadius,
        borderSide: BorderSide(
          color: scheme.onSurface.withValues(alpha: light ? 0.5 : 0.55),
          width: 1.5,
        ),
        glow: scheme.tertiary.withValues(alpha: light ? 0.16 : 0.28),
      ),
      contentPadding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
      hintStyle: TextStyle(color: onSurfaceMuted(scheme), fontSize: 14),
      labelStyle: TextStyle(color: scheme.onSurfaceVariant),
    ),
    textSelectionTheme: TextSelectionThemeData(
      cursorColor: scheme.onSurface,
      selectionColor: scheme.tertiary.withValues(alpha: light ? 0.22 : 0.35),
      selectionHandleColor: scheme.tertiary,
    ),
    progressIndicatorTheme: ProgressIndicatorThemeData(
      color: scheme.onSurfaceVariant,
      linearTrackColor: scheme.surfaceContainerHighest,
      circularTrackColor: Colors.transparent,
    ),
    // Controls are ink: an "on" switch is not a status, so it is not green.
    switchTheme: SwitchThemeData(
      mouseCursor: clickable,
      thumbColor: WidgetStateProperty.resolveWith(
        (states) => states.contains(WidgetState.selected)
            ? scheme.onPrimary
            : scheme.outline,
      ),
      trackColor: WidgetStateProperty.resolveWith(
        (states) => states.contains(WidgetState.selected)
            ? scheme.primary
            : scheme.surfaceContainerHighest,
      ),
      trackOutlineColor: const WidgetStatePropertyAll(Colors.transparent),
    ),
    sliderTheme: SliderThemeData(
      activeTrackColor: scheme.primary,
      inactiveTrackColor: scheme.surfaceContainerHighest,
      thumbColor: scheme.primary,
      overlayColor: scheme.primary.withValues(alpha: 0.08),
      activeTickMarkColor: scheme.onPrimary.withValues(alpha: 0.5),
      inactiveTickMarkColor: scheme.onSurface.withValues(alpha: 0.25),
      valueIndicatorColor: scheme.inverseSurface,
      valueIndicatorTextStyle: ui(12, color: scheme.onInverseSurface),
      mouseCursor: clickable,
    ),
    listTileTheme: ListTileThemeData(
      mouseCursor: clickable,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(kControlRadius),
      ),
      iconColor: scheme.onSurfaceVariant,
      titleTextStyle: ui(14, weight: FontWeight.w400, color: scheme.onSurface),
      subtitleTextStyle: ui(
        12.5,
        weight: FontWeight.w400,
        color: scheme.onSurfaceVariant,
      ),
      minLeadingWidth: 20,
    ),
    dialogTheme: DialogThemeData(
      backgroundColor: light
          ? Colors.white.withValues(alpha: 0.96)
          : const Color(0xFF252528).withValues(alpha: 0.97),
      surfaceTintColor: Colors.transparent,
      elevation: 24,
      shadowColor: floatShadow,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(kDialogRadius),
        side: BorderSide(
          color: light ? Colors.white : Colors.white.withValues(alpha: 0.1),
        ),
      ),
      titleTextStyle: ui(18, weight: FontWeight.w600, color: scheme.onSurface),
      contentTextStyle: ui(
        14,
        weight: FontWeight.w400,
        color: scheme.onSurfaceVariant,
      ).copyWith(height: 1.55),
      actionsPadding: const EdgeInsets.fromLTRB(20, 4, 20, 20),
      insetPadding: const EdgeInsets.symmetric(horizontal: 24, vertical: 24),
    ),
    popupMenuTheme: PopupMenuThemeData(
      elevation: 10,
      shadowColor: floatShadow,
      color: floatingSurface(scheme),
      surfaceTintColor: Colors.transparent,
      shape: menuShape,
      menuPadding: const EdgeInsets.all(6),
      textStyle: ui(13.5, weight: FontWeight.w400, color: scheme.onSurface),
    ),
    menuTheme: MenuThemeData(
      style: MenuStyle(
        elevation: const WidgetStatePropertyAll(10),
        shadowColor: WidgetStatePropertyAll(floatShadow),
        surfaceTintColor: const WidgetStatePropertyAll(Colors.transparent),
        backgroundColor: WidgetStatePropertyAll(floatingSurface(scheme)),
        shape: WidgetStatePropertyAll(menuShape),
        padding: const WidgetStatePropertyAll(EdgeInsets.all(6)),
      ),
    ),
    tooltipTheme: TooltipThemeData(
      waitDuration: const Duration(milliseconds: 450),
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      decoration: BoxDecoration(
        color: scheme.inverseSurface,
        borderRadius: BorderRadius.circular(8),
      ),
      textStyle: ui(12, color: scheme.onInverseSurface),
    ),
    scrollbarTheme: ScrollbarThemeData(
      thickness: WidgetStateProperty.resolveWith(
        (states) => states.contains(WidgetState.hovered) ? 8.0 : 6.0,
      ),
      radius: const Radius.circular(4),
      crossAxisMargin: 2,
      mainAxisMargin: 2,
    ),
    cardTheme: CardThemeData(
      elevation: 0,
      color: raisedCardColor(scheme),
      surfaceTintColor: Colors.transparent,
      margin: EdgeInsets.zero,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(kCardRadius),
        side: BorderSide(color: raisedCardEdge(scheme)),
      ),
    ),
    iconTheme: IconThemeData(color: scheme.onSurfaceVariant, size: 20),
  );
}

/// The theme for [brightness]. Isthmus is a Mac app with a pointer, so there
/// is one density: pb-mapper's desktop one.
ThemeData buildAppTheme(Brightness brightness) =>
    _base(appColorScheme(brightness));
