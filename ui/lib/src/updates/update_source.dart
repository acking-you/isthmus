/// Where updates come from: GitHub's release listing, a release's signed
/// manifest, and its files.
library;

import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:crypto/crypto.dart';
import 'package:isthmus_ui/src/ffi/isthmus_api.dart';
import 'package:isthmus_ui/src/updates/release.dart';

/// The user cancelled a download.
class UpdateCancelled implements Exception {
  const UpdateCancelled();
}

/// The network, behind an interface so tests need none.
abstract interface class UpdateHttp {
  /// The body at [url], refusing one larger than [limit] bytes.
  Future<List<int>> get(Uri url, {int limit = 1 << 20});

  /// Stream [url] into [target] and return the SHA-256 (lower-case hex) of
  /// what was written, once all of it is. Fails with [UpdateFailure.checksum]
  /// the moment more than [size] bytes arrive, or when fewer than [size] did,
  /// and with [UpdateFailure.install] when the file could not be written.
  Future<String> download(
    Uri url,
    File target, {
    required int size,
    required void Function(int received) onProgress,
  });

  /// Stop a [download] in flight; it fails with [UpdateCancelled].
  void cancel();
}

/// [UpdateHttp] over `dart:io`, which follows GitHub's redirects to its
/// download host on its own.
class IoUpdateHttp implements UpdateHttp {
  HttpClient? _active;
  bool _cancelled = false;

  HttpClient _client() => HttpClient()
    ..connectionTimeout = const Duration(seconds: 15)
    ..userAgent = 'isthmus-updater';

  Future<HttpClientResponse> _open(HttpClient client, Uri url) async {
    final request = await client.getUrl(url);
    request.headers.set(
      HttpHeaders.acceptHeader,
      url.host == 'api.github.com' ? 'application/vnd.github+json' : '*/*',
    );
    final response = await request.close().timeout(const Duration(seconds: 30));
    if (response.statusCode != HttpStatus.ok) {
      await response.drain<void>().catchError((_) {});
      throw UpdateException(
        UpdateFailure.network,
        'HTTP ${response.statusCode} from $url',
      );
    }
    return response;
  }

  @override
  Future<List<int>> get(Uri url, {int limit = 1 << 20}) async {
    final client = _client();
    try {
      final response = await _open(client, url);
      final body = BytesBuilder(copy: false);
      await for (final chunk in response.timeout(const Duration(seconds: 30))) {
        body.add(chunk);
        if (body.length > limit) {
          throw UpdateException(
            UpdateFailure.network,
            '$url is larger than $limit bytes',
          );
        }
      }
      return body.takeBytes();
    } on UpdateException {
      rethrow;
    } catch (error) {
      throw UpdateException(UpdateFailure.network, '$error');
    } finally {
      client.close(force: true);
    }
  }

  @override
  Future<String> download(
    Uri url,
    File target, {
    required int size,
    required void Function(int received) onProgress,
  }) async {
    _cancelled = false;
    final client = _active = _client();
    final digest = _DigestSink();
    final hasher = sha256.startChunkedConversion(digest);
    var received = 0;
    RandomAccessFile? file;
    try {
      // The hash describes the bytes received; it vouches for the file only
      // if every one of them was written. So the file is opened up front
      // and each write awaited: a disk that refuses it (no space) fails the
      // download here, not the install later with a truncated file.
      file = await target.open(mode: FileMode.write);
      final response = await _open(client, url);
      await for (final chunk in response.timeout(const Duration(seconds: 60))) {
        if (_cancelled) throw const UpdateCancelled();
        received += chunk.length;
        if (received > size) {
          throw const UpdateException(
            UpdateFailure.checksum,
            'The download is larger than the release lists',
          );
        }
        hasher.add(chunk);
        await file.writeFrom(chunk);
        onProgress(received);
      }
      if (received != size) {
        throw UpdateException(
          UpdateFailure.checksum,
          'Received $received of $size bytes',
        );
      }
      hasher.close();
      await file.flush();
      final written = file;
      file = null;
      await written.close();
      return '${digest.value}';
    } on UpdateException {
      rethrow;
    } on UpdateCancelled {
      rethrow;
    } on FileSystemException catch (error) {
      throw UpdateException(UpdateFailure.install, '$error');
    } catch (error) {
      if (_cancelled) throw const UpdateCancelled();
      throw UpdateException(UpdateFailure.network, '$error');
    } finally {
      _active = null;
      // Only on the way out of a failure, which is the one reported.
      await file?.close().catchError((_) {});
      client.close(force: true);
    }
  }

  @override
  void cancel() {
    _cancelled = true;
    _active?.close(force: true);
  }
}

class _DigestSink implements Sink<Digest> {
  Digest? value;

  @override
  void add(Digest data) => value = data;

  @override
  void close() {}
}

/// The releases on GitHub, and the trust check between them and the app.
class UpdateSource {
  UpdateSource({
    required this.api,
    UpdateHttp? http,
    this.publicKey = updatePublicKey,
  }) : http = http ?? IoUpdateHttp();

  /// The library, which checks the signature (`update_verify`).
  final IsthmusApiClient api;
  final UpdateHttp http;
  final String publicKey;

  static final Uri releasesUrl = Uri.https(
    'api.github.com',
    '/repos/$updateRepository/releases',
    {'per_page': '30'},
  );

  /// The newest published release, or null when there is none.
  Future<PublishedRelease?> newest() async {
    final body = await http.get(releasesUrl, limit: 8 << 20);
    final Object? listing;
    try {
      listing = jsonDecode(utf8.decode(body));
    } on FormatException catch (error) {
      throw UpdateException(UpdateFailure.network, error.message);
    }
    if (listing is! List) {
      throw const UpdateException(
        UpdateFailure.network,
        'GitHub did not answer with a list of releases',
      );
    }
    return PublishedRelease.newest(listing);
  }

  /// [release]'s manifest, once its signature is shown to be the release
  /// key's and it is shown to describe [release] itself.
  Future<ReleaseManifest> manifest(PublishedRelease release) async {
    final manifestUrl = release.files[manifestName];
    final signatureUrl = release.files[signatureName];
    if (manifestUrl == null || signatureUrl == null) {
      throw const UpdateException(UpdateFailure.unsigned);
    }
    final manifest = await http.get(manifestUrl);
    final signatureFile = await http.get(signatureUrl, limit: 4096);
    final List<int> signature;
    try {
      signature = base64Decode(utf8.decode(signatureFile).trim());
    } on FormatException catch (error) {
      throw UpdateException(UpdateFailure.signature, error.message);
    }
    final bool valid;
    try {
      valid = await api.verifySignature(
        message: manifest,
        signature: signature,
        publicKey: base64Decode(publicKey),
      );
    } on IsthmusCallException catch (error) {
      throw UpdateException(UpdateFailure.signature, error.message);
    }
    if (!valid) {
      throw const UpdateException(
        UpdateFailure.signature,
        'The manifest is not signed by the release key',
      );
    }
    final ReleaseManifest parsed;
    try {
      parsed = ReleaseManifest.parse(utf8.decode(manifest));
    } on FormatException catch (error) {
      throw UpdateException(UpdateFailure.signature, error.message);
    }
    // A genuinely signed manifest of an older release must not stand in for
    // this one's.
    if (parsed.version != release.version) {
      throw UpdateException(
        UpdateFailure.signature,
        'The manifest describes ${parsed.version}, the release is '
        '${release.version}',
      );
    }
    return parsed;
  }
}
