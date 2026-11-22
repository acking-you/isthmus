import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/src/shell/window_chrome.dart';

/// A page's title bar in the sheet beside the rail.
const double kPageBarHeight = 58;

/// A page: its title bar and its body. The bar is window chrome too: its
/// empty space drags the window.
class PageFrame extends StatelessWidget {
  const PageFrame({
    super.key,
    required this.title,
    required this.body,
    this.actions = const [],
  });

  final String title;
  final Widget body;

  /// Quiet actions at the bar's end.
  final List<Widget> actions;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SizedBox(
          height: kPageBarHeight,
          child: Stack(
            alignment: AlignmentDirectional.centerStart,
            children: [
              const Positioned.fill(child: WindowDragArea()),
              Padding(
                padding: const EdgeInsetsDirectional.only(start: 22, end: 12),
                child: Row(
                  children: [
                    Expanded(
                      child: IgnorePointer(
                        child: Text(
                          title,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: theme.textTheme.titleLarge,
                        ),
                      ),
                    ),
                    ...actions,
                  ],
                ),
              ),
            ],
          ),
        ),
        Expanded(child: body),
      ],
    );
  }
}
