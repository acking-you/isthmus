import 'package:desktop_drop/desktop_drop.dart';
import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/src/common/l10n_extension.dart';
import 'package:isthmus_ui/src/theme/app_icons.dart';
import 'package:isthmus_ui/src/theme/glass.dart';
import 'package:isthmus_ui/src/theme/metrics.dart';

/// The whole window as a drop target for files and folders: while a drag
/// hovers it shows where they will go, and a drop hands their paths to
/// [onDrop]. Only items with a path on disk count; a dragged web link or a
/// file promised by an app that never writes it is skipped.
///
/// The plugin attaches to the native window, so tests, which have none,
/// pass `enabled: false` and get the child alone.
class FileDropZone extends StatefulWidget {
  const FileDropZone({
    super.key,
    required this.onDrop,
    required this.child,
    this.enabled = true,
  });

  final void Function(List<String> paths) onDrop;
  final Widget child;
  final bool enabled;

  @override
  State<FileDropZone> createState() => _FileDropZoneState();
}

class _FileDropZoneState extends State<FileDropZone> {
  bool _hovering = false;

  void _hover(bool on) {
    if (mounted && _hovering != on) setState(() => _hovering = on);
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.enabled) return widget.child;
    return DropTarget(
      acceptedKinds: const {DropItemKind.file, DropItemKind.directory},
      onEnter: (_) => _hover(true),
      onLeave: () => _hover(false),
      onDrop: (details) {
        _hover(false);
        final paths = [
          for (final item in details.items)
            ?switch (item) {
              DropFileItem(:final localPath) => localPath,
              DropDirectoryItem(:final localPath) => localPath,
              DropStringItem() => null,
            },
        ];
        if (paths.isNotEmpty) widget.onDrop(paths);
      },
      child: Stack(
        children: [
          widget.child,
          if (_hovering)
            Positioned.fill(
              child: IgnorePointer(child: _DropOverlay(key: const Key('drop'))),
            ),
        ],
      ),
    );
  }
}

/// What a hovering drag sees: a pane of glass over the window saying the
/// files will wait to be sent.
class _DropOverlay extends StatelessWidget {
  const _DropOverlay({super.key});

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.all(14),
      child: Glass(
        depth: GlassDepth.overlay,
        borderRadius: BorderRadius.circular(kDialogRadius),
        child: Center(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                AppIcons.drop,
                size: 40,
                color: theme.colorScheme.onSurfaceVariant,
              ),
              const SizedBox(height: 12),
              Text(
                context.l10n.devicesDropRelease,
                style: theme.textTheme.titleMedium,
              ),
            ],
          ),
        ),
      ),
    );
  }
}
