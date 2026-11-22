import 'dart:async';
import 'dart:convert';
import 'dart:ffi';
import 'dart:isolate';

import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart';
import 'package:isthmus_ui/src/ffi/isthmus_ffi.dart';
import 'package:isthmus_ui/src/ffi/models.dart';

/// The bridge to `libisthmus_ffi`: one handle, calls on worker isolates, and
/// the change hints Rust sends.
///
/// Every call runs on a `compute` isolate, so a call that waits on the HID
/// thread never holds up a frame. The handle crosses to the worker as its
/// address; a worker still in a call borrows it, so [dispose] frees it only
/// once every call has returned (a use-after-free otherwise).
class IsthmusService {
  IsthmusService._();

  static final IsthmusService _instance = IsthmusService._();

  /// The one service per isolate; Rust's own service is one per process.
  factory IsthmusService() => _instance;

  final IsthmusFFI _ffi = IsthmusFFI();
  Pointer<Void>? _handle;
  int _activeCalls = 0;
  bool _disposeRequested = false;

  static final StreamController<Change> _changes =
      StreamController<Change>.broadcast();
  static ReceivePort? _changePort;

  /// Fires when something the UI shows has gone stale, whoever changed it:
  /// this window, the CLI, a device arriving, the front app changing.
  static Stream<Change> get changeStream => _changes.stream;

  Pointer<Void> _requireHandle() {
    if (_disposeRequested) throw StateError('The Isthmus service is disposed');
    final handle = _handle;
    if (handle != null && handle != nullptr) return handle;
    final created = _ffi.create();
    if (created == nullptr) {
      throw StateError('The Isthmus library could not make a handle');
    }
    return _handle = created;
  }

  /// One `{"op": …}` request on a worker isolate, answered with the
  /// envelope: `{"success": true, "data" | "message": …}` or
  /// `{"success": false, "message": …, "code": …}`. Never throws for a
  /// failure Rust reports; only for a library that cannot be loaded.
  Future<Map<String, Object?>> call(Map<String, Object?> request) async {
    final handle = _requireHandle();
    _activeCalls++;
    try {
      return await compute(_callJsonIsolate, (
        handle.address,
        jsonEncode(request),
      ), debugLabel: 'isthmus ${request['op']}');
    } finally {
      _activeCalls--;
      _destroyIfIdle();
    }
  }

  /// Begin delivering [changeStream]. Safe to call more than once.
  ///
  /// Rust posts each hint to a [ReceivePort] through the VM's own
  /// `Dart_PostCObject`, never by calling into this isolate. A hot restart
  /// kills this isolate without [dispose], and Rust keeps whatever it was
  /// given until the new isolate registers: a dead isolate's port only
  /// refuses the message, where a dead `NativeCallable` aborts the process.
  void initChangeEvents() {
    if (_changePort != null) return;
    // Looked up first: a library that will not load throws here, before a
    // port exists that nothing would ever close.
    final register = _ffi.setChangePort;
    final port = openChangePort(_changes.add);
    register(NativeApi.postCObject, port.sendPort.nativePort);
    _changePort = port;
  }

  /// Release the handle and the change port. Rust's service keeps running:
  /// the Runner's quit hook, not this, puts the system back.
  void dispose() {
    _disposeRequested = true;
    final port = _changePort;
    if (port != null) {
      // Clearing the slot waits for a post in flight, so nothing reaches
      // the port once it is closed.
      _ffi.setChangePort(nullptr, 0);
      port.close();
      _changePort = null;
    }
    _destroyIfIdle();
  }

  void _destroyIfIdle() {
    if (!_disposeRequested || _activeCalls != 0) return;
    final handle = _handle;
    if (handle != null && handle != nullptr) _ffi.destroy(handle);
    _handle = null;
  }
}

/// A port that turns each hint Rust posts into a [Change] for [onChange].
@visibleForTesting
ReceivePort openChangePort(void Function(Change) onChange) {
  final port = ReceivePort('isthmus change hints');
  port.listen((message) {
    final change = decodeChangeHint(message);
    if (change != null) onChange(change);
  });
  return port;
}

/// One message from Rust's change port as a [Change]: a JSON object string,
/// `{"kind": …, "seq": n}`. Anything else is null and ignored: a malformed
/// hint is not worth taking the app down for, since the next one, or the
/// next status poll, re-reads everything anyway.
@visibleForTesting
Change? decodeChangeHint(Object? message) {
  if (message is! String) return null;
  try {
    final decoded = jsonDecode(message);
    if (decoded is Map) return Change.fromJson(decoded.cast<String, Object?>());
  } on FormatException {
    // Ignored, as above.
  }
  return null;
}

/// The worker isolate's side of [IsthmusService.call]: rebuild the handle
/// from its address, make the call, and free both strings.
Map<String, Object?> _callJsonIsolate((int, String) args) {
  final (address, request) = args;
  final ffi = IsthmusFFI();
  final arg = request.toNativeUtf8();
  try {
    return decodeEnvelope(
      ffi.callJson(Pointer<Void>.fromAddress(address), arg),
      ffi.freeString,
    );
  } finally {
    malloc.free(arg);
  }
}

/// The envelope at [ptr] as a map, freed with [free] whatever it holds. A
/// null, empty or non-object answer becomes a failure with code `INTERNAL`,
/// so the caller has one shape to read.
@visibleForTesting
Map<String, Object?> decodeEnvelope(
  Pointer<Utf8> ptr,
  void Function(Pointer<Utf8>) free,
) {
  if (ptr == nullptr) {
    return const {
      'success': false,
      'message': 'The library returned nothing',
      'code': 'INTERNAL',
    };
  }
  try {
    final raw = ptr.toDartString();
    final decoded = raw.isEmpty ? null : jsonDecode(raw);
    if (decoded is Map) return decoded.cast<String, Object?>();
    return const {
      'success': false,
      'message': 'The library returned something other than an envelope',
      'code': 'INTERNAL',
    };
  } on FormatException catch (e) {
    return {
      'success': false,
      'message': 'The library returned invalid JSON: ${e.message}',
      'code': 'INTERNAL',
    };
  } finally {
    free(ptr);
  }
}
