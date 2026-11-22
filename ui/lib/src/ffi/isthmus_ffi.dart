import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

/// `isthmus_call_json(handle, request)`: one `{"op": …}` request in, one
/// JSON envelope out, which the caller frees with `isthmus_free_string`.
typedef CallJsonNative = Pointer<Utf8> Function(
  Pointer<Void> handle,
  Pointer<Utf8> request,
);

/// `Dart_PostCObject`, as [NativeApi.postCObject] hands it out: the VM's
/// own function, valid for the life of the process.
typedef PostCObjectNative = Int8 Function(Int64, Pointer<Dart_CObject>);

/// `isthmus_set_change_port(post, port)`: Rust posts each change hint, a
/// JSON string, to `port` through `post`. A null `post` or port 0 clears the
/// slot, waiting for a post in flight before it returns.
typedef SetChangePortNative = Void Function(
  Pointer<NativeFunction<PostCObjectNative>> post,
  Int64 port,
);

/// The raw symbols of `libisthmus_ffi`, one copy per isolate.
///
/// The library file name is also spelled in the Runner's xcconfigs, its Run
/// Script build phase and `scripts/build/isthmus-ffi-macos.sh`; renaming it
/// here alone breaks the app only when the first call is made.
class IsthmusFFI {
  IsthmusFFI._();

  static IsthmusFFI? _instance;
  static DynamicLibrary? _lib;

  /// The one copy in this isolate. Worker isolates make their own, which
  /// open the same library the process already loaded.
  factory IsthmusFFI() => _instance ??= IsthmusFFI._();

  /// The library, opened on first use so a test that never calls Rust never
  /// needs it.
  DynamicLibrary get lib => _lib ??= _loadLibrary();

  /// The copy the Runner embeds in `Contents/Frameworks` first; the bare
  /// name second, for `flutter test` with the library beside the project.
  static DynamicLibrary _loadLibrary() {
    if (Platform.isMacOS) {
      try {
        return DynamicLibrary.open(
          '@executable_path/../Frameworks/libisthmus_ffi.dylib',
        );
      } on ArgumentError {
        return DynamicLibrary.open('libisthmus_ffi.dylib');
      }
    }
    throw UnsupportedError('Unsupported platform: ${Platform.operatingSystem}');
  }

  /// A new handle, a view of the process's one service.
  late final create = lib
      .lookupFunction<Pointer<Void> Function(), Pointer<Void> Function()>(
        'isthmus_create',
      );

  /// Frees a handle; every call borrowing it must have returned.
  late final destroy = lib
      .lookupFunction<
        Void Function(Pointer<Void>),
        void Function(Pointer<Void>)
      >('isthmus_destroy');

  /// Every operation, as `{"op": …}` JSON.
  late final callJson = lib.lookupFunction<CallJsonNative, CallJsonNative>(
    'isthmus_call_json',
  );

  /// Frees every string the library returns.
  late final freeString = lib
      .lookupFunction<
        Void Function(Pointer<Utf8>),
        void Function(Pointer<Utf8>)
      >('isthmus_free_string');

  /// Registers (or with `nullptr` and 0, clears) the port change hints go
  /// to.
  late final setChangePort = lib
      .lookupFunction<
        SetChangePortNative,
        void Function(Pointer<NativeFunction<PostCObjectNative>>, int)
      >('isthmus_set_change_port');
}
