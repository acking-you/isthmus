import 'dart:async';
import 'dart:ffi';

import 'package:ffi/ffi.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/ffi/isthmus_api.dart';
import 'package:isthmus_ui/src/ffi/isthmus_service.dart';

/// `Dart_CObject_kString` in `dart_native_api.h`.
const _kString = 5;

/// Posts [text] to [port] as Rust's `emit_to` does: a `Dart_CObject` of
/// type `kString` (the type tag, then the union at offset 8, 48 bytes in
/// all) through the VM's own `Dart_PostCObject`. True if the VM took it.
bool _postAsRust(int port, String text) {
  final post = NativeApi.postCObject
      .asFunction<int Function(int, Pointer<Dart_CObject>)>();
  final message = calloc<Uint8>(48);
  final string = text.toNativeUtf8();
  try {
    message.cast<Int32>().value = _kString;
    Pointer<IntPtr>.fromAddress(message.address + 8).value = string.address;
    return post(port, message.cast()) != 0;
  } finally {
    calloc.free(message);
    malloc.free(string);
  }
}

void main() {
  /// A hint Rust posts arrives as a Change on the port the app registers,
  /// with its kind and sequence number, through the VM alone: no callback
  /// into an isolate is involved.
  test('a hint posted through the VM arrives as a change', () async {
    final received = Completer<Change>();
    final port = openChangePort(received.complete);
    addTearDown(port.close);

    expect(
      _postAsRust(port.sendPort.nativePort, '{"kind":"frontApp","seq":7}'),
      isTrue,
    );
    final change = await received.future.timeout(const Duration(seconds: 5));
    expect(change.kind, ChangeKind.frontApp);
    expect(change.seq, 7);
  });

  /// What keeps a hot restart from aborting the app: once the port is
  /// closed, as when its isolate dies, a post to it is refused, not
  /// delivered to code that no longer exists.
  test('a post to a closed port is refused', () {
    final port = openChangePort((_) => fail('a closed port got a hint'));
    final nativePort = port.sendPort.nativePort;
    port.close();
    expect(_postAsRust(nativePort, '{"kind":"status","seq":1}'), isFalse);
  });

  /// A hint that is not a JSON object string is dropped: the next one, or
  /// the next status poll, re-reads everything anyway.
  test('anything but a JSON object string is not a change', () {
    expect(
      decodeChangeHint('{"kind":"devices","seq":2}')?.kind,
      ChangeKind.devices,
    );
    expect(decodeChangeHint('{"kind":"newKind"}')?.kind, ChangeKind.unknown);
    for (final message in [null, 42, '', 'not json', '[1,2]']) {
      expect(decodeChangeHint(message), isNull, reason: '$message');
    }
  });
}
