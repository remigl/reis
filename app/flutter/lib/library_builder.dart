// Building a Reis library on the device, from the user's own sounds.
//
// main isolate : lists / decodes audio through the native "reis/audio"
//                channel (Android MediaStore + MediaCodec: every format the
//                phone can play) and sends PCM to the worker
// worker isolate: owns the Rust ReisLibraryBuilder (fingerprints, embeddings)
//                 so the UI never freezes, then writes the .reis file.
import 'dart:async';
import 'dart:io';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:flutter/services.dart';

import 'reis_ffi.dart';

const _audio = MethodChannel('reis/audio');

/// What the library should recognise.
enum LibraryKind {
  music, // exact engine: the same recording, its position, synced lyrics
  sounds, // semantic engine: the same kind of sound (another dog, another doorbell)
  both,
}

extension LibraryKindX on LibraryKind {
  List<String> get engines => switch (this) {
        LibraryKind.music => ['exact'],
        LibraryKind.sounds => ['semantic'],
        LibraryKind.both => ['exact', 'semantic'],
      };
  bool get needsModel => this != LibraryKind.music;
}

/// One audio file to index.
class SoundSource {
  final String source; // content:// uri (Android) or file path
  final String? path; // file path when known (sidecar .lrc lookup)
  final String name;
  final String label; // category = folder
  final String? title, artist, album;
  final int durationMs;
  String? lyrics; // LRC text when available
  SoundSource({
    required this.source,
    this.path,
    required this.name,
    required this.label,
    this.title,
    this.artist,
    this.album,
    this.durationMs = 0,
    this.lyrics,
  });

  Map<String, Object?> info() => {
        'name': name,
        'label': label,
        'file': (path ?? source).split('/').last,
        'meta': {
          if (title != null && title!.isNotEmpty) 'title': title,
          if (artist != null && artist!.isNotEmpty) 'artist': artist,
          if (album != null && album!.isNotEmpty) 'album': album,
        },
        if (lyrics != null) 'lyrics': lyrics,
      };
}

/// The native decoder exists on Android only; elsewhere the core decodes
/// files itself (WAV, or anything with ffmpeg installed).
bool get hasNativeDecoder => Platform.isAndroid;

Future<bool> requestAudioPermission() async {
  if (!Platform.isAndroid) return true;
  return await _audio.invokeMethod<bool>('requestPermission') ?? false;
}

/// Every audio file of the phone, grouped by folder (folder name = category).
Future<Map<String, List<SoundSource>>> listPhoneAudio() async {
  final raw = await _audio.invokeListMethod<Map>('listAudio') ?? const [];
  final byFolder = <String, List<SoundSource>>{};
  for (final m in raw) {
    final folder = (m['folder'] as String? ?? '').replaceAll('\\', '/');
    final label = folder.isEmpty ? 'sons' : folder.split('/').where((s) => s.isNotEmpty).last;
    final title = m['title'] as String?;
    final artist = m['artist'] as String?;
    final file = m['fileName'] as String? ?? '';
    final name = (artist != null && title != null) ? '$artist - $title' : (title ?? file);
    byFolder.putIfAbsent(folder, () => []).add(SoundSource(
          source: m['source'] as String,
          path: m['path'] as String?,
          name: name,
          label: label,
          title: title,
          artist: artist,
          album: m['album'] as String?,
          durationMs: (m['durationMs'] as num?)?.toInt() ?? 0,
        ));
  }
  return byFolder;
}

class BuildProgress {
  final int done, total, failed;
  final String current;
  final Duration elapsed;
  const BuildProgress(this.done, this.total, this.failed, this.current, this.elapsed);
  Duration? get remaining => done == 0 ? null : elapsed * ((total - done) / done);
}

class BuildResult {
  final String path;
  final int added, failed;
  final List<String> errors;
  const BuildResult(this.path, this.added, this.failed, this.errors);
}

/// Builds a library and reports progress. Cancel with [cancel].
class LibraryBuildJob {
  final List<SoundSource> sources;
  final LibraryKind kind;
  final String outPath;
  final bool augmentSpeaker;
  final _progress = StreamController<BuildProgress>.broadcast();
  bool _cancelled = false;

  LibraryBuildJob({required this.sources, required this.kind, required this.outPath, this.augmentSpeaker = false});

  Stream<BuildProgress> get progress => _progress.stream;
  void cancel() => _cancelled = true;

  Future<BuildResult> run() async {
    Uint8List? model;
    if (kind.needsModel) {
      final d = await rootBundle.load('assets/models/mn10_as.rnm');
      model = d.buffer.asUint8List(d.offsetInBytes, d.lengthInBytes);
    }
    final config = '{"engines":[${kind.engines.map((e) => '"$e"').join(',')}]'
        '${augmentSpeaker ? ',"augment":["speaker"]' : ''}}';
    final fromWorker = ReceivePort();
    final iso = await Isolate.spawn(_builderWorker, fromWorker.sendPort);
    final replies = StreamIterator(fromWorker);
    Future<dynamic> next() async => (await replies.moveNext()) ? replies.current : null;
    final toWorker = await next() as SendPort;
    toWorker.send(['init', config, model == null ? null : TransferableTypedData.fromList([model])]);
    final init = await next();
    if (init is String) {
      iso.kill();
      fromWorker.close();
      throw StateError(init);
    }
    final sw = Stopwatch()..start();
    var done = 0, failed = 0;
    final errors = <String>[];
    for (final s in sources) {
      if (_cancelled) break;
      _progress.add(BuildProgress(done, sources.length, failed, s.name, sw.elapsed));
      try {
        Uint8List? art;
        if (hasNativeDecoder) {
          s.lyrics ??= await _audio.invokeMethod<String>('readSidecar', {'path': s.path, 'ext': 'lrc'});
          if (kind != LibraryKind.sounds) {
            art = await _audio.invokeMethod<Uint8List>('thumbnail', {'source': s.source, 'size': 160});
          }
          final d = await _audio.invokeMapMethod<String, dynamic>('decode', {'source': s.source, 'maxSeconds': 900.0});
          final pcm = d!['pcm'] as Uint8List;
          toWorker.send(['pcm', TransferableTypedData.fromList([pcm]), d['rate'] as int, s.info(), art]);
        } else {
          final lrc = File('${s.source.substring(0, s.source.lastIndexOf('.'))}.lrc');
          if (s.lyrics == null && lrc.existsSync()) s.lyrics = lrc.readAsStringSync();
          toWorker.send(['file', s.source, s.info(), art]);
        }
        final r = await next();
        if (r is String) {
          failed++;
          errors.add('${s.name}: $r');
        }
      } catch (e) {
        failed++;
        errors.add('${s.name}: $e');
      }
      done++;
    }
    _progress.add(BuildProgress(done, sources.length, failed, 'écriture de la bibliothèque…', sw.elapsed));
    toWorker.send(['finish', outPath]);
    final fin = await next();
    iso.kill();
    fromWorker.close();
    await _progress.close();
    if (fin is String) throw StateError(fin);
    return BuildResult(outPath, done - failed, failed, errors);
  }
}

void _builderWorker(SendPort out) {
  final inbox = ReceivePort();
  out.send(inbox.sendPort);
  ReisLibraryBuilder? b;
  inbox.listen((m) {
    final cmd = m[0] as String;
    try {
      switch (cmd) {
        case 'init':
          final model = m[2] == null ? null : (m[2] as TransferableTypedData).materialize().asUint8List();
          b = ReisLibraryBuilder.create(m[1] as String, model);
          out.send(true);
        case 'pcm':
          final bytes = (m[1] as TransferableTypedData).materialize().asUint8List();
          final pcm = Uint8List.fromList(bytes).buffer.asInt16List();
          b!.addPcm(pcm, m[2] as int, Map<String, Object?>.from(m[3] as Map), m[4] as Uint8List?);
          out.send(true);
        case 'file':
          b!.addFile(m[1] as String, Map<String, Object?>.from(m[2] as Map), m[3] as Uint8List?);
          out.send(true);
        case 'finish':
          b!.finish(m[1] as String);
          b = null;
          out.send(true);
      }
    } catch (e) {
      out.send(e is StateError ? e.message : e.toString());
    }
  });
}
