/// Putting a verified update in place, per platform.
///
/// By the time anything here runs, the file's size and SHA-256 have been
/// checked against a manifest signed by the release key. What is left is the
/// platform's own way of replacing a running app: Windows starts the
/// installer, which waits for the app to quit; a Mac copies the new app out
/// of the disk image beside this one, checks it is the same app signed by
/// the same developer, and swaps the two once the app has quit; and a
/// portable archive, or a disk image whose app cannot be swapped in, is
/// handed to the user, who knows where the app lives.
library;

import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:isthmus_ui/src/updates/release.dart';
import 'package:url_launcher/url_launcher.dart';

/// The platform effects of an update, behind an interface so the flow can be
/// tested without replacing anything.
abstract interface class UpdateInstaller {
  /// Start [installer], which waits for this process to quit before it
  /// replaces anything and opens the app again when it ends. Completes once
  /// the installer runs with the rights it installs with, which Windows may
  /// first ask the user for, in front of this app: the caller then quits.
  /// Throws an [UpdateException] when the installer could not start or ended
  /// before that (permission refused, a failure), and the caller stays.
  Future<void> startInstaller(File installer);

  /// Copy the app out of [image] beside [bundle] and check it, then arrange
  /// for it to take [bundle]'s place, and open, once this process has
  /// exited. The caller quits right after. Throws [ReplaceRefused] when this
  /// copy is not one an update may replace (a folder the user cannot write
  /// to, an unsigned development build), and an [UpdateException] when the
  /// image's app is not this app signed by the same developer.
  Future<void> replaceApp(File image, String bundle);

  /// Move [download] into Downloads as [name] and show it; returns where it
  /// went.
  Future<String> saveToDownloads(File download, String name);
}

/// This copy of the app is not one an update may replace in place; the
/// update is saved for the user instead.
class ReplaceRefused implements Exception {
  const ReplaceRefused(this.reason);

  final String reason;

  @override
  String toString() => reason;
}

class PlatformUpdateInstaller implements UpdateInstaller {
  const PlatformUpdateInstaller();

  @override
  Future<void> startInstaller(File installer) =>
      WindowsInstallerUpdate.start(installer, pid);

  @override
  Future<void> replaceApp(File image, String bundle) async {
    final staged = await MacAppUpdate.stage(image, bundle);
    await MacAppUpdate.swapAfterExit(staged, bundle, pid);
  }

  @override
  Future<String> saveToDownloads(File download, String name) async {
    final downloads = Directory(downloadsFolder() ?? download.parent.path);
    await downloads.create(recursive: true);
    final target = File('${downloads.path}${Platform.pathSeparator}$name');
    if (await target.exists()) await target.delete();
    final saved = await download.copy(target.path);
    await _delete(download);
    if (Platform.isWindows) {
      // Explorer, with the file selected.
      await Process.start('explorer.exe', ['/select,', saved.path]);
    } else if (Platform.isMacOS) {
      // Opening the disk image mounts it, with the app ready to drag into
      // Applications.
      await Process.run('/usr/bin/open', [saved.path]);
    }
    return saved.path;
  }

  /// The user's Downloads folder, where file sharing also saves: the
  /// default location under their home folder.
  static String? downloadsFolder() {
    final home = Platform.isWindows
        ? Platform.environment['USERPROFILE']
        : Platform.environment['HOME'];
    if (home == null || home.isEmpty) return null;
    return '$home${Platform.pathSeparator}Downloads';
  }
}

/// Open [url] in the browser.
Future<void> openInBrowser(Uri url) async {
  try {
    await launchUrl(url, mode: LaunchMode.externalApplication);
  } catch (error) {
    debugPrint('Could not open $url: $error');
  }
}

Future<void> _delete(File file) async {
  try {
    if (await file.exists()) await file.delete();
  } catch (_) {
    // A leftover temporary file is not worth failing over.
  }
}

/// Windows, installed: the Inno Setup installer that
/// `ui/windows/packaging/exe/inno_setup.iss` defines. It updates the
/// installation it finds by its AppId, a machine-wide one after Windows asks
/// for permission, waits for the app named by `/UPDATEPID` to quit before it
/// replaces a file, and when it ends opens the installed app again as the
/// user who started it.
abstract final class WindowsInstallerUpdate {
  /// Whether this copy was set up by the installer rather than unpacked from
  /// the portable archive: Inno Setup leaves its uninstaller beside the
  /// executable.
  static bool isInstalled([String? executable]) => File(
    '${File(executable ?? Platform.resolvedExecutable).parent.path}'
    '${Platform.pathSeparator}unins000.exe',
  ).existsSync();

  /// Setup's log, beside the installer: the download's folder is its own.
  static File log(File installer) =>
      File('${installer.parent.path}${Platform.pathSeparator}setup.log');

  /// Start [installer] to update the app running as [processId]; see
  /// [UpdateInstaller.startInstaller].
  static Future<void> start(File installer, int processId) =>
      run(installer.path, [
        '/SILENT',
        '/SUPPRESSMSGBOXES',
        '/NORESTART',
        '/SP-',
        '/LOG=${log(installer).path}',
        '/UPDATEPID=$processId',
      ], log(installer));

  /// Start [executable] with [arguments], and complete once [ready] exists
  /// while it still runs. Setup starts its log only in the process that
  /// installs: for an installation for all users, the one Windows starts
  /// elevated once the user allowed it. A [ready] left by an earlier attempt
  /// is removed first, so that it cannot count.
  ///
  /// Started from here, while this app is open and in front, so that Windows
  /// asks for that permission in front of it rather than from behind a
  /// closed app, where the prompt can go unseen.
  ///
  /// Throws an [UpdateException] when [executable] cannot start, or ends
  /// first.
  @visibleForTesting
  static Future<void> run(
    String executable,
    List<String> arguments,
    File ready, {
    Duration poll = const Duration(milliseconds: 100),
  }) async {
    if (await ready.exists()) await ready.delete();
    final Process setup;
    try {
      setup = await Process.start(executable, arguments);
    } on ProcessException catch (error) {
      throw UpdateException(
        UpdateFailure.install,
        'Could not start the installer: ${error.message}',
      );
    }
    // Setup writes nothing there; drained, the pipes can never fill.
    unawaited(setup.stdout.drain<void>());
    unawaited(setup.stderr.drain<void>());
    int? ended;
    unawaited(setup.exitCode.then((code) => ended = code));
    while (ended == null && !await ready.exists()) {
      await Future<void>.delayed(poll);
    }
    if (ended case final code?) {
      throw UpdateException(
        UpdateFailure.install,
        'The installer ended with exit code $code before installing anything'
        '${code == 2 ? ': Windows was not given permission, or it was cancelled' : ''}.',
      );
    }
  }

  /// What is left of an update's download once the app it installed has
  /// opened: the installer goes, and Setup's log too when the update went in,
  /// then the folder if that left it empty. The log of an update that did not
  /// go in stays, for whoever asks why.
  static Future<void> removeDownload(
    String installer, {
    required bool installed,
  }) async {
    final file = File(installer);
    await _delete(file);
    if (installed) await _delete(log(file));
    try {
      await file.parent.delete();
    } on FileSystemException {
      // Not empty, or already gone.
    }
  }
}

/// A new app copied out of the disk image, waiting beside the one it
/// replaces.
class StagedApp {
  const StagedApp({required this.folder, required this.app});

  /// The hidden folder beside the installed app; removed once the swap is
  /// done, with the old app in it.
  final String folder;

  /// The new app inside it.
  final String app;
}

/// How a bundle is signed, as `codesign` reports it.
@visibleForTesting
class CodeIdentity {
  const CodeIdentity({required this.identifier, required this.team});

  /// The bundle identifier the signature seals.
  final String? identifier;

  /// The developer team, or null for an ad-hoc signature ("not set").
  final String? team;

  /// Reads `codesign --display --verbose=2`'s report.
  static CodeIdentity parse(String report) {
    String? field(String name) {
      final match = RegExp('^$name=(.+)\$', multiLine: true).firstMatch(report);
      final value = match?[1]?.trim();
      return value == null || value == 'not set' ? null : value;
    }

    return CodeIdentity(
      identifier: field('Identifier'),
      team: field('TeamIdentifier'),
    );
  }
}

/// macOS: the app is replaced the way the user would replace it, by putting
/// the disk image's `Isthmus.app` where this one is, but only after this
/// process has quit, and only with a copy that is the same app signed by the
/// same developer. That signature is what Accessibility's grant hangs on, so
/// the grant carries over to the new version.
abstract final class MacAppUpdate {
  /// The `.app` this process runs from, or null when it runs from no bundle.
  static String? current([String? executable]) {
    final bundle = File(executable ?? Platform.resolvedExecutable)
        .parent
        .parent
        .parent
        .path;
    return bundle.endsWith('.app') ? bundle : null;
  }

  /// The staging folder beside [bundle] for this process.
  static Directory stagingFor(String bundle, int processId) =>
      Directory('${File(bundle).parent.path}/.isthmus-update-$processId');

  /// Copy the app out of [image] next to [bundle] and check that it is this
  /// app, signed by the same developer. See [UpdateInstaller.replaceApp].
  static Future<StagedApp> stage(File image, String bundle) async {
    final current = await identity(bundle);
    if (current.team == null || current.identifier == null) {
      throw const ReplaceRefused(
        'This copy has no Developer ID signature, so an update would not '
        'keep its Accessibility permission',
      );
    }
    final staging = stagingFor(bundle, pid);
    try {
      if (await staging.exists()) await staging.delete(recursive: true);
      await staging.create();
    } on FileSystemException catch (error) {
      throw ReplaceRefused('Cannot write beside $bundle: ${error.message}');
    }
    try {
      final staged = '${staging.path}/Isthmus.app';
      await _copyOutOf(image, staged);
      await _run('/usr/bin/codesign', [
        '--verify',
        '--deep',
        '--strict',
        staged,
      ], 'The new app fails its signature check');
      final next = await identity(staged);
      if (next.team != current.team || next.identifier != current.identifier) {
        throw UpdateException(
          UpdateFailure.install,
          'The new app is ${next.identifier} from ${next.team}, this one is '
          '${current.identifier} from ${current.team}',
        );
      }
      return StagedApp(folder: staging.path, app: staged);
    } catch (_) {
      await staging.delete(recursive: true).catchError((_) => staging);
      rethrow;
    }
  }

  /// `codesign`'s report on [path]'s signature.
  static Future<CodeIdentity> identity(String path) async {
    final result = await Process.run('/usr/bin/codesign', [
      '--display',
      '--verbose=2',
      path,
    ]);
    // The report goes to standard error.
    return CodeIdentity.parse('${result.stderr}\n${result.stdout}');
  }

  /// Mounts [image] where nobody sees it, copies its app to [target] with
  /// everything a signature covers, and unmounts it.
  static Future<void> _copyOutOf(File image, String target) async {
    final mount = await Directory.systemTemp.createTemp('isthmus-update-');
    await _run('/usr/bin/hdiutil', [
      'attach',
      '-readonly',
      '-nobrowse',
      '-noautoopen',
      '-mountpoint',
      mount.path,
      image.path,
    ], 'Could not open the disk image');
    try {
      final app = Directory('${mount.path}/Isthmus.app');
      if (!await app.exists()) {
        throw const UpdateException(
          UpdateFailure.install,
          'The disk image holds no Isthmus.app',
        );
      }
      await _run('/usr/bin/ditto', [
        app.path,
        target,
      ], 'Could not copy the new app');
    } finally {
      final detached = await Process.run('/usr/bin/hdiutil', [
        'detach',
        mount.path,
      ]);
      if (detached.exitCode != 0) {
        await Process.run('/usr/bin/hdiutil', ['detach', '-force', mount.path]);
      }
      await mount.delete().catchError((_) => mount);
    }
  }

  static Future<void> _run(
    String executable,
    List<String> arguments,
    String failure,
  ) async {
    final ProcessResult result;
    try {
      result = await Process.run(executable, arguments);
    } on ProcessException catch (error) {
      throw UpdateException(UpdateFailure.install, '$failure: $error');
    }
    if (result.exitCode != 0) {
      throw UpdateException(
        UpdateFailure.install,
        '$failure: ${'${result.stderr}'.trim()}',
      );
    }
  }

  /// The helper that waits for [processId] to end, swaps [staged] in for
  /// [bundle] (two renames in one folder, the old app kept until the new
  /// one is in place and put back if it could not be), and opens whichever
  /// app is there. It gives up after a minute if the app never quits.
  @visibleForTesting
  static const swapScript = r'''
i=0
while /bin/kill -0 "$1" 2>/dev/null; do
  i=$((i + 1))
  if [ "$i" -gt 600 ]; then /bin/rm -rf "$4"; exit 1; fi
  /bin/sleep 0.1
done
if /bin/mv "$3" "$4/previous.app"; then
  /bin/mv "$2" "$3" || /bin/mv "$4/previous.app" "$3"
fi
if [ -d "$3" ]; then /bin/rm -rf "$4"; fi
/usr/bin/open "$3"
''';

  /// Starts that helper, detached, so it outlives this process.
  static Future<void> swapAfterExit(
    StagedApp staged,
    String bundle,
    int processId,
  ) async {
    try {
      await Process.start('/bin/sh', [
        '-c',
        swapScript,
        'isthmus-update',
        '$processId',
        staged.app,
        bundle,
        staged.folder,
      ], mode: ProcessStartMode.detached);
    } on ProcessException catch (error) {
      await Directory(staged.folder)
          .delete(recursive: true)
          .catchError((_) => Directory(staged.folder));
      throw UpdateException(
        UpdateFailure.install,
        'Could not start the update helper: $error',
      );
    }
  }
}
