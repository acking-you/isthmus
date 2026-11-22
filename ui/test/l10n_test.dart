import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

Map<String, Object?> _arb(String name) =>
    jsonDecode(File('lib/l10n/$name').readAsStringSync())
        as Map<String, Object?>;

/// The placeholders `{name}` a message uses, plural branches included.
Set<String> _placeholders(String message) => {
  for (final match in RegExp(r'\{(\w+)[,}]').allMatches(message))
    match.group(1)!,
};

void main() {
  /// The user reads Chinese: a key missing from app_zh.arb would fall back
  /// to English without a word from gen-l10n.
  test('every English string has a Chinese translation', () {
    final en = _arb('app_en.arb');
    final zh = _arb('app_zh.arb');
    final messages = {
      for (final key in en.keys)
        if (!key.startsWith('@')) key,
    };
    final translated = {
      for (final key in zh.keys)
        if (!key.startsWith('@')) key,
    };
    expect(translated, messages);
  });

  /// A translation that drops or renames a placeholder loses the value.
  test('translations use the same placeholders', () {
    final en = _arb('app_en.arb');
    final zh = _arb('app_zh.arb');
    for (final key in en.keys.where((k) => !k.startsWith('@'))) {
      expect(
        _placeholders(zh[key]! as String),
        _placeholders(en[key]! as String),
        reason: key,
      );
    }
  });

  /// gen-l10n's own report, when it has run: nothing untranslated.
  test('gen-l10n reports nothing untranslated', () {
    final report = File('l10n_untranslated.json');
    if (!report.existsSync()) return;
    expect(jsonDecode(report.readAsStringSync()), isEmpty);
  });
}
