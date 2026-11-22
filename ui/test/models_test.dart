import 'dart:convert';
import 'dart:ffi';

import 'package:ffi/ffi.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/ffi/isthmus_api.dart';
import 'package:isthmus_ui/src/ffi/isthmus_service.dart';

void main() {
  /// A newer library's enum values read as the variant that changes
  /// nothing, as Rust's `#[serde(other)]` does, so an old app never crashes.
  test('unknown enum values read as the variant that changes nothing', () {
    final settings = Settings({
      'keyboard': {
        'defaultStyle': 'linux',
        'appRules': [
          {'bundleId': 'a.b', 'class': 'quantum'},
        ],
      },
      'mouse': {
        'defaults': {'pointer': 'warp', 'linesPerNotch': 99},
      },
    });
    expect(settings.keyboardDefaultStyle, KeyboardStyle.mac);
    expect(settings.appRules.single.appClass, AppClass.standard);
    expect(settings.mouseDefaults.pointer, PointerMode.system);
    expect(settings.mouseDefaults.linesPerNotch, 12);

    final status = Status({
      'keyboardTap': 'exploded',
      'permissions': {'accessibility': 'maybe'},
      'issues': [
        {'kind': 'brandNew'},
      ],
    });
    expect(status.keyboardTap, TapState.failed);
    expect(status.accessibility, PermissionState.unknown);
    expect(status.issues.single.kind, IssueKind.unknown);
    expect(DeviceEntry.fromJson({'kind': 'pen'}).kind, DeviceKind.unknown);
    expect(Change.fromJson({'kind': 'permissions'}).kind, ChangeKind.unknown);
    expect(ErrorCode.parse('SOMETHING_NEW'), ErrorCode.internal);
    expect(LoginItem.parse('later'), LoginItem.unavailable);
  });

  /// An empty file is Windows habits, switched off: Rust's defaults.
  test('an empty settings file reads as Windows habits switched off', () {
    final settings = Settings.initial;
    expect(settings.enabled, isFalse);
    expect(settings.keyboardStyle, ProfileStyle.windows);
    expect(settings.mouseStyle, ProfileStyle.windows);
    expect(settings.mouseDefaults, MouseProfile.windows);
  });

  /// The style names match Rust's: numbers count only while their switch is
  /// on, and one mouse of its own makes the mouse Custom.
  test('styles are decided as Rust decides them', () {
    var settings = Settings.initial.withPreset(Preset.mac, PresetTarget.mouse);
    expect(settings.mouseStyle, ProfileStyle.mac);
    // The speed is tuned only in Windows mode, so it does not unmake Mac.
    settings = settings.withMouseDefaults(
      settings.mouseDefaults.copyWith(pointerSpeed: 15),
    );
    expect(settings.mouseStyle, ProfileStyle.mac);
    settings = settings.withMouseDevice(
      'm:1',
      name: 'MX',
      profile: MouseProfile.windows,
    );
    expect(settings.mouseStyle, ProfileStyle.custom);

    var keyboard = Settings.initial.withKeyboardDevice(
      'k:1',
      name: 'K8',
      style: KeyboardStyle.mac,
    );
    expect(keyboard.keyboardStyle, ProfileStyle.custom);
    keyboard = keyboard.withPreset(Preset.mac, PresetTarget.keyboard);
    expect(keyboard.keyboardStyle, ProfileStyle.mac);
    expect(keyboard.functionKeysOf('k:1'), FunctionKeys.system);
  });

  /// The Windows preset switches Isthmus on and puts every group back on;
  /// layouts and rules are facts, not style, so they stay.
  test('the Windows preset keeps layouts and rules', () {
    final settings = Settings.initial
        .withKeyboardDevice('k:1', name: 'K8', layout: LayoutChoice.mac)
        .withGroup(ShortcutGroup.editing, false)
        .withAppRule(
          const AppRule(bundleId: 'x.y', appClass: AppClass.passthrough),
        )
        .withPreset(Preset.windows, PresetTarget.all);
    expect(settings.enabled, isTrue);
    expect(settings.groupEnabled(ShortcutGroup.editing), isTrue);
    expect(settings.layoutOf('k:1'), LayoutChoice.mac);
    expect(settings.appRules.single.appClass, AppClass.passthrough);
    expect(settings.keyboardStyle, ProfileStyle.windows);
  });

  /// An edit never reaches the settings it was made from.
  test('edits leave the original settings untouched', () {
    final original = Settings({
      'keyboard': {
        'groups': {'editing': true},
      },
    });
    final edited = original.withGroup(ShortcutGroup.editing, false);
    expect(original.groupEnabled(ShortcutGroup.editing), isTrue);
    expect(edited.groupEnabled(ShortcutGroup.editing), isFalse);
    expect(edited == original, isFalse);
    expect(
      Settings(jsonDecode(jsonEncode(edited.json)) as Map<String, Object?>),
      edited,
    );
  });

  group('the envelope decoder', () {
    /// Every answer is freed exactly once and read into one shape, whatever
    /// the library returned.
    Map<String, Object?> decode(String? raw) {
      var freed = 0;
      final ptr = raw == null ? nullptr.cast<Utf8>() : raw.toNativeUtf8();
      final result = decodeEnvelope(ptr, (p) {
        freed++;
        if (p != nullptr) malloc.free(p);
      });
      expect(freed, raw == null ? 0 : 1);
      return result;
    }

    test('passes a well-formed envelope through', () {
      expect(decode('{"success":true,"data":{"x":1}}'), {
        'success': true,
        'data': {'x': 1},
      });
    });

    test('turns nothing, junk and non-objects into INTERNAL failures', () {
      for (final raw in [null, '', 'not json', '[1,2]']) {
        final result = decode(raw);
        expect(result['success'], isFalse, reason: '$raw');
        expect(result['code'], 'INTERNAL', reason: '$raw');
      }
    });
  });
}
