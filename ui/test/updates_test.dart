import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:isthmus_ui/src/shell/app_scope.dart';
import 'package:isthmus_ui/src/updates/release.dart';
import 'package:isthmus_ui/src/updates/update_controller.dart';
import 'package:isthmus_ui/src/updates/update_installers.dart';
import 'package:isthmus_ui/src/updates/update_record.dart';
import 'package:isthmus_ui/src/updates/update_source.dart';
import 'package:material_ui/material_ui.dart';

import 'fake_isthmus_api.dart';
import 'support.dart';

/// GitHub, from memory: a release listing, one release's manifest and its
/// signature, and the bytes of each file.
class FakeHttp implements UpdateHttp {
  FakeHttp(this.bodies, {this.files = const {}});

  final Map<Uri, List<int>> bodies;
  final Map<Uri, List<int>> files;
  final List<Uri> downloads = [];

  @override
  Future<List<int>> get(Uri url, {int limit = 1 << 20}) async {
    final body = bodies[url];
    if (body == null) {
      throw UpdateException(UpdateFailure.network, 'HTTP 404 from $url');
    }
    return body;
  }

  @override
  Future<String> download(
    Uri url,
    File target, {
    required int size,
    required void Function(int received) onProgress,
  }) async {
    downloads.add(url);
    final bytes = files[url]!;
    await target.writeAsBytes(bytes);
    onProgress(bytes.length);
    return '${sha256.convert(bytes)}';
  }

  @override
  void cancel() {}
}

/// Records what it was asked to do, and replaces nothing.
class FakeInstaller implements UpdateInstaller {
  final List<String> calls = [];
  bool refuseReplace = false;

  @override
  Future<void> startInstaller(File installer) async =>
      calls.add('installer ${installer.uri.pathSegments.last}');

  @override
  Future<void> replaceApp(File image, String bundle) async {
    if (refuseReplace) throw const ReplaceRefused('read-only');
    calls.add('replace $bundle with ${image.uri.pathSegments.last}');
  }

  @override
  Future<String> saveToDownloads(File download, String name) async {
    calls.add('save $name');
    return '/Downloads/$name';
  }
}

class MemoryRecord implements UpdateRecord {
  PendingUpdate? pending;

  @override
  Future<void> write(PendingUpdate pending) async => this.pending = pending;

  @override
  Future<PendingUpdate?> take() async {
    final taken = pending;
    pending = null;
    return taken;
  }
}

/// A published release of [version] with [files], as GitHub lists it.
Map<String, Object?> releaseJson(
  String tag, {
  List<String> files = const [],
  bool draft = false,
  bool prerelease = false,
}) => {
  'tag_name': tag,
  'draft': draft,
  'prerelease': prerelease,
  'html_url': 'https://github.com/acking-you/isthmus/releases/tag/$tag',
  'assets': [
    for (final name in files)
      {
        'name': name,
        'browser_download_url':
            'https://github.com/acking-you/isthmus/releases/download/$tag/$name',
      },
  ],
};

Uri asset(String tag, String name) => Uri.parse(
  'https://github.com/acking-you/isthmus/releases/download/$tag/$name',
);

/// A world with Isthmus 0.2.0 released, signed, with a DMG for Apple
/// silicon whose bytes are [dmg]; the manifest lists [listed] as its hash.
({FakeHttp http, FakeIsthmusApi api, List<int> dmg}) world({
  String? listed,
  bool signed = true,
}) {
  const tag = 'v0.2.0';
  final dmg = utf8.encode('the new app');
  final manifest = utf8.encode(
    jsonEncode({
      'version': '0.2.0',
      'assets': [
        {
          'name': 'Isthmus-macos-arm64.dmg',
          'size': dmg.length,
          'sha256': listed ?? '${sha256.convert(dmg)}',
        },
      ],
    }),
  );
  final api = FakeIsthmusApi();
  if (signed) api.validSignatures.add('good signature');
  final http = FakeHttp(
    {
      UpdateSource.releasesUrl: utf8.encode(
        jsonEncode([
          releaseJson(
            'v0.3.0-beta',
            files: const ['release-manifest.json'],
            prerelease: true,
          ),
          releaseJson(
            tag,
            files: const [
              'Isthmus-macos-arm64.dmg',
              manifestName,
              signatureName,
            ],
          ),
          releaseJson('v0.1.0'),
        ]),
      ),
      asset(tag, manifestName): manifest,
      asset(tag, signatureName): utf8.encode(
        base64Encode(utf8.encode('good signature')),
      ),
    },
    files: {asset(tag, 'Isthmus-macos-arm64.dmg'): dmg},
  );
  return (http: http, api: api, dmg: dmg);
}

UpdateController controller(
  ({FakeHttp http, FakeIsthmusApi api, List<int> dmg}) world, {
  required FakeInstaller installer,
  required MemoryRecord record,
  required List<String> quits,
  String installed = '0.1.0',
}) => UpdateController(
  quit: () async => quits.add('quit'),
  api: world.api,
  source: UpdateSource(api: world.api, http: world.http),
  installer: installer,
  record: record,
  loadInstalled: () async => Version.tryParse(installed)!,
  loadHost: () async => const UpdateHost(
    os: 'macos',
    arch: 'arm64',
    appBundle: '/Applications/Isthmus.app',
  ),
  workDir: () => Directory.systemTemp.createTemp('isthmus-update-test-'),
  replaceSelf: true,
);

void main() {
  /// The newest stable `vX.Y.Z` release is the update; drafts, prereleases
  /// and other tags are not.
  test('the newest stable release is the one offered', () {
    final newest = PublishedRelease.newest([
      releaseJson('v0.3.0', draft: true),
      releaseJson('v0.4.0-rc1'),
      releaseJson('v0.2.5', prerelease: true),
      releaseJson('v0.2.0'),
      releaseJson('v0.10.0'),
      releaseJson('v0.9.9'),
    ]);
    expect('${newest!.version}', '0.10.0');
  });

  /// Each computer takes its own file and its own way of applying it.
  test('each computer takes its own package and method', () {
    final manifest = ReleaseManifest.parse(
      jsonEncode({
        'version': '1.0.0',
        'assets': [
          for (final name in [
            'Isthmus-macos-arm64.dmg',
            'Isthmus-macos-x86_64.dmg',
            'Isthmus-windows-x86_64-setup.exe',
            'Isthmus-windows-x86_64.zip',
          ])
            {'name': name, 'size': 1, 'sha256': 'a' * 64},
        ],
      }),
    );
    String plan(UpdateHost host) {
      final plan = planUpdate(manifest, host)!;
      return '${plan.asset.name} ${plan.method.name}';
    }

    expect(
      plan(
        const UpdateHost(os: 'windows', arch: 'x64', windowsInstalled: true),
      ),
      'Isthmus-windows-x86_64-setup.exe installer',
    );
    expect(
      plan(const UpdateHost(os: 'windows', arch: 'x64')),
      'Isthmus-windows-x86_64.zip saveDownload',
    );
    expect(
      plan(const UpdateHost(os: 'macos', arch: 'arm64', appBundle: '/A.app')),
      'Isthmus-macos-arm64.dmg replaceApp',
    );
    expect(
      plan(const UpdateHost(os: 'macos', arch: 'x64')),
      'Isthmus-macos-x86_64.dmg saveDownload',
    );
    expect(
      planUpdate(manifest, const UpdateHost(os: 'linux', arch: 'x64')),
      isNull,
    );
  });

  /// A name from the manifest becomes a file name on disk, so one that could
  /// leave its folder is refused.
  test('an asset name that could leave its folder is refused', () {
    for (final name in ['../Isthmus.dmg', '/etc/passwd', '.hidden', 'a/b']) {
      expect(
        () => ReleaseAsset.fromJson({
          'name': name,
          'size': 1,
          'sha256': 'a' * 64,
        }),
        throwsFormatException,
        reason: name,
      );
    }
  });

  /// `codesign`'s report gives the identifier and the team, and an ad-hoc
  /// signature has no team.
  test('the signing team is read from codesign, and ad hoc has none', () {
    final signed = CodeIdentity.parse(
      'Executable=/A.app/Contents/MacOS/Isthmus\n'
      'Identifier=dev.acking.isthmus\nTeamIdentifier=ABCDE12345\n',
    );
    expect(signed.identifier, 'dev.acking.isthmus');
    expect(signed.team, 'ABCDE12345');
    final adHoc = CodeIdentity.parse(
      'Identifier=dev.acking.isthmus-local\nTeamIdentifier=not set\n',
    );
    expect(adHoc.team, isNull);
  });

  /// Found, confirmed, downloaded and verified, a Mac update swaps the app
  /// in place, notes where it was going, and quits; the next launch reads
  /// the note once.
  test('a signed, matching download replaces the app and quits', () async {
    final w = world();
    final installer = FakeInstaller();
    final record = MemoryRecord();
    final quits = <String>[];
    final updates = controller(
      w,
      installer: installer,
      record: record,
      quits: quits,
    );

    await updates.check();
    expect(updates.stage, UpdateStage.available);
    expect('${updates.published!.version}', '0.2.0');
    expect(w.api.calls, contains('update_verify'));

    await updates.start();
    expect(installer.calls, [
      'replace /Applications/Isthmus.app with Isthmus-macos-arm64.dmg',
    ]);
    expect(quits, ['quit']);
    expect('${record.pending!.to}', '0.2.0');

    final next = controller(
      w,
      installer: installer,
      record: record,
      quits: quits,
      installed: '0.2.0',
    );
    final outcome = await next.takeOutcome();
    expect(outcome!.installed, isTrue);
    expect(await next.takeOutcome(), isNull, reason: 'read once');
  });

  /// A manifest the release key did not sign is refused before anything is
  /// downloaded.
  test('an unsigned manifest stops before any download', () async {
    final w = world(signed: false);
    final installer = FakeInstaller();
    final updates = controller(
      w,
      installer: installer,
      record: MemoryRecord(),
      quits: [],
    );
    await updates.check();
    expect(updates.stage, UpdateStage.failed);
    expect(updates.failure!.kind, UpdateFailure.signature);
    expect(w.http.downloads, isEmpty);
  });

  /// A download whose hash is not the one the manifest lists is never
  /// installed, and the app stays open.
  test(
    'a download that does not match the manifest is never installed',
    () async {
      final w = world(listed: 'b' * 64);
      final installer = FakeInstaller();
      final quits = <String>[];
      final updates = controller(
        w,
        installer: installer,
        record: MemoryRecord(),
        quits: quits,
      );
      await updates.check();
      await updates.start();
      expect(updates.stage, UpdateStage.failed);
      expect(updates.failure!.kind, UpdateFailure.checksum);
      expect(installer.calls, isEmpty);
      expect(quits, isEmpty);
    },
  );

  /// A copy that may not be replaced in place (a folder the user cannot
  /// write to, a development build) gets the disk image in Downloads, and
  /// the app stays open.
  test('an app that cannot be replaced gets the disk image instead', () async {
    final w = world();
    final installer = FakeInstaller()..refuseReplace = true;
    final quits = <String>[];
    final updates = controller(
      w,
      installer: installer,
      record: MemoryRecord(),
      quits: quits,
    );
    await updates.check();
    await updates.start();
    expect(updates.stage, UpdateStage.saved);
    expect(installer.calls, ['save Isthmus-macos-arm64.dmg']);
    expect(quits, isEmpty);
  });

  /// Settings offers the release a check found, and its dialog says what
  /// will happen before anything is downloaded.
  testWidgets('settings offers a newer release and opens its dialog', (
    tester,
  ) async {
    final w = world();
    final updates = controller(
      w,
      installer: FakeInstaller(),
      record: MemoryRecord(),
      quits: [],
    );
    await tester.runAsync(updates.check);
    await pumpApp(tester, w.api, page: AppPage.settings, updates: updates);

    await tapInList(
      tester,
      find.byKey(const Key('settings-update')),
      list: 'settings-list',
    );
    expect(find.byKey(const Key('update-available')), findsOneWidget);
    expect(find.text('Isthmus 0.2.0 is available'), findsOneWidget);
    expect(w.http.downloads, isEmpty);

    await tester.tap(find.text('Cancel'));
    await settle(tester);
    await finish(tester);
  });

  /// The keep-awake switch writes the choice and shows what the library
  /// holds.
  testWidgets('the keep-awake switch turns it on', (tester) async {
    final api = FakeIsthmusApi();
    await pumpApp(tester, api, page: AppPage.settings);

    await tapInList(
      tester,
      find.byKey(const Key('settings-awake')),
      list: 'settings-list',
    );
    expect(api.calls, contains('awake_set true'));
    expect(api.awake.enabled, isTrue);
    await finish(tester);
  });
}
