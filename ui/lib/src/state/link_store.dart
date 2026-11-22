import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:isthmus_ui/src/ffi/isthmus_api.dart';
import 'package:isthmus_ui/src/state/app_store.dart' show StoreAction;

/// File sharing as the Devices page and the app see it: Rust's snapshot,
/// the files waiting to be sent, and the calls that change either.
///
/// The snapshot is read on every `link` hint (Rust folds transfer progress
/// into a few hints a second) and when the window comes back. Reads never
/// overlap: a hint that arrives during a read asks for one more after it,
/// so a burst of progress costs at most two reads.
///
/// Files dropped on the window wait here, in the app, until a device is
/// chosen: Rust is told about them only when they are sent.
class LinkStore extends ChangeNotifier {
  LinkStore({required this.api, this.onFailure});

  final IsthmusApiClient api;

  /// Told about every failed call; the app turns it into a toast.
  void Function(StoreAction action, IsthmusCallException error)? onFailure;

  /// A transfer to this device just finished.
  void Function(TransferEntry transfer)? onReceived;

  /// Another device asked to pair: the code to show is up.
  void Function(PairingInfo pairing)? onPairingRequest;

  /// A copy crossed to or from a paired device.
  void Function(ClipboardEvent event)? onClipboard;

  /// What this computer's clipboard allows, from the last snapshot.
  ClipboardState get clipboard =>
      status?.clipboardAccess ?? ClipboardState.unknown;

  /// The last snapshot, or null before the first read lands.
  LinkStatus? status;

  /// Why the snapshot cannot be read (file sharing did not start), or null.
  IsthmusCallException? error;

  final List<String> _staged = [];
  bool _disposed = false;
  Future<void>? _reading;
  bool _readAgain = false;

  /// The files and folders waiting for a device, in the order dropped.
  List<String> get staged => List.unmodifiable(_staged);

  /// Read the snapshot now, or once more after the read in flight.
  Future<void> refresh() {
    if (_reading != null) {
      _readAgain = true;
      return _reading!;
    }
    return _reading = _read().whenComplete(() {
      _reading = null;
      if (_readAgain && !_disposed) {
        _readAgain = false;
        unawaited(refresh());
      }
    });
  }

  Future<void> _read() async {
    try {
      final next = await api.linkStatus();
      if (_disposed) return;
      _apply(next);
    } on IsthmusCallException catch (failure) {
      if (_disposed) return;
      if (error?.code != failure.code || error?.message != failure.message) {
        error = failure;
        _notify();
      }
    } catch (_) {
      // The library did not answer at all; the next hint tries again.
    }
  }

  void _apply(LinkStatus next) {
    final previous = status;
    final hadError = error != null;
    error = null;
    if (previous == next && !hadError) return;
    status = next;
    if (previous != null) _announce(previous, next);
    _notify();
  }

  /// Tells the app what just happened that it shows outside the page: a
  /// receive that finished, a pairing request that arrived, a copy that
  /// crossed.
  void _announce(LinkStatus previous, LinkStatus next) {
    final copy = next.clipboard.last;
    if (copy != null && copy.serial != previous.clipboard.last?.serial) {
      onClipboard?.call(copy);
    }
    final before = {
      for (final transfer in previous.transfers) transfer.id: transfer.state,
    };
    for (final transfer in next.transfers) {
      if (transfer.direction == TransferDirection.receive &&
          transfer.state == TransferState.done &&
          before[transfer.id] != TransferState.done) {
        onReceived?.call(transfer);
      }
    }
    final pairing = next.pairing;
    final was = previous.pairing;
    if (pairing != null &&
        _asksThisUser(pairing) &&
        (was == null ||
            !_asksThisUser(was) ||
            was.peerId != pairing.peerId ||
            was.code != pairing.code)) {
      onPairingRequest?.call(pairing);
    }
  }

  /// Adds dropped paths to the files waiting, each once.
  void stage(Iterable<String> paths) {
    var added = false;
    for (final path in paths) {
      if (path.isNotEmpty && !_staged.contains(path)) {
        _staged.add(path);
        added = true;
      }
    }
    if (added) _notify();
  }

  void unstage(String path) {
    if (_staged.remove(path)) _notify();
  }

  void clearStaged() {
    if (_staged.isEmpty) return;
    _staged.clear();
    _notify();
  }

  /// Sends the waiting files to [peerId]; they stop waiting once Rust took
  /// them. True when it did.
  Future<bool> sendStaged(String peerId) async {
    if (_staged.isEmpty) return false;
    final paths = List.of(_staged);
    final sent = await _run(
      StoreAction.linkSend,
      () => api.linkSend(peerId, paths),
    );
    if (sent) {
      _staged.removeWhere(paths.contains);
      _notify();
    }
    return sent;
  }

  /// Asks [peerId] to pair; both show a code, and it accepts.
  Future<bool> pair(String peerId) =>
      _run(StoreAction.linkPair, () => api.linkPair(peerId));

  /// The code the user read off the other device.
  Future<bool> enterCode(String code) =>
      _run(StoreAction.linkPair, () => api.linkPairCode(code));

  /// Shares the clipboard with paired devices, or stops.
  Future<bool> setClipboardSharing(bool enabled) => _run(
    StoreAction.linkClipboard,
    () async => _apply(await api.linkClipboardSet(enabled)),
  );

  /// Raises macOS's prompt, so Isthmus can be set to always allow.
  Future<bool> requestClipboardAccess() =>
      _run(StoreAction.linkClipboard, api.clipboardRequestAccess);

  /// Accepts the pairing another device asked for.
  Future<bool> acceptPairing() =>
      _run(StoreAction.linkPair, api.linkPairAccept);

  /// Whether [pairing] waits on this computer's user: to accept, once both
  /// show their code, or to read the code off for a version 1 device.
  static bool _asksThisUser(PairingInfo pairing) =>
      pairing.role == PairRole.incoming &&
      (pairing.stage == PairStage.code || pairing.stage == PairStage.confirm);

  /// Cancels the pairing in progress, declines one waiting to be accepted,
  /// or closes one that finished.
  Future<bool> cancelPairing() =>
      _run(StoreAction.linkPair, api.linkPairCancel);

  Future<bool> unpair(String peerId) =>
      _run(StoreAction.linkUnpair, () => api.linkUnpair(peerId));

  Future<bool> cancelTransfer(String id) =>
      _run(StoreAction.linkTransfer, () => api.linkTransferCancel(id));

  Future<bool> clearTransfers() =>
      _run(StoreAction.linkTransfer, api.linkTransfersClear);

  Future<bool> rename(String name) async {
    try {
      final next = await api.linkRename(name);
      if (!_disposed) _apply(next);
      return true;
    } catch (failure) {
      _fail(StoreAction.linkRename, failure);
      return false;
    }
  }

  /// Shows a received transfer, or the receive folder, in the file manager.
  Future<bool> reveal([String? id]) =>
      _run(StoreAction.linkReveal, () => api.linkReveal(id));

  /// Runs [call], reports a failure, and reads the snapshot after either.
  Future<bool> _run(StoreAction action, Future<Object?> Function() call) async {
    try {
      await call();
      return true;
    } catch (failure) {
      _fail(action, failure);
      return false;
    } finally {
      if (!_disposed) unawaited(refresh());
    }
  }

  void _fail(StoreAction action, Object failure) {
    if (_disposed) return;
    onFailure?.call(
      action,
      failure is IsthmusCallException
          ? failure
          : IsthmusCallException('$failure', code: ErrorCode.internal),
    );
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
