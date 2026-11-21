// dart:ffi bindings to the Rust core (core/src/ffi.rs). Keep in sync.
import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

final class ReisState extends Struct {
  @Uint32()
  external int phase; // 0 listening, 1 locked, 2 paused
  @Int32()
  external int song;
  @Double()
  external double positionMs;
  @Float()
  external double confidence;
  @Uint8()
  external int wantsAudio;
  @Uint8()
  external int uncertain;
  @Float()
  external double levelDbfs;
  @Uint32()
  external int lastScore;
  @Uint64()
  external int lastMatchUs;
}

DynamicLibrary _open() {
  if (Platform.isAndroid || Platform.isLinux) return DynamicLibrary.open('libreis_core.so');
  if (Platform.isWindows) return DynamicLibrary.open('reis_core.dll');
  if (Platform.isMacOS) return DynamicLibrary.open('libreis_core.dylib');
  return DynamicLibrary.process(); // iOS: statically linked
}

final DynamicLibrary _lib = _open();

final _catOpen = _lib.lookupFunction<Pointer<Void> Function(Pointer<Utf8>), Pointer<Void> Function(Pointer<Utf8>)>('reis_catalog_open');
final _catFree = _lib.lookupFunction<Void Function(Pointer<Void>), void Function(Pointer<Void>)>('reis_catalog_free');
final _songCount = _lib.lookupFunction<Uint32 Function(Pointer<Void>), int Function(Pointer<Void>)>('reis_catalog_song_count');
final _songStr = _lib.lookupFunction<Pointer<Uint8> Function(Pointer<Void>, Uint32, Uint32, Pointer<Uint32>),
    Pointer<Uint8> Function(Pointer<Void>, int, int, Pointer<Uint32>)>('reis_song_str');
final _songDur = _lib.lookupFunction<Uint32 Function(Pointer<Void>, Uint32), int Function(Pointer<Void>, int)>('reis_song_duration_ms');
final _songArt = _lib.lookupFunction<Pointer<Uint8> Function(Pointer<Void>, Uint32, Pointer<Uint32>),
    Pointer<Uint8> Function(Pointer<Void>, int, Pointer<Uint32>)>('reis_song_artwork');
final _lyrCount = _lib.lookupFunction<Uint32 Function(Pointer<Void>, Uint32), int Function(Pointer<Void>, int)>('reis_lyrics_count');
final _lyrLine = _lib.lookupFunction<Pointer<Uint8> Function(Pointer<Void>, Uint32, Uint32, Pointer<Uint32>, Pointer<Uint32>),
    Pointer<Uint8> Function(Pointer<Void>, int, int, Pointer<Uint32>, Pointer<Uint32>)>('reis_lyrics_line');
final _wordCount = _lib.lookupFunction<Uint32 Function(Pointer<Void>, Uint32, Uint32), int Function(Pointer<Void>, int, int)>('reis_lyrics_word_count');
final _word = _lib.lookupFunction<Uint8 Function(Pointer<Void>, Uint32, Uint32, Uint32, Pointer<Uint32>, Pointer<Uint32>),
    int Function(Pointer<Void>, int, int, int, Pointer<Uint32>, Pointer<Uint32>)>('reis_lyrics_word');
final _catHas = _lib.lookupFunction<Uint8 Function(Pointer<Void>, Uint32), int Function(Pointer<Void>, int)>('reis_catalog_has');
final _refLabel = _lib.lookupFunction<Pointer<Uint8> Function(Pointer<Void>, Uint32, Pointer<Uint32>),
    Pointer<Uint8> Function(Pointer<Void>, int, Pointer<Uint32>)>('reis_ref_label');
final _recNew = _lib.lookupFunction<Pointer<Void> Function(Pointer<Void>), Pointer<Void> Function(Pointer<Void>)>('reis_recognizer_new');
final _recFree = _lib.lookupFunction<Void Function(Pointer<Void>), void Function(Pointer<Void>)>('reis_recognizer_free');
final _recI16 = _lib.lookupFunction<Pointer<Utf8> Function(Pointer<Void>, Pointer<Int16>, Uint32, Uint32, Uint32),
    Pointer<Utf8> Function(Pointer<Void>, Pointer<Int16>, int, int, int)>('reis_recognize_i16');
final _strFree = _lib.lookupFunction<Void Function(Pointer<Utf8>), void Function(Pointer<Utf8>)>('reis_string_free');
final _engNew = _lib.lookupFunction<Pointer<Void> Function(Pointer<Void>), Pointer<Void> Function(Pointer<Void>)>('reis_engine_new');
final _engFree = _lib.lookupFunction<Void Function(Pointer<Void>), void Function(Pointer<Void>)>('reis_engine_free');
final _engReset = _lib.lookupFunction<Void Function(Pointer<Void>), void Function(Pointer<Void>)>('reis_engine_reset');
final _engPush = _lib.lookupFunction<Void Function(Pointer<Void>, Pointer<Int16>, Uint32, Uint32, Uint32, Double),
    void Function(Pointer<Void>, Pointer<Int16>, int, int, int, double)>('reis_engine_push_i16');
final _engTick = _lib.lookupFunction<Void Function(Pointer<Void>, Double, Pointer<ReisState>),
    void Function(Pointer<Void>, double, Pointer<ReisState>)>('reis_engine_tick');

class LyricWord {
  final int tMs;
  final int byteEnd;
  const LyricWord(this.tMs, this.byteEnd);
}

class LyricLine {
  final int tMs;
  final String text;
  final List<LyricWord> words;
  const LyricLine(this.tMs, this.text, this.words);
}

class SongInfo {
  final int id;
  final String title, artist, album;
  final int durationMs;
  final Uint8List? artwork;
  final List<LyricLine> lyrics;
  const SongInfo(this.id, this.title, this.artist, this.album, this.durationMs, this.artwork, this.lyrics);
}

/// Read-only catalog handle (memory-mapped by the core; cheap to open twice).
class ReisCatalog {
  final Pointer<Void> ptr;
  ReisCatalog._(this.ptr);

  static ReisCatalog? open(String path) {
    final p = path.toNativeUtf8();
    final c = _catOpen(p);
    malloc.free(p);
    return c == nullptr ? null : ReisCatalog._(c);
  }

  int get songCount => _songCount(ptr);

  /// Library built with the exact engine (same recording + position + lyrics).
  bool get hasExact => _catHas(ptr, 0) == 1;

  /// Library built with the semantic engine (same kind of sound).
  bool get hasSemantic => _catHas(ptr, 1) == 1;

  String label(int id) => using((arena) {
        final len = arena<Uint32>();
        return _str(_refLabel(ptr, id, len), len.value);
      });

  String _str(Pointer<Uint8> p, int len) => len == 0 ? '' : utf8.decode(p.asTypedList(len), allowMalformed: true);

  SongInfo song(int id) {
    return using((arena) {
      final len = arena<Uint32>();
      final t = arena<Uint32>();
      String s(int f) => _str(_songStr(ptr, id, f, len), len.value);
      final title = s(0), artist = s(1), album = s(2);
      final ap = _songArt(ptr, id, len);
      final art = ap == nullptr ? null : Uint8List.fromList(ap.asTypedList(len.value));
      final lines = <LyricLine>[];
      final n = _lyrCount(ptr, id);
      final be = arena<Uint32>();
      for (var i = 0; i < n; i++) {
        final txt = _str(_lyrLine(ptr, id, i, t, len), len.value);
        final lt = t.value;
        final words = <LyricWord>[];
        final nw = _wordCount(ptr, id, i);
        for (var j = 0; j < nw; j++) {
          if (_word(ptr, id, i, j, t, be) == 1) words.add(LyricWord(t.value, be.value));
        }
        lines.add(LyricLine(lt, txt, words));
      }
      return SongInfo(id, title, artist, album, _songDur(ptr, id), art, lines);
    });
  }

  void dispose() => _catFree(ptr);
}

class EngineSnapshot {
  final int phase;
  final int song;
  final double positionMs;
  final double confidence;
  final bool wantsAudio;
  final bool uncertain;
  final double levelDbfs;
  const EngineSnapshot(this.phase, this.song, this.positionMs, this.confidence, this.wantsAudio, this.uncertain, this.levelDbfs);

  List<Object> toMessage() => [phase, song, positionMs, confidence, wantsAudio, uncertain, levelDbfs];
  static EngineSnapshot fromMessage(List m) =>
      EngineSnapshot(m[0] as int, m[1] as int, m[2] as double, m[3] as double, m[4] as bool, m[5] as bool, m[6] as double);
}

/// One-shot recogniser (all engines + general tags). Returns the JSON
/// described in docs/API.md. Owned by the worker isolate.
class ReisRecognizer {
  final Pointer<Void> ptr;
  Pointer<Int16> _buf = calloc<Int16>(1);
  int _cap = 1;

  ReisRecognizer._(this.ptr);

  static ReisRecognizer? create(ReisCatalog cat) {
    final p = _recNew(cat.ptr);
    return p == nullptr ? null : ReisRecognizer._(p);
  }

  /// [samples]: mono 16-bit PCM.
  String recognize(Int16List samples, int rate) {
    if (samples.length > _cap) {
      calloc.free(_buf);
      _cap = samples.length;
      _buf = calloc<Int16>(_cap);
    }
    _buf.asTypedList(samples.length).setAll(0, samples);
    final out = _recI16(ptr, _buf, samples.length, 1, rate);
    final txt = out.toDartString();
    _strFree(out);
    return txt;
  }

  void dispose() {
    _recFree(ptr);
    calloc.free(_buf);
  }
}

/// Real-time exact engine. Owned by the worker isolate (see engine_service.dart).
class ReisEngine {
  final Pointer<Void> ptr;
  final Pointer<ReisState> _st = calloc<ReisState>();
  Pointer<Int16> _buf = calloc<Int16>(4096);
  int _cap = 4096;

  ReisEngine._(this.ptr);

  /// Null when the library has no exact engine.
  static ReisEngine? create(ReisCatalog cat) {
    final p = _engNew(cat.ptr);
    return p == nullptr ? null : ReisEngine._(p);
  }

  void pushPcm16(Uint8List bytes, int rate, int channels, double clockEndMs) {
    final n = bytes.lengthInBytes ~/ 2;
    if (n > _cap) {
      calloc.free(_buf);
      _cap = n;
      _buf = calloc<Int16>(_cap);
    }
    _buf.cast<Uint8>().asTypedList(n * 2).setAll(0, bytes.sublist(0, n * 2)); // copy bytes: no alignment assumption
    _engPush(ptr, _buf, n ~/ channels, channels, rate, clockEndMs);
  }

  EngineSnapshot tick(double nowMs) {
    _engTick(ptr, nowMs, _st);
    final s = _st.ref;
    return EngineSnapshot(s.phase, s.song, s.positionMs, s.confidence, s.wantsAudio != 0, s.uncertain != 0, s.levelDbfs);
  }

  void reset() => _engReset(ptr);

  void dispose() {
    _engFree(ptr);
    calloc.free(_st);
    calloc.free(_buf);
  }
}

// ------------------------------------------------------------------ building a library on the device

final _bNew = _lib.lookupFunction<Pointer<Void> Function(Pointer<Utf8>, Pointer<Uint8>, IntPtr),
    Pointer<Void> Function(Pointer<Utf8>, Pointer<Uint8>, int)>('reis_builder_new');
final _bAddI16 = _lib.lookupFunction<
    Int32 Function(Pointer<Void>, Pointer<Int16>, Uint32, Uint32, Uint32, Pointer<Utf8>, Pointer<Uint8>, IntPtr),
    int Function(Pointer<Void>, Pointer<Int16>, int, int, int, Pointer<Utf8>, Pointer<Uint8>, int)>('reis_builder_add_i16');
final _bAddFile = _lib.lookupFunction<Int32 Function(Pointer<Void>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Uint8>, IntPtr),
    int Function(Pointer<Void>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Uint8>, int)>('reis_builder_add_file');
final _bCount = _lib.lookupFunction<Uint32 Function(Pointer<Void>), int Function(Pointer<Void>)>('reis_builder_count');
final _bFinish = _lib.lookupFunction<Int32 Function(Pointer<Void>, Pointer<Utf8>), int Function(Pointer<Void>, Pointer<Utf8>)>('reis_builder_finish');
final _bFree = _lib.lookupFunction<Void Function(Pointer<Void>), void Function(Pointer<Void>)>('reis_builder_free');
final _lastError = _lib.lookupFunction<Pointer<Utf8> Function(), Pointer<Utf8> Function()>('reis_last_error');

String reisLastError() {
  final p = _lastError();
  final s = p.toDartString();
  _strFree(p);
  return s;
}

/// Creates a .reis library from audio decoded by the host. Not thread-safe:
/// use it from a single (worker) isolate.
class ReisLibraryBuilder {
  Pointer<Void> _ptr;
  ReisLibraryBuilder._(this._ptr);

  /// [configJson]: same keys as reis.json, e.g. {"engines":["exact","semantic"]}.
  /// [model]: .rnm bytes, required for the semantic engine.
  static ReisLibraryBuilder create(String configJson, Uint8List? model) {
    final c = configJson.toNativeUtf8();
    final m = model == null ? nullptr : malloc<Uint8>(model.length);
    if (model != null) m.asTypedList(model.length).setAll(0, model);
    final p = _bNew(c, m, model?.length ?? 0);
    malloc.free(c);
    if (model != null) malloc.free(m);
    if (p == nullptr) throw StateError(reisLastError());
    return ReisLibraryBuilder._(p);
  }

  int get count => _bCount(_ptr);

  /// Mono 16-bit PCM. Returns the reference id; throws on error.
  int addPcm(Int16List pcm, int rate, Map<String, Object?> info, Uint8List? art) {
    final buf = malloc<Int16>(pcm.length);
    buf.asTypedList(pcm.length).setAll(0, pcm);
    final j = jsonEncode(info).toNativeUtf8();
    final a = art == null ? nullptr : malloc<Uint8>(art.length);
    if (art != null) a.asTypedList(art.length).setAll(0, art);
    final id = _bAddI16(_ptr, buf, pcm.length, 1, rate, j, a, art?.length ?? 0);
    malloc.free(buf);
    malloc.free(j);
    if (art != null) malloc.free(a);
    if (id < 0) throw StateError(reisLastError());
    return id;
  }

  /// File decoded by the core (WAV natively, other formats need ffmpeg: desktop).
  int addFile(String path, Map<String, Object?> info, Uint8List? art) {
    final p = path.toNativeUtf8();
    final j = jsonEncode(info).toNativeUtf8();
    final a = art == null ? nullptr : malloc<Uint8>(art.length);
    if (art != null) a.asTypedList(art.length).setAll(0, art);
    final id = _bAddFile(_ptr, p, j, a, art?.length ?? 0);
    malloc.free(p);
    malloc.free(j);
    if (art != null) malloc.free(a);
    if (id < 0) throw StateError(reisLastError());
    return id;
  }

  /// Writes the library and releases the builder.
  void finish(String outPath) {
    final o = outPath.toNativeUtf8();
    final r = _bFinish(_ptr, o);
    malloc.free(o);
    _ptr = nullptr;
    if (r != 0) throw StateError(reisLastError());
  }

  void dispose() {
    if (_ptr != nullptr) _bFree(_ptr);
    _ptr = nullptr;
  }
}
