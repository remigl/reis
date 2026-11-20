// Microphone + recognition loop. The Rust engine runs in a worker isolate so
// fingerprinting never blocks the UI thread; the microphone is only open when
// the engine asks for audio (`wantsAudio`), which is a small fraction of the
// time once a song is locked.
import 'dart:async';
import 'dart:convert';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:flutter/foundation.dart';
import 'package:record/record.dart';

import 'reis_ffi.dart';

const int kMicRate = 16000;

class EngineService extends ChangeNotifier {
  final String catalogPath;
  final AudioRecorder _rec = AudioRecorder();
  final Stopwatch _clock = Stopwatch()..start();
  SendPort? _toWorker;
  Isolate? _worker;
  StreamSubscription<Uint8List>? _micSub;
  bool _micOn = false;
  bool _starting = false;
  bool permissionDenied = false;

  EngineSnapshot state = const EngineSnapshot(0, -1, 0, 0, true, false, -120);
  Detection? detection;
  double stateClockMs = 0; // when `state` was produced, for smooth interpolation

  EngineService(this.catalogPath);

  double nowMs() => _clock.elapsedMicroseconds / 1000.0;

  /// Position extrapolated to "now" (60 fps smooth even though ticks are 20 Hz).
  double get positionMs {
    if (state.song < 0 || state.phase == 2) return state.positionMs;
    return state.positionMs + (nowMs() - stateClockMs);
  }

  Future<void> start() async {
    final rp = ReceivePort();
    _worker = await Isolate.spawn(_workerMain, [rp.sendPort, catalogPath]);
    rp.listen((m) {
      if (m is SendPort) {
        _toWorker = m;
      } else if (m is List && m.isNotEmpty && m[0] == 'detect') {
        final txt = m[1] as String;
        detection = txt.isEmpty ? null : Detection.fromJson(jsonDecode(txt) as Map<String, dynamic>);
        notifyListeners();
      } else if (m is List) {
        state = EngineSnapshot.fromMessage(m);
        stateClockMs = m[7] as double; // clock of the tick that produced it
        _syncMic();
        notifyListeners();
      }
    });
    Timer.periodic(const Duration(milliseconds: 50), (_) => _toWorker?.send(['tick', nowMs()]));
    await _syncMic();
  }

  Future<void> _syncMic() async {
    final want = state.wantsAudio;
    if (want && !_micOn && !_starting) {
      _starting = true;
      try {
        if (!await _rec.hasPermission()) {
          permissionDenied = true;
          notifyListeners();
          return;
        }
        permissionDenied = false;
        final stream = await _rec.startStream(const RecordConfig(
          encoder: AudioEncoder.pcm16bits,
          sampleRate: kMicRate,
          numChannels: 1,
          autoGain: false, // AGC pumps the spectrum; the core is gain-invariant anyway
          echoCancel: false,
          noiseSuppress: false, // suppressors remove exactly the music we want to hear
        ));
        _micOn = true;
        _micSub = stream.listen((chunk) {
          _toWorker?.send(['pcm', TransferableTypedData.fromList([chunk]), nowMs()]);
        });
      } finally {
        _starting = false;
      }
    } else if (!want && _micOn) {
      _micOn = false;
      await _micSub?.cancel();
      await _rec.stop();
    }
  }

  void restart() => _toWorker?.send(['reset']);

  @override
  void dispose() {
    _micSub?.cancel();
    _rec.dispose();
    _worker?.kill();
    super.dispose();
  }
}

/// What the semantic engine currently hears (libraries of sounds, not only music).
class Detection {
  final String kind; // similar | tag | exact | unknown
  final String label; // category, or AudioSet tag
  final double score;
  final List<(String, double)> candidates;
  const Detection(this.kind, this.label, this.score, this.candidates);

  static Detection fromJson(Map<String, dynamic> j) {
    final v = j['verdict'] as Map<String, dynamic>;
    final kind = v['kind'] as String;
    String label = '';
    double score = 0;
    if (kind == 'similar') {
      label = v['label'] as String;
      score = (v['score'] as num).toDouble();
    } else if (kind == 'tag') {
      label = v['label'] as String;
      score = (v['probability'] as num).toDouble();
    } else if (kind == 'exact') {
      label = (v['reference'] as Map<String, dynamic>)['name'] as String;
      score = 1;
    }
    final cands = <(String, double)>[
      for (final s in (j['similar'] as List? ?? const [])) ((s as Map)['label'] as String, (s['score'] as num).toDouble()),
    ];
    return Detection(kind, label, score, cands);
  }
}

void _workerMain(List args) {
  final SendPort out = args[0];
  final cat = ReisCatalog.open(args[1] as String)!;
  final eng = ReisEngine.create(cat); // null: library without the exact engine
  final rec = cat.hasSemantic ? ReisRecognizer.create(cat) : null;
  final inbox = ReceivePort();
  out.send(inbox.sendPort);
  var last = <Object>[];
  var lastNow = 0.0;
  // last 3 s of audio for the semantic engine
  final ring = Int16List(kMicRate * 3);
  var filled = 0;
  var nextDetect = 0.0;
  String lastDetection = '';
  inbox.listen((m) {
    final cmd = m[0] as String;
    if (cmd == 'pcm') {
      final bytes = (m[1] as TransferableTypedData).materialize().asUint8List();
      eng?.pushPcm16(bytes, kMicRate, 1, m[2] as double);
      if (rec != null) {
        final pcm = Int16List.fromList(Uint8List.fromList(bytes).buffer.asInt16List());
        if (pcm.length >= ring.length) {
          ring.setAll(0, pcm.sublist(pcm.length - ring.length));
          filled = ring.length;
        } else {
          ring.setRange(0, ring.length - pcm.length, ring, pcm.length); // shift left
          ring.setRange(ring.length - pcm.length, ring.length, pcm);
          filled = (filled + pcm.length).clamp(0, ring.length);
        }
      }
    } else if (cmd == 'tick') {
      final now = m[1] as double;
      if (eng != null) {
        final s = eng.tick(now).toMessage();
        // Only wake the UI isolate when something it cannot extrapolate changed.
        bool changed = last.isEmpty || s[0] != last[0] || s[1] != last[1] || s[4] != last[4] || s[5] != last[5];
        if (!changed && s[0] == 1) {
          final expected = (last[2] as double) + (now - lastNow);
          changed = ((s[2] as double) - expected).abs() > 30; // drift correction / seek
        }
        if (!changed && s[0] == 0) changed = ((s[6] as double) - (last[6] as double)).abs() > 3; // level meter
        if (changed) {
          out.send([...s, now]);
          last = s;
          lastNow = now;
        }
      } else if (last.isEmpty) {
        // no exact engine: keep the microphone open, report "listening"
        last = const EngineSnapshot(0, -1, 0, 0, true, false, -120).toMessage();
        out.send([...last, now]);
      }
      // semantic detection once per second while no exact lock
      final locked = last.isNotEmpty && (last[1] as int) >= 0;
      if (rec != null && !locked && filled == ring.length && now >= nextDetect) {
        nextDetect = now + 1000;
        final txt = rec.recognize(ring, kMicRate);
        if (txt != lastDetection) {
          lastDetection = txt;
          out.send(['detect', txt]);
        }
      }
    } else if (cmd == 'reset') {
      eng?.reset();
      filled = 0;
      lastDetection = '';
      out.send(['detect', '']);
    }
  });
}
