/// What an update that replaces the app expects to find on the other side.
///
/// The app quits for the update, and whatever opens next is a new process:
/// left to itself it cannot tell an update that worked from one that never
/// ran (a declined permission prompt, a failed swap), and neither can the
/// person, who saw the app vanish. So the update writes down where it was
/// going before it hands off, and the next launch reads it once.
library;

import 'dart:convert';

import 'package:isthmus_ui/src/updates/release.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// An update handed off, on its way from [from] to [to].
class PendingUpdate {
  const PendingUpdate({
    required this.from,
    required this.to,
    required this.at,
    this.installer,
  });

  final Version from;
  final Version to;
  final DateTime at;

  /// The downloaded Windows installer it runs, which cannot remove itself.
  final String? installer;

  Map<String, Object> toJson() => {
    'from': '$from',
    'to': '$to',
    'at': at.toUtc().toIso8601String(),
    'installer': ?installer,
  };

  static PendingUpdate? fromJson(Object? json) {
    if (json is! Map) return null;
    final from = Version.tryParse('${json['from']}');
    final to = Version.tryParse('${json['to']}');
    final at = DateTime.tryParse('${json['at']}');
    if (from == null || to == null || at == null) return null;
    final installer = json['installer'];
    return PendingUpdate(
      from: from,
      to: to,
      at: at,
      installer: installer is String ? installer : null,
    );
  }
}

/// Where a [PendingUpdate] waits for the next launch.
abstract interface class UpdateRecord {
  Future<void> write(PendingUpdate pending);

  /// The pending update, if there is one, which is forgotten as it is read.
  Future<PendingUpdate?> take();
}

/// The app's preferences, which an update leaves alone: macOS keeps them by
/// bundle identifier and Windows by the names in the executable, neither of
/// which a version changes.
class PrefsUpdateRecord implements UpdateRecord {
  const PrefsUpdateRecord();

  static const _key = 'pendingUpdate';

  @override
  Future<void> write(PendingUpdate pending) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_key, jsonEncode(pending.toJson()));
  }

  @override
  Future<PendingUpdate?> take() async {
    final prefs = await SharedPreferences.getInstance();
    final text = prefs.getString(_key);
    if (text == null) return null;
    await prefs.remove(_key);
    try {
      return PendingUpdate.fromJson(jsonDecode(text));
    } on FormatException {
      return null;
    }
  }
}
