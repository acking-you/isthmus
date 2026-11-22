import 'dart:io' show Platform;

import 'package:flutter/services.dart' show FilteringTextInputFormatter;
import 'package:material_ui/material_ui.dart';
import 'package:isthmus_ui/l10n/app_localizations.dart';
import 'package:isthmus_ui/src/common/l10n_extension.dart';
import 'package:isthmus_ui/src/ffi/isthmus_api.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:isthmus_ui/src/shell/page_frame.dart';
import 'package:isthmus_ui/src/state/link_store.dart';
import 'package:isthmus_ui/src/theme/app_icons.dart';
import 'package:isthmus_ui/src/theme/app_theme.dart';
import 'package:isthmus_ui/src/theme/metrics.dart';
import 'package:isthmus_ui/src/updates/update_installers.dart'
    show openInBrowser;
import 'package:isthmus_ui/src/widgets/setting_rows.dart';
import 'package:isthmus_ui/src/widgets/status.dart';
import 'package:isthmus_ui/src/widgets/surfaces.dart';

/// Other Isthmus computers and the files between them: this device, a
/// pairing in progress, the files waiting to be sent, paired devices with a
/// Send button each, devices nearby to pair, and the transfers.
///
/// Files arrive in [LinkStore.staged] by being dropped anywhere on the
/// window (the app's drop target), so the page itself never needs a picker:
/// drop, then click Send beside the device that should get them.
class DevicesPage extends StatelessWidget {
  const DevicesPage({super.key});

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final link = AppScope.of(context).store.link;
    return PageFrame(
      title: l10n.navDevices,
      actions: [
        TextButton.icon(
          key: const Key('devices-open-received'),
          onPressed: () => link.reveal(),
          icon: const Icon(AppIcons.folderOpen, size: AppIconSize.button),
          label: Text(l10n.devicesOpenReceived),
        ),
      ],
      body: ListenableBuilder(
        listenable: link,
        builder: (context, _) {
          final status = link.status;
          final error = link.error;
          if (status == null) {
            return PageList(
              listKey: const Key('devices-list'),
              children: [
                if (error != null)
                  NoticeCard(
                    key: const Key('devices-unavailable'),
                    color: cautionColor(Theme.of(context).colorScheme),
                    icon: AppIcons.warning,
                    title: l10n.linkUnavailableTitle,
                    body: error.code == ErrorCode.anotherInstance
                        ? l10n.linkAnotherInstance
                        : error.message,
                  )
                else
                  const Padding(
                    padding: EdgeInsets.all(32),
                    child: Center(child: CircularProgressIndicator()),
                  ),
              ],
            );
          }
          final paired = status.paired;
          final nearby = status.nearby;
          final finished = status.transfers.any((t) => t.state.finished);
          final pairing = status.pairing;
          return PageList(
            listKey: const Key('devices-list'),
            children: [
              _MeCard(link: link, status: status),
              if (pairing != null) ...[
                const SizedBox(height: 16),
                PairingCard(link: link, pairing: pairing),
              ],
              const SizedBox(height: 16),
              _StagedCard(link: link),
              SectionHeader(
                title: l10n.devicesPairedSection,
                count: paired.length,
              ),
              if (paired.isEmpty)
                EmptyNote(icon: AppIcons.pair, text: l10n.devicesPairedNone)
              else
                for (final peer in paired) _PeerCard(link: link, peer: peer),
              const SizedBox(height: 12),
              _ClipboardCard(link: link, info: status.clipboard),
              SectionHeader(
                title: l10n.devicesNearbySection,
                count: nearby.length,
              ),
              if (nearby.isEmpty)
                EmptyNote(
                  icon: AppIcons.searching,
                  text: l10n.devicesNearbyNone,
                )
              else
                for (final peer in nearby) _PeerCard(link: link, peer: peer),
              if (status.transfers.isNotEmpty) ...[
                SectionHeader(
                  title: l10n.transfersSection,
                  count: status.transfers.length,
                  action: finished
                      ? TextButton(
                          key: const Key('transfers-clear'),
                          onPressed: link.clearTransfers,
                          child: Text(l10n.transfersClear),
                        )
                      : null,
                ),
                for (final transfer in status.transfers)
                  _TransferCard(link: link, transfer: transfer),
              ],
            ],
          );
        },
      ),
    );
  }
}

/// Sharing the clipboard with paired devices: the switch, what last
/// crossed, and on a Mac what macOS needs to let Isthmus read it.
class _ClipboardCard extends StatelessWidget {
  const _ClipboardCard({required this.link, required this.info});

  final LinkStore link;
  final ClipboardInfo info;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final state = link.clipboard;
    final last = info.last;
    final needsMac =
        info.enabled &&
        state.supported &&
        state.access != ClipboardAccess.allowed;
    return GroupCard(
      key: const Key('devices-clipboard'),
      title: l10n.clipboardTitle,
      icon: AppIcons.clipboard,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SwitchRow(
            switchKey: const Key('clipboard-switch'),
            title: l10n.clipboardSwitch,
            subtitle: l10n.clipboardHint,
            value: info.enabled,
            onChanged: link.setClipboardSharing,
          ),
          if (info.enabled && last != null) ...[
            const SizedBox(height: 6),
            _Quiet(
              last.direction == TransferDirection.receive
                  ? l10n.clipboardLastReceived(
                      last.peers.join(', '),
                      last.preview,
                    )
                  : l10n.clipboardLastSent(last.peers.join(', '), last.preview),
              maxLines: 2,
            ),
          ],
          if (needsMac) ...[
            const SizedBox(height: 10),
            Text(
              state.access == ClipboardAccess.denied
                  ? l10n.clipboardMacDenied
                  : l10n.clipboardMacAsk,
              key: const Key('clipboard-access-note'),
              style: theme.textTheme.bodyMedium?.copyWith(
                color: cautionColor(theme.colorScheme),
              ),
            ),
            const SizedBox(height: 8),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                if (state.access == ClipboardAccess.ask)
                  FilledButton.tonal(
                    key: const Key('clipboard-allow'),
                    onPressed: link.requestClipboardAccess,
                    child: Text(l10n.clipboardAllow),
                  ),
                OutlinedButton.icon(
                  key: const Key('clipboard-settings'),
                  onPressed: () => openInBrowser(
                    Uri.parse(
                      'x-apple.systempreferences:com.apple.preference.security?Privacy',
                    ),
                  ),
                  icon: const Icon(AppIcons.external, size: AppIconSize.inline),
                  label: Text(l10n.clipboardOpenSettings),
                ),
              ],
            ),
          ],
        ],
      ),
    );
  }
}

/// A path kind's name.
String pathLabel(AppLocalizations l10n, PathKind kind) => switch (kind) {
  PathKind.direct => l10n.pathDirect,
  PathKind.lan => l10n.pathLan,
};

IconData _pathIcon(PathKind kind) => switch (kind) {
  PathKind.direct => AppIcons.cable,
  PathKind.lan => AppIcons.network,
};

IconData _osIcon(PeerOs os) => switch (os) {
  PeerOs.macos => AppIcons.mac,
  PeerOs.windows || PeerOs.linux || PeerOs.other => AppIcons.pc,
};

/// A byte count for a person: 1.2 MB, 340 KB.
String formatBytes(int bytes) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  var value = bytes.toDouble();
  var unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  final digits = unit == 0 || value >= 100 ? 0 : 1;
  return '${value.toStringAsFixed(digits)} ${units[unit]}';
}

/// The last component of a path, either separator.
String baseName(String path) {
  final parts = path.split(RegExp(r'[\\/]')).where((p) => p.isNotEmpty);
  return parts.isEmpty ? path : parts.last;
}

/// A quiet line of help.
class _Quiet extends StatelessWidget {
  const _Quiet(this.text, {this.maxLines});

  final String text;
  final int? maxLines;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Text(
      text,
      maxLines: maxLines,
      overflow: maxLines == null ? null : TextOverflow.ellipsis,
      style: theme.textTheme.bodySmall?.copyWith(
        color: quietText(theme.colorScheme),
      ),
    );
  }
}

/// This device: its name (renameable), where others reach it, and where
/// received files go.
class _MeCard extends StatelessWidget {
  const _MeCard({required this.link, required this.status});

  final LinkStore link;
  final LinkStatus status;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final me = status.me;
    return GroupCard(
      key: const Key('devices-me'),
      title: l10n.devicesThisDevice,
      icon: _osIcon(me.os),
      trailing: TextButton.icon(
        key: const Key('devices-rename'),
        onPressed: () => _rename(context),
        icon: const Icon(AppIcons.rename, size: AppIconSize.inline),
        label: Text(l10n.devicesRename),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(me.name, style: theme.textTheme.titleMedium),
          const SizedBox(height: 8),
          Wrap(
            spacing: 8,
            runSpacing: 6,
            children: [
              if (me.paths.isEmpty) _Quiet(l10n.devicesNoAddress),
              for (final path in me.paths)
                _PathChip(
                  icon: _pathIcon(path.kind),
                  label:
                      '${pathLabel(l10n, path.kind)} · ${path.interface} · '
                      '${path.address}',
                ),
            ],
          ),
          const SizedBox(height: 10),
          _Quiet(l10n.devicesReceiveTo(status.receiveDir), maxLines: 2),
        ],
      ),
    );
  }

  Future<void> _rename(BuildContext context) async {
    final l10n = context.l10n;
    final controller = TextEditingController(text: status.me.name);
    final name = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(l10n.devicesRenameTitle),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            TextField(
              key: const Key('devices-rename-field'),
              controller: controller,
              autofocus: true,
              maxLength: 63,
              onSubmitted: (value) => Navigator.of(context).pop(value),
            ),
            _Quiet(l10n.devicesRenameHint),
          ],
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(),
            child: Text(l10n.cancel),
          ),
          FilledButton(
            key: const Key('devices-rename-save'),
            onPressed: () => Navigator.of(context).pop(controller.text),
            child: Text(l10n.devicesSave),
          ),
        ],
      ),
    );
    controller.dispose();
    if (name != null && name.trim().isNotEmpty && name != status.me.name) {
      await link.rename(name.trim());
    }
  }
}

/// A small pill: an icon and a line.
class _PathChip extends StatelessWidget {
  const _PathChip({required this.icon, required this.label});

  final IconData icon;
  final String label;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 4),
      decoration: BoxDecoration(
        color: softWash(scheme, strength: 1.2),
        borderRadius: BorderRadius.circular(999),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icon, size: 14, color: scheme.onSurfaceVariant),
          const SizedBox(width: 6),
          Text(
            label,
            style: theme.textTheme.labelSmall?.copyWith(
              color: scheme.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }
}

/// A pairing in progress or just finished, from either side: the code to
/// type here, the code to show, or how it ended.
class PairingCard extends StatefulWidget {
  const PairingCard({super.key, required this.link, required this.pairing});

  final LinkStore link;
  final PairingInfo pairing;

  @override
  State<PairingCard> createState() => _PairingCardState();
}

class _PairingCardState extends State<PairingCard> {
  final _code = TextEditingController();

  @override
  void dispose() {
    _code.dispose();
    super.dispose();
  }

  void _submit() {
    final code = _code.text.replaceAll(RegExp(r'\D'), '');
    if (code.length == 6) widget.link.enterCode(code);
  }

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final pairing = widget.pairing;
    final name = pairing.peerName;
    final cancel = TextButton(
      key: const Key('pairing-cancel'),
      onPressed: widget.link.cancelPairing,
      child: Text(
        pairing.role == PairRole.incoming ? l10n.pairDecline : l10n.cancel,
      ),
    );
    final (title, body, actions) = switch ((pairing.role, pairing.stage)) {
      (_, PairStage.connecting) => (l10n.pairConnecting(name), null, [cancel]),
      (PairRole.outgoing, PairStage.code) => (
        l10n.pairEnterCode(name),
        l10n.pairEnterCodeHint(name),
        [
          cancel,
          FilledButton(
            key: const Key('pairing-submit'),
            onPressed: _submit,
            child: Text(l10n.pairConfirm),
          ),
        ],
      ),
      (PairRole.incoming, PairStage.code) => (
        l10n.pairIncomingTitle(name),
        l10n.pairIncomingBody(name),
        [cancel],
      ),
      (PairRole.incoming, PairStage.confirm) => (
        l10n.pairIncomingTitle(name),
        l10n.pairAcceptBody(name),
        [
          cancel,
          FilledButton(
            key: const Key('pairing-accept'),
            onPressed: widget.link.acceptPairing,
            child: Text(l10n.pairAccept),
          ),
        ],
      ),
      (PairRole.outgoing, PairStage.confirm) => (
        l10n.pairWaitingTitle(name),
        l10n.pairWaitingBody(name),
        [cancel],
      ),
      (_, PairStage.verifying) => (l10n.pairVerifying, null, [cancel]),
      (_, PairStage.paired) => (
        l10n.pairPaired(name),
        l10n.pairPairedBody,
        [
          FilledButton(
            key: const Key('pairing-done'),
            onPressed: widget.link.cancelPairing,
            child: Text(l10n.pairDone),
          ),
        ],
      ),
      (_, PairStage.failed) => (
        l10n.pairFailedTitle(name),
        _failure(l10n, pairing.failure),
        [
          TextButton(
            key: const Key('pairing-close'),
            onPressed: widget.link.cancelPairing,
            child: Text(l10n.pairDone),
          ),
          if (pairing.role == PairRole.outgoing)
            FilledButton(
              key: const Key('pairing-retry'),
              onPressed: () => widget.link.pair(pairing.peerId),
              child: Text(l10n.pairRetry),
            ),
        ],
      ),
    };
    final color = switch (pairing.stage) {
      PairStage.paired => successColor(scheme),
      PairStage.failed => cautionColor(scheme),
      _ => signalColor(scheme),
    };
    final code = pairing.code;
    return GroupCard(
      key: const Key('devices-pairing'),
      title: title,
      icon: AppIcons.pair,
      trailing: StatusDot(color: color),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (code != null &&
              (pairing.stage == PairStage.confirm ||
                  (pairing.role == PairRole.incoming &&
                      pairing.stage == PairStage.code))) ...[
            Center(
              child: SelectableText(
                '${code.substring(0, 3)} ${code.substring(3)}',
                key: const Key('pairing-code'),
                style: theme.textTheme.displaySmall?.copyWith(
                  fontFamily: 'GeistMono',
                  fontWeight: FontWeight.w600,
                  letterSpacing: 4,
                ),
              ),
            ),
            const SizedBox(height: 10),
          ],
          if (body != null) _Quiet(body),
          if (pairing.role == PairRole.outgoing &&
              pairing.stage == PairStage.code) ...[
            const SizedBox(height: 12),
            TextField(
              key: const Key('pairing-code-field'),
              controller: _code,
              autofocus: true,
              keyboardType: TextInputType.number,
              inputFormatters: [
                FilteringTextInputFormatter.allow(RegExp(r'[0-9 ]')),
              ],
              maxLength: 7,
              style: theme.textTheme.titleLarge?.copyWith(
                fontFamily: 'GeistMono',
                letterSpacing: 3,
              ),
              decoration: InputDecoration(labelText: l10n.pairCodeField),
              onSubmitted: (_) => _submit(),
            ),
          ],
          if (pairing.stage == PairStage.connecting ||
              pairing.stage == PairStage.verifying) ...[
            const SizedBox(height: 10),
            const LinearProgressIndicator(),
          ],
          const SizedBox(height: 12),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              for (final (index, action) in actions.indexed) ...[
                if (index > 0) const SizedBox(width: 8),
                action,
              ],
            ],
          ),
        ],
      ),
    );
  }

  static String _failure(AppLocalizations l10n, PairFailure? failure) =>
      switch (failure) {
        PairFailure.wrongCode => l10n.pairFailWrongCode,
        PairFailure.declined => l10n.pairFailDeclined,
        PairFailure.unreachable => l10n.pairFailUnreachable,
        PairFailure.busy => l10n.pairFailBusy,
        PairFailure.canceled => l10n.pairFailCanceled,
        PairFailure.timedOut => l10n.pairFailTimedOut,
        PairFailure.mismatch => l10n.pairFailMismatch,
        PairFailure.lost || PairFailure.other || null => l10n.pairFailLost,
      };
}

/// The files dropped on the window, waiting for a device.
class _StagedCard extends StatelessWidget {
  const _StagedCard({required this.link});

  final LinkStore link;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final staged = link.staged;
    return GroupCard(
      key: const Key('devices-staged'),
      title: l10n.devicesDropTitle,
      icon: AppIcons.drop,
      trailing: staged.isEmpty
          ? null
          : TextButton(
              key: const Key('devices-staged-clear'),
              onPressed: link.clearStaged,
              child: Text(l10n.devicesClearStaged),
            ),
      child: staged.isEmpty
          ? _Quiet(l10n.devicesDropHint)
          : Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                for (final path in staged)
                  Padding(
                    padding: const EdgeInsets.only(bottom: 4),
                    child: Row(
                      children: [
                        Icon(
                          AppIcons.file,
                          size: AppIconSize.inline,
                          color: scheme.onSurfaceVariant,
                        ),
                        const SizedBox(width: 8),
                        Expanded(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Text(
                                baseName(path),
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: theme.textTheme.bodyMedium,
                              ),
                              _Quiet(path, maxLines: 1),
                            ],
                          ),
                        ),
                        IconButton(
                          tooltip: l10n.devicesRemoveStaged,
                          onPressed: () => link.unstage(path),
                          icon: const Icon(
                            AppIcons.close,
                            size: AppIconSize.inline,
                          ),
                        ),
                      ],
                    ),
                  ),
              ],
            ),
    );
  }
}

/// One device: paired ones with their path and a Send button, nearby ones
/// with Pair.
class _PeerCard extends StatelessWidget {
  const _PeerCard({required this.link, required this.peer});

  final LinkStore link;
  final PeerEntry peer;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final connection = peer.connection;
    final (Color dot, String line) = peer.paired
        ? connection != null
              ? (
                  successColor(scheme),
                  l10n.peerConnectedVia(
                    [
                      pathLabel(l10n, connection.kind),
                      ?connection.interface,
                    ].join(' · '),
                  ),
                )
              : (scheme.outline, l10n.peerOffline)
        : (
            signalColor(scheme),
            l10n.peerSeenVia(
              peer.routes.map((kind) => pathLabel(l10n, kind)).join(' / '),
            ),
          );
    final staged = link.staged.length;
    final Widget action;
    if (!peer.paired) {
      action = OutlinedButton.icon(
        key: Key('peer-pair-${peer.id}'),
        onPressed: link.status?.pairing?.stage.finished == false
            ? null
            : () => link.pair(peer.id),
        icon: const Icon(AppIcons.pair, size: AppIconSize.inline),
        label: Text(l10n.peerPair),
      );
    } else {
      final button = FilledButton.icon(
        key: Key('peer-send-${peer.id}'),
        onPressed: peer.online && staged > 0
            ? () => link.sendStaged(peer.id)
            : null,
        icon: const Icon(AppIcons.send, size: AppIconSize.inline),
        label: Text(l10n.peerSend(staged)),
      );
      action = Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          staged == 0
              ? Tooltip(message: l10n.peerSendNeedsFiles, child: button)
              : button,
          PopupMenuButton<String>(
            key: Key('peer-menu-${peer.id}'),
            icon: const Icon(AppIcons.more, size: AppIconSize.button),
            onSelected: (_) => _unpair(context),
            itemBuilder: (context) => [
              PopupMenuItem(
                key: Key('peer-unpair-${peer.id}'),
                value: 'unpair',
                child: Row(
                  children: [
                    const Icon(AppIcons.unpair, size: AppIconSize.button),
                    const SizedBox(width: 10),
                    Text(l10n.peerUnpair),
                  ],
                ),
              ),
            ],
          ),
        ],
      );
    }
    return ListCard(
      key: Key('peer-${peer.id}'),
      padding: const EdgeInsets.fromLTRB(16, 12, 8, 12),
      child: Row(
        children: [
          Icon(
            _osIcon(peer.os),
            size: AppIconSize.tile,
            color: scheme.onSurfaceVariant,
          ),
          const SizedBox(width: 14),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  peer.name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: theme.textTheme.titleSmall,
                ),
                const SizedBox(height: 3),
                StatusChip(color: dot, label: line, filled: false),
              ],
            ),
          ),
          const SizedBox(width: 8),
          action,
        ],
      ),
    );
  }

  Future<void> _unpair(BuildContext context) async {
    final l10n = context.l10n;
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(l10n.peerUnpairTitle(peer.name)),
        content: Text(l10n.peerUnpairBody),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: Text(l10n.cancel),
          ),
          FilledButton(
            key: const Key('peer-unpair-confirm'),
            onPressed: () => Navigator.of(context).pop(true),
            child: Text(l10n.peerUnpair),
          ),
        ],
      ),
    );
    if (confirmed == true) await link.unpair(peer.id);
  }
}

/// One transfer: which way, with whom, how far, how fast, over which path.
class _TransferCard extends StatelessWidget {
  const _TransferCard({required this.link, required this.transfer});

  final LinkStore link;
  final TransferEntry transfer;

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final sending = transfer.direction == TransferDirection.send;
    final who = sending
        ? l10n.transferTo(transfer.peerName)
        : l10n.transferFrom(transfer.peerName);
    final size =
        '${l10n.transferFiles(transfer.files)} · '
        '${formatBytes(transfer.totalBytes)}';
    final path = transfer.path;
    final where = path == null
        ? null
        : [pathLabel(l10n, path), ?transfer.interface].join(' ');
    final retry = transfer.retry;
    final line = switch (transfer.state) {
      TransferState.connecting => l10n.transferConnecting,
      TransferState.running => [
        l10n.transferProgress(
          formatBytes(transfer.doneBytes),
          formatBytes(transfer.totalBytes),
        ),
        if (transfer.bytesPerSecond > 0)
          l10n.transferSpeed(formatBytes(transfer.bytesPerSecond)),
        ?where,
      ].join(' · '),
      TransferState.retrying when retry == null => l10n.transferRetrying,
      TransferState.retrying =>
        sending
            ? l10n.transferRetryingSend(retry!.attempt, _reason(l10n, retry))
            : l10n.transferRetryingReceive(
                transfer.peerName,
                _reason(l10n, retry!),
              ),
      TransferState.done => [
        l10n.transferDone,
        size,
        if (path != null) pathLabel(l10n, path),
      ].join(' · '),
      TransferState.canceled => l10n.transferCanceled,
      TransferState.failed => [
        _failure(l10n, transfer.failure),
        if (retry != null) _reason(l10n, retry),
      ].join(': '),
    };
    // The system's own words, for whoever needs to know why.
    final detail =
        transfer.state == TransferState.retrying ||
            transfer.state == TransferState.failed
        ? retry?.detail
        : null;
    final color = switch (transfer.state) {
      TransferState.done => successColor(scheme),
      TransferState.failed => scheme.error,
      TransferState.canceled => scheme.outline,
      TransferState.retrying => cautionColor(scheme),
      TransferState.connecting || TransferState.running => signalColor(scheme),
    };
    final active = !transfer.state.finished;
    final canReveal =
        transfer.direction == TransferDirection.receive &&
        transfer.state == TransferState.done;
    return ListCard(
      key: Key('transfer-${transfer.id}'),
      padding: const EdgeInsets.fromLTRB(16, 12, 8, 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Icon(
                sending ? AppIcons.outgoing : AppIcons.incoming,
                size: AppIconSize.button,
                color: scheme.onSurfaceVariant,
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      transfer.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: theme.textTheme.titleSmall,
                    ),
                    _Quiet('$who · $size', maxLines: 1),
                  ],
                ),
              ),
              if (canReveal)
                TextButton(
                  key: Key('transfer-reveal-${transfer.id}'),
                  onPressed: () => link.reveal(transfer.id),
                  child: Text(
                    Platform.isWindows
                        ? l10n.transferRevealExplorer
                        : l10n.transferRevealFinder,
                  ),
                ),
              if (active)
                IconButton(
                  key: Key('transfer-cancel-${transfer.id}'),
                  tooltip: l10n.cancel,
                  onPressed: () => link.cancelTransfer(transfer.id),
                  icon: const Icon(AppIcons.close, size: AppIconSize.inline),
                ),
            ],
          ),
          if (active) ...[
            const SizedBox(height: 8),
            Padding(
              padding: const EdgeInsets.only(right: 8),
              child: ClipRRect(
                borderRadius: BorderRadius.circular(kControlRadius),
                // Retrying holds the real fraction: the transfer carries on
                // from there, nothing is lost.
                child: LinearProgressIndicator(
                  value: transfer.state == TransferState.connecting
                      ? null
                      : transfer.progress,
                  minHeight: 5,
                ),
              ),
            ),
          ],
          const SizedBox(height: 6),
          StatusChip(color: color, label: line, filled: false),
          if (detail != null) ...[
            const SizedBox(height: 4),
            SelectableText(
              detail,
              key: Key('transfer-detail-${transfer.id}'),
              maxLines: 2,
              style: theme.textTheme.bodySmall?.copyWith(
                color: scheme.onSurfaceVariant,
              ),
            ),
          ],
        ],
      ),
    );
  }

  /// What went wrong, in words: the path and its interface where it
  /// matters.
  static String _reason(AppLocalizations l10n, TransferRetry retry) {
    final path = retry.path;
    final where = path == null
        ? l10n.retryTheConnection
        : [pathLabel(l10n, path), ?retry.interface].join(' ');
    return switch (retry.reason) {
      RetryReason.stalled => l10n.retryStalled(where),
      RetryReason.lost => l10n.retryLost(where),
      RetryReason.unreachable => l10n.retryUnreachable,
      RetryReason.busy => l10n.retryBusy,
      RetryReason.other => l10n.transferRetrying,
    };
  }

  static String _failure(AppLocalizations l10n, TransferFailure? failure) =>
      switch (failure) {
        TransferFailure.notPaired => l10n.transferFailNotPaired,
        TransferFailure.unreachable => l10n.transferFailUnreachable,
        TransferFailure.lost => l10n.transferFailLost,
        TransferFailure.disk => l10n.transferFailDisk,
        TransferFailure.source => l10n.transferFailSource,
        TransferFailure.refused => l10n.transferFailRefused,
        TransferFailure.canceledByPeer => l10n.transferFailCanceledByPeer,
        TransferFailure.other || null => l10n.transferFailOther,
      };
}
