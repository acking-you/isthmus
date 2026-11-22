import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:isthmus_ui/src/shell/navigation.dart';
import 'package:isthmus_ui/src/shell/window_chrome.dart';
import 'package:isthmus_ui/src/theme/app_theme.dart';
import 'package:isthmus_ui/src/theme/glass.dart';
import 'package:isthmus_ui/src/theme/metrics.dart';

/// The app's frame, in the layers of POCOX's design: the rail sits straight
/// on the lit backdrop and the page on a rounded sheet beside it. The window
/// never gets narrower than 760, so there is no compact layout.
class AppShell extends StatelessWidget {
  const AppShell({
    super.key,
    required this.page,
    required this.onSelect,
    required this.child,
  });

  final AppPage page;
  final ValueChanged<AppPage> onSelect;

  /// The page itself, built by the caller.
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Scaffold(
      backgroundColor: backdropBase(scheme),
      body: AppBackdrop(
        child: Stack(
          children: [
            Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                SizedBox(
                  width: kRailWidth,
                  child: NavRail(page: page, onSelect: onSelect),
                ),
                Expanded(
                  child: Padding(
                    padding: const EdgeInsets.fromLTRB(
                      0,
                      kSheetInset,
                      kSheetInset,
                      kSheetInset,
                    ),
                    // Every page is a page of cards, on the grouped sheet
                    // that white cards lift off.
                    child: ContentSheet(
                      grouped: true,
                      // Each page settles onto its sheet as it arrives, so a
                      // switch reads as a change of page rather than a flash.
                      child: EntranceMotion(
                        key: ValueKey(page),
                        offset: 8,
                        child: child,
                      ),
                    ),
                  ),
                ),
              ],
            ),
            // The strip above the sheet is window chrome too: it drags.
            if (isFramelessDesktop)
              const Positioned(
                top: 0,
                left: kRailWidth,
                right: 0,
                height: kSheetInset,
                child: WindowDragArea(),
              ),
          ],
        ),
      ),
    );
  }
}
