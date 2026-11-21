// Optional online lyrics from LRCLIB (https://lrclib.net), the only network
// feature of Reis. Only the title / artist / duration of a song are sent.
//
//  * guessTrack   : title + artist from tags, or deduced from the file name
//  * LrcLib       : /api/get (best match, ±2 s on duration) and /api/search
//  * LyricsStore  : lyrics chosen in the app, kept next to the library
import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;

import 'package:path_provider/path_provider.dart';

import 'reis_ffi.dart';

const _userAgent = 'Reis/0.4.0 (https://github.com/remigl/reis)';

class TrackGuess {
  final String title;
  final String? artist;
  const TrackGuess(this.title, this.artist);
  @override
  String toString() => artist == null ? title : '$artist — $title';
}

final _noise = RegExp(
    r'[\(\[\{][^\)\]\}]*(official|officiel|video|vidéo|clip|lyrics?|paroles|audio|hd|hq|4k|remaster(ed)?|visuali[sz]er|live|explicit|version|edit|mix|feat\.?|ft\.|featuring)[^\)\]\}]*[\)\]\}]',
    caseSensitive: false);
final _trackNo = RegExp(r'^\s*(\d{1,3}|[a-d]\d{1,2})\s*([\.\-_)]\s*|\s+)');
final _feat = RegExp(r'\s+(feat\.?|ft\.?|featuring)\s+.*$', caseSensitive: false);

/// Title and artist from tags, else from a file name such as
/// "03 - Daft Punk - One More Time (Official Video).mp3" or "Daft_Punk-One_More_Time".
TrackGuess guessTrack({String? title, String? artist, required String fileName}) {
  String clean(String s) => s.replaceAll(_noise, '').replaceAll(_feat, '').replaceAll(RegExp(r'\s+'), ' ').trim();
  final looksLikeRecording = RegExp(r'\d{6,}'); // AUD-20251102-WA0003, REC_20240101_1200
  final t = title?.trim() ?? '';
  final a = artist?.trim() ?? '';
  final tagTitleLooksLikeFile = t.isEmpty || t.toLowerCase() == fileName.toLowerCase() || RegExp(r'\.\w{2,4}$').hasMatch(t);
  if (!tagTitleLooksLikeFile && a.isNotEmpty) return TrackGuess(clean(t), clean(a));
  var n = (tagTitleLooksLikeFile ? fileName : t);
  if (n.contains('.')) n = n.substring(0, n.lastIndexOf('.'));
  n = n.replaceAll('_', ' ').replaceFirst(_trackNo, '');
  n = clean(n);
  if (looksLikeRecording.hasMatch(n)) return TrackGuess(n, a.isEmpty ? null : a);
  for (final sep in [' - ', ' – ', ' — ', ' -', '- ', '-']) {
    final i = n.indexOf(sep);
    if (i > 0) {
      var art = clean(n.substring(0, i));
      var tit = clean(n.substring(i + sep.length));
      // "Artist - 05 - Title"
      tit = tit.replaceFirst(_trackNo, '');
      if (art.isNotEmpty && tit.isNotEmpty) return TrackGuess(tit, a.isNotEmpty ? a : art);
    }
  }
  return TrackGuess(n, a.isEmpty ? null : a);
}

class LyricsCandidate {
  final int id;
  final String track, artist, album;
  final double duration;
  final String? synced, plain;
  final bool instrumental;
  const LyricsCandidate(this.id, this.track, this.artist, this.album, this.duration, this.synced, this.plain, this.instrumental);

  static LyricsCandidate fromJson(Map<String, dynamic> j) => LyricsCandidate(
        (j['id'] as num?)?.toInt() ?? 0,
        j['trackName'] as String? ?? j['name'] as String? ?? '',
        j['artistName'] as String? ?? '',
        j['albumName'] as String? ?? '',
        (j['duration'] as num?)?.toDouble() ?? 0,
        (j['syncedLyrics'] as String?)?.trim().isEmpty == true ? null : j['syncedLyrics'] as String?,
        (j['plainLyrics'] as String?)?.trim().isEmpty == true ? null : j['plainLyrics'] as String?,
        j['instrumental'] as bool? ?? false,
      );

  bool get isSynced => synced != null;
  bool get hasLyrics => synced != null || plain != null;

  /// LRC text (plain lyrics become un-timed lines).
  String get lrc => synced ?? plain ?? '';
}

class LrcLibException implements Exception {
  final String message;
  LrcLibException(this.message);
  @override
  String toString() => message;
}

/// Minimal LRCLIB client: sequential requests, pacing, Retry-After on 429.
class LrcLib {
  static final _client = HttpClient()
    ..connectionTimeout = const Duration(seconds: 10)
    ..userAgent = _userAgent;
  static DateTime _last = DateTime.fromMillisecondsSinceEpoch(0);
  static final _rnd = math.Random();

  static Future<dynamic> _getJson(String path, Map<String, String> q) async {
    for (var attempt = 0; attempt < 3; attempt++) {
      final wait = const Duration(milliseconds: 350) - DateTime.now().difference(_last);
      if (!wait.isNegative) await Future<void>.delayed(wait);
      _last = DateTime.now();
      final uri = Uri.https('lrclib.net', path, q);
      HttpClientResponse res;
      try {
        final req = await _client.getUrl(uri).timeout(const Duration(seconds: 15));
        req.headers.set('Lrclib-Client', _userAgent);
        res = await req.close().timeout(const Duration(seconds: 20));
      } on SocketException {
        throw LrcLibException('Pas de connexion Internet.');
      } on TimeoutException {
        throw LrcLibException('LRCLIB ne répond pas.');
      } on HandshakeException {
        throw LrcLibException('Connexion sécurisée impossible avec lrclib.net.');
      }
      final body = await res.transform(utf8.decoder).join();
      if (res.statusCode == 404) return null;
      if (res.statusCode == 429 || res.statusCode == 503) {
        final ra = int.tryParse(res.headers.value('retry-after') ?? '') ?? 5;
        await Future<void>.delayed(Duration(milliseconds: (ra * 1000 * (1 + 0.3 * _rnd.nextDouble())).round()));
        continue;
      }
      if (res.statusCode != 200) throw LrcLibException('LRCLIB : erreur ${res.statusCode}');
      if (!(res.headers.contentType?.mimeType.contains('json') ?? false)) throw LrcLibException('LRCLIB : réponse inattendue');
      return jsonDecode(body);
    }
    throw LrcLibException('LRCLIB est surchargé, réessaie plus tard.');
  }

  /// Best match for a song (null if none). Falls back to a search ranked by duration.
  static Future<LyricsCandidate?> best(TrackGuess g, {double? durationS, String? album}) async {
    if (g.artist != null && g.artist!.isNotEmpty) {
      final q = {
        'track_name': g.title,
        'artist_name': g.artist!,
        if (album != null && album.isNotEmpty) 'album_name': album,
        if (durationS != null && durationS >= 1 && durationS <= 3600) 'duration': durationS.round().toString(),
      };
      final j = await _getJson('/api/get', q);
      if (j is Map<String, dynamic>) {
        final c = LyricsCandidate.fromJson(j);
        if (c.hasLyrics) return c;
      }
    }
    final list = await search(g, durationS: durationS);
    for (final c in list) {
      if (c.hasLyrics && (durationS == null || (c.duration - durationS).abs() <= 5)) return c;
    }
    return null;
  }

  /// Candidates, synced first, then closest duration.
  static Future<List<LyricsCandidate>> search(TrackGuess g, {double? durationS, String? freeText}) async {
    final q = freeText != null && freeText.trim().isNotEmpty
        ? {'q': freeText.trim()}
        : {'track_name': g.title, if (g.artist != null && g.artist!.isNotEmpty) 'artist_name': g.artist!};
    var j = await _getJson('/api/search', q);
    if ((j is! List || j.isEmpty) && freeText == null) {
      j = await _getJson('/api/search', {'q': g.toString().replaceAll('—', ' ')});
    }
    if (j is! List) return [];
    final out = j.whereType<Map<String, dynamic>>().map(LyricsCandidate.fromJson).where((c) => !c.instrumental || c.hasLyrics).toList();
    double dd(LyricsCandidate c) => durationS == null ? 0 : (c.duration - durationS).abs();
    out.sort((a, b) {
      final close = (dd(a) <= 3 ? 0 : 1).compareTo(dd(b) <= 3 ? 0 : 1);
      if (close != 0) return close;
      final sync = (a.isSynced ? 0 : 1).compareTo(b.isSynced ? 0 : 1);
      if (sync != 0) return sync;
      return dd(a).compareTo(dd(b));
    });
    return out;
  }
}

// ------------------------------------------------------------------ parsing / storage

final _ts = RegExp(r'\[(\d{1,3}):(\d{1,2})(?:[\.:](\d{1,3}))?\]');

/// LRC → lines. Text without timestamps (plain lyrics) gives un-timed lines (tMs = -1).
List<LyricLine> parseLrc(String src) {
  final out = <LyricLine>[];
  var offset = 0;
  for (final raw in const LineSplitter().convert(src)) {
    final off = RegExp(r'^\[offset:\s*([+-]?\d+)\]', caseSensitive: false).firstMatch(raw.trim());
    if (off != null) {
      offset = int.parse(off.group(1)!);
      continue;
    }
    final ms = _ts.allMatches(raw).toList();
    final text = raw.replaceAll(_ts, '').replaceAll(RegExp(r'<\d{1,3}:\d{1,2}(?:[\.:]\d{1,3})?>'), '').trim();
    if (ms.isEmpty) {
      if (RegExp(r'^\[[a-z]+:').hasMatch(raw.trim())) continue; // [ar:…] tags
      out.add(LyricLine(-1, text, const []));
      continue;
    }
    for (final m in ms) {
      final frac = m.group(3) ?? '0';
      final f = int.parse(frac) * (frac.length == 1 ? 100 : frac.length == 2 ? 10 : 1);
      final t = int.parse(m.group(1)!) * 60000 + int.parse(m.group(2)!) * 1000 + f - offset;
      out.add(LyricLine(math.max(0, t), text, const []));
    }
  }
  if (out.any((l) => l.tMs >= 0)) {
    out.removeWhere((l) => l.tMs < 0);
    out.sort((a, b) => a.tMs.compareTo(b.tMs));
  }
  return out;
}

/// Lyrics chosen in the app, stored as JSON next to the libraries:
/// key = normalised artist|title|duration (survives library rebuilds).
class LyricsStore {
  final File _file;
  final Map<String, dynamic> _data;
  LyricsStore._(this._file, this._data);

  static Future<LyricsStore> open() async {
    final f = File('${(await getApplicationSupportDirectory()).path}/lyrics.json');
    Map<String, dynamic> d = {};
    try {
      if (await f.exists()) d = jsonDecode(await f.readAsString()) as Map<String, dynamic>;
    } catch (_) {}
    return LyricsStore._(f, d);
  }

  static String keyOf(SongInfo s) {
    String n(String v) => v.toLowerCase().replaceAll(RegExp(r'[^a-z0-9]+'), '');
    return '${n(s.artist)}|${n(s.title)}|${(s.durationMs / 1000).round()}';
  }

  String? lrcFor(SongInfo s) => (_data[keyOf(s)] as Map?)?['lrc'] as String?;

  Future<void> save(SongInfo s, LyricsCandidate c) async {
    _data[keyOf(s)] = {'lrc': c.lrc, 'source': 'lrclib', 'id': c.id, 'synced': c.isSynced};
    await _file.writeAsString(jsonEncode(_data), flush: true);
  }

  Future<void> remove(SongInfo s) async {
    _data.remove(keyOf(s));
    await _file.writeAsString(jsonEncode(_data), flush: true);
  }
}
