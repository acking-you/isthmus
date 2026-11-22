/// What a release offers, as its signed manifest describes it, and which of
/// its files this installation should take.
///
/// After pb-mapper's updater. The release workflow lists every file of a
/// `vX.Y.Z` release with its size and SHA-256 in `release-manifest.json` and
/// signs that with the release key (`scripts/release/`); the app installs
/// nothing the signature does not cover.
library;

import 'dart:convert';

/// The repository whose releases are this app's updates.
const String updateRepository = 'acking-you/isthmus';

/// The Ed25519 key release manifests are signed with: its 32 raw bytes in
/// base64. `scripts/release/update_signing_key.pub.pem` is the same key for
/// the release workflow; the private half lives only in the repository's
/// `UPDATE_SIGNING_KEY` secret and the maintainer's backup.
const String updatePublicKey = 'hdesiJwz0+lsW43ArRyqqYuNubEAPuLw5pa1ZmTzLGU=';

const String manifestName = 'release-manifest.json';
const String signatureName = 'release-manifest.json.sig';

/// Why an update could not go ahead. The dialog words it from [kind].
class UpdateException implements Exception {
  const UpdateException(this.kind, [this.detail]);

  final UpdateFailure kind;

  /// The underlying error, for the log and the dialog's small print.
  final String? detail;

  @override
  String toString() => detail == null ? kind.name : '${kind.name}: $detail';
}

enum UpdateFailure {
  /// GitHub could not be reached, or answered with something unexpected.
  network,

  /// The release carries no signed manifest: its signing step failed, or it
  /// was published by hand.
  unsigned,

  /// The manifest's signature, or its version, does not hold.
  signature,

  /// A download does not match the size or SHA-256 the manifest lists.
  checksum,

  /// Putting the update in place failed.
  install,
}

/// A release version, `major.minor.patch`.
class Version implements Comparable<Version> {
  const Version(this.major, this.minor, this.patch);

  final int major;
  final int minor;
  final int patch;

  static final _pattern = RegExp(r'^(\d+)\.(\d+)\.(\d+)$');

  /// [text] as a version, or null when it is not one. A build suffix
  /// (`0.1.0+1`) is ignored.
  static Version? tryParse(String text) {
    final match = _pattern.firstMatch(text.split('+').first.trim());
    if (match == null) return null;
    return Version(
      int.parse(match[1]!),
      int.parse(match[2]!),
      int.parse(match[3]!),
    );
  }

  @override
  int compareTo(Version other) {
    if (major != other.major) return major.compareTo(other.major);
    if (minor != other.minor) return minor.compareTo(other.minor);
    return patch.compareTo(other.patch);
  }

  bool operator >(Version other) => compareTo(other) > 0;

  @override
  bool operator ==(Object other) => other is Version && compareTo(other) == 0;

  @override
  int get hashCode => Object.hash(major, minor, patch);

  @override
  String toString() => '$major.$minor.$patch';
}

/// One file of a release.
class ReleaseAsset {
  const ReleaseAsset({
    required this.name,
    required this.size,
    required this.sha256,
  });

  final String name;
  final int size;

  /// Lower-case hex.
  final String sha256;

  static final _hash = RegExp(r'^[0-9a-f]{64}$');

  // A name becomes a path segment of the download URL and a file name on
  // disk, so it may hold nothing that could leave either.
  static final _name = RegExp(r'^[A-Za-z0-9][A-Za-z0-9._-]*$');

  static ReleaseAsset fromJson(Map<String, dynamic> json) {
    final name = json['name'];
    final size = json['size'];
    final sha256 = json['sha256'];
    if (name is! String || size is! int || sha256 is! String) {
      throw const FormatException('An asset is missing its name, size or hash');
    }
    if (!_name.hasMatch(name) || name.contains('..')) {
      throw FormatException('Asset name $name is not a plain file name');
    }
    if (!_hash.hasMatch(sha256) || size <= 0) {
      throw FormatException('Asset $name has a malformed size or hash');
    }
    return ReleaseAsset(name: name, size: size, sha256: sha256);
  }
}

/// A release's manifest, trusted only once its signature has been checked.
class ReleaseManifest {
  const ReleaseManifest({required this.version, required this.assets});

  final Version version;
  final List<ReleaseAsset> assets;

  static ReleaseManifest parse(String text) {
    final json = jsonDecode(text);
    if (json is! Map<String, dynamic>) {
      throw const FormatException('The manifest is not an object');
    }
    final version = Version.tryParse('${json['version']}');
    final assets = json['assets'];
    if (version == null || assets is! List) {
      throw const FormatException('The manifest has no version or assets');
    }
    return ReleaseManifest(
      version: version,
      assets: [
        for (final asset in assets)
          if (asset is Map<String, dynamic>) ReleaseAsset.fromJson(asset),
      ],
    );
  }

  ReleaseAsset? named(String name) {
    for (final asset in assets) {
      if (asset.name == name) return asset;
    }
    return null;
  }
}

/// The newest release on GitHub, before its manifest is read.
class PublishedRelease {
  const PublishedRelease({
    required this.version,
    required this.page,
    required this.files,
  });

  final Version version;

  /// The release's page, for its notes.
  final Uri page;

  /// Its files' download URLs, by name.
  final Map<String, Uri> files;

  static final _tag = RegExp(r'^v(\d+\.\d+\.\d+)$');

  /// The newest published release in a page of GitHub's release listing, or
  /// null when it has none. Drafts and prereleases are not updates.
  static PublishedRelease? newest(List<dynamic> releases) {
    PublishedRelease? best;
    for (final release in releases) {
      if (release is! Map<String, dynamic>) continue;
      if (release['draft'] == true || release['prerelease'] == true) continue;
      final tag = _tag.firstMatch('${release['tag_name']}');
      final version = tag == null ? null : Version.tryParse(tag[1]!);
      final page = Uri.tryParse('${release['html_url']}');
      if (version == null || page == null) continue;
      if (best != null && !(version > best.version)) continue;
      final files = <String, Uri>{};
      for (final asset in release['assets'] as List<dynamic>? ?? const []) {
        if (asset is! Map<String, dynamic>) continue;
        final url = Uri.tryParse('${asset['browser_download_url']}');
        if (url != null && url.scheme == 'https') {
          files['${asset['name']}'] = url;
        }
      }
      best = PublishedRelease(version: version, page: page, files: files);
    }
    return best;
  }
}

/// How an update reaches this installation.
enum UpdateMethod {
  /// Run the installer quietly once the app has quit, then reopen it
  /// (Windows, installed with Setup).
  installer,

  /// Put the disk image's app in place of this one once the app has quit,
  /// then reopen it (macOS, from a copy the user may replace).
  replaceApp,

  /// Save the file to Downloads and show it: unpack the portable archive,
  /// or drag the app out of the disk image.
  saveDownload,
}

/// What this installation is, as far as choosing an update goes.
class UpdateHost {
  const UpdateHost({
    required this.os,
    required this.arch,
    this.windowsInstalled = false,
    this.appBundle,
  });

  /// `windows` or `macos`, as [Platform.operatingSystem] names it.
  final String os;

  /// `x64` or `arm64`, as Dart names the processor.
  final String arch;

  /// Whether the Windows app was set up by the installer rather than
  /// unpacked from the portable archive.
  final bool windowsInstalled;

  /// The `.app` the Mac app runs from, when an update may replace it.
  final String? appBundle;
}

/// The file to fetch and how to apply it.
class UpdatePlan {
  const UpdatePlan(this.asset, this.method);

  final ReleaseAsset asset;
  final UpdateMethod method;
}

/// The release file meant for [host]'s platform, by the names the release
/// workflow gives them.
String? packageFor(UpdateHost host) => switch (host.os) {
  'windows' when host.arch == 'x64' =>
    host.windowsInstalled
        ? 'Isthmus-windows-x86_64-setup.exe'
        : 'Isthmus-windows-x86_64.zip',
  'macos' when host.arch == 'arm64' => 'Isthmus-macos-arm64.dmg',
  'macos' when host.arch == 'x64' => 'Isthmus-macos-x86_64.dmg',
  _ => null,
};

/// The file of [manifest] meant for [host] and how to apply it, or null when
/// the release has none for it.
UpdatePlan? planUpdate(ReleaseManifest manifest, UpdateHost host) {
  final name = packageFor(host);
  final asset = name == null ? null : manifest.named(name);
  if (asset == null) return null;
  final method = switch (host.os) {
    'windows' when host.windowsInstalled => UpdateMethod.installer,
    'macos' when host.appBundle != null => UpdateMethod.replaceApp,
    _ => UpdateMethod.saveDownload,
  };
  return UpdatePlan(asset, method);
}
