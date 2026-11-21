// Reis — open the app, it listens, it shows the song and its synced lyrics.
import 'dart:io';
import 'dart:math' as math;

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';

import 'build_screen.dart';
import 'engine_service.dart';
import 'lyrics_view.dart';
import 'reis_ffi.dart';
import 'settings.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  runApp(const ReisApp());
}

class ReisApp extends StatelessWidget {
  const ReisApp({super.key});
  @override
  Widget build(BuildContext context) => MaterialApp(
        title: 'Reis',
        debugShowCheckedModeBanner: false,
        theme: ThemeData(colorSchemeSeed: Colors.deepPurple, brightness: Brightness.dark, useMaterial3: true),
        home: const CatalogGate(),
      );
}

/// Finds the offline catalog: app storage, else bundled asset, else one-time import.
class CatalogGate extends StatefulWidget {
  const CatalogGate({super.key});
  @override
  State<CatalogGate> createState() => _CatalogGateState();
}

class _CatalogGateState extends State<CatalogGate> {
  String? _path;
  String? _error;
  bool _busy = true;

  @override
  void initState() {
    super.initState();
    _locate();
  }

  Future<Directory> _dir() async => getApplicationSupportDirectory();
  Future<File> _pointer() async => File('${(await _dir()).path}/current_library.txt');

  /// The active library: path stored in current_library.txt, else catalog.reis,
  /// else a library bundled in the app assets.
  Future<void> _locate() async {
    String? path;
    final ptr = await _pointer();
    if (await ptr.exists()) {
      final p = (await ptr.readAsString()).trim();
      if (p.isNotEmpty && await File(p).exists()) path = p;
    }
    final legacy = File('${(await _dir()).path}/catalog.reis');
    if (path == null && await legacy.exists()) path = legacy.path;
    if (path == null) {
      try {
        final data = await rootBundle.load('assets/catalog.reis');
        await legacy.writeAsBytes(data.buffer.asUint8List(data.offsetInBytes, data.lengthInBytes), flush: true);
        path = legacy.path;
      } catch (_) {/* no bundled catalog */}
    }
    setState(() {
      _busy = false;
      _path = path;
    });
  }

  Future<String> _newPath() async => '${(await _dir()).path}/library-${DateTime.now().millisecondsSinceEpoch}.reis';

  /// Switch to a freshly written library and forget the previous one.
  Future<void> _activate(String path) async {
    final old = _path;
    await (await _pointer()).writeAsString(path, flush: true);
    setState(() {
      _path = path;
      _error = null;
    });
    if (old != null && old != path) {
      // the previous library may still be mapped for a moment: delete it later if needed
      Future.delayed(const Duration(seconds: 2), () async {
        try {
          await File(old).delete();
        } catch (_) {}
      });
    }
  }

  Future<void> _create() async {
    final out = await _newPath();
    final ok = await Navigator.push<bool>(context, MaterialPageRoute(builder: (_) => BuildScreen(outPath: out)));
    if (ok == true && await File(out).exists()) await _activate(out);
  }

  Future<void> _import() async {
    final r = await FilePicker.platform.pickFiles(dialogTitle: 'Choisir une bibliothèque .reis');
    final src = r?.files.single.path;
    if (src == null) return;
    setState(() => _busy = true);
    final out = await _newPath();
    await File(src).copy(out);
    final ok = ReisCatalog.open(out);
    setState(() => _busy = false);
    if (ok == null) {
      await File(out).delete();
      setState(() => _error = 'Ce fichier n\'est pas une bibliothèque Reis valide (ou date d\'une ancienne version).');
      return;
    }
    ok.dispose();
    await _activate(out);
  }

  void _libraryMenu() => showModalBottomSheet<void>(
        context: context,
        showDragHandle: true,
        builder: (c) => SafeArea(
          child: Column(mainAxisSize: MainAxisSize.min, children: [
            ListTile(
              leading: const Icon(Icons.fingerprint),
              title: const Text('Créer une nouvelle bibliothèque'),
              subtitle: const Text('à partir des sons et musiques de cet appareil'),
              onTap: () {
                Navigator.pop(c);
                _create();
              },
            ),
            ListTile(
              leading: const Icon(Icons.file_open),
              title: const Text('Importer un fichier .reis'),
              subtitle: const Text('construit sur ordinateur avec « reis index »'),
              onTap: () {
                Navigator.pop(c);
                _import();
              },
            ),
          ]),
        ),
      );

  @override
  Widget build(BuildContext context) {
    if (_busy) return const Scaffold(body: Center(child: CircularProgressIndicator()));
    if (_path != null) return HomeScreen(key: ValueKey(_path), catalogPath: _path!, onLibraryMenu: _libraryMenu);
    return Scaffold(
      body: Center(
        child: Padding(
          padding: const EdgeInsets.all(32),
          child: Column(mainAxisSize: MainAxisSize.min, children: [
            const Icon(Icons.library_music_outlined, size: 64),
            const SizedBox(height: 16),
            const Text('Aucune bibliothèque', style: TextStyle(fontSize: 20)),
            const SizedBox(height: 8),
            const Text('Reis reconnaît les sons que tu lui as appris, sans Internet. '
                'Donne-lui tes musiques ou tes sons : il en crée les empreintes, ici, sur le téléphone.', textAlign: TextAlign.center),
            if (_error != null) Padding(padding: const EdgeInsets.only(top: 12), child: Text(_error!, style: const TextStyle(color: Colors.redAccent))),
            const SizedBox(height: 20),
            FilledButton.icon(onPressed: _create, icon: const Icon(Icons.fingerprint), label: const Text('Créer ma bibliothèque')),
            const SizedBox(height: 8),
            TextButton.icon(onPressed: _import, icon: const Icon(Icons.file_open), label: const Text('Importer un fichier .reis')),
          ]),
        ),
      ),
    );
  }
}

class HomeScreen extends StatefulWidget {
  final String catalogPath;
  final VoidCallback onLibraryMenu;
  const HomeScreen({super.key, required this.catalogPath, required this.onLibraryMenu});
  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  late final ReisCatalog _cat = ReisCatalog.open(widget.catalogPath)!;
  late final EngineService _svc = EngineService(widget.catalogPath);
  final _prefs = LyricsPrefs();
  SongInfo? _song;

  @override
  void initState() {
    super.initState();
    _prefs.load();
    _svc.addListener(_onState);
    _svc.start(); // mic starts immediately: no button to press
  }

  void _onState() {
    final id = _svc.state.song;
    if (id >= 0 && _song?.id != id) {
      _song = _cat.song(id);
    } else if (id < 0 && _song != null && _svc.state.phase == 0 && !_svc.state.uncertain) {
      _song = null;
    }
    setState(() {});
  }

  @override
  void dispose() {
    _svc.removeListener(_onState);
    _svc.dispose();
    _cat.dispose();
    super.dispose();
  }

  void _settings() => showModalBottomSheet(context: context, isScrollControlled: true, showDragHandle: true,
      builder: (_) => FractionallySizedBox(heightFactor: 0.8, child: SettingsSheet(prefs: _prefs)));

  @override
  Widget build(BuildContext context) {
    final s = _svc.state;
    return Scaffold(
      backgroundColor: Colors.black,
      body: SafeArea(
        child: Stack(children: [
          if (_svc.permissionDenied)
            const Center(child: Padding(padding: EdgeInsets.all(32),
                child: Text('Reis a besoin du micro pour écouter la musique.\nAutorise-le dans les réglages du téléphone.', textAlign: TextAlign.center)))
          else if (_song == null && _svc.detection != null && const ['similar', 'tag', 'exact'].contains(_svc.detection!.kind))
            DetectionView(d: _svc.detection!)
          else if (_song == null)
            ListeningView(level: s.levelDbfs)
          else
            NowPlaying(song: _song!, svc: _svc, prefs: _prefs),
          Positioned(
            right: 4,
            top: 4,
            child: Row(children: [
              if (_song != null)
                IconButton(tooltip: 'Réécouter', icon: const Icon(Icons.refresh), onPressed: () {
                  _svc.restart();
                  setState(() => _song = null);
                }),
              IconButton(tooltip: 'Paroles', icon: const Icon(Icons.text_fields), onPressed: _settings),
              IconButton(tooltip: 'Bibliothèque', icon: const Icon(Icons.library_music), onPressed: widget.onLibraryMenu),
            ]),
          ),
        ]),
      ),
    );
  }
}

/// Library of sounds (animals, noises…): shows what is heard right now.
class DetectionView extends StatelessWidget {
  final Detection d;
  const DetectionView({super.key, required this.d});

  @override
  Widget build(BuildContext context) {
    final c = Theme.of(context).colorScheme;
    final known = d.kind == 'similar' || d.kind == 'exact';
    final name = d.label.split('/').last.replaceAll('_', ' ');
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(32),
        child: Column(mainAxisSize: MainAxisSize.min, children: [
          Icon(known ? Icons.graphic_eq : Icons.help_outline, size: 56, color: known ? c.primary : Colors.white54),
          const SizedBox(height: 16),
          Text(name, textAlign: TextAlign.center, style: const TextStyle(fontSize: 34, fontWeight: FontWeight.w700)),
          if (d.label.contains('/'))
            Text(d.label.substring(0, d.label.lastIndexOf('/')), style: const TextStyle(color: Colors.white54)),
          const SizedBox(height: 8),
          Text(
            known ? 'reconnu dans ta bibliothèque · ${(d.score * 100).round()}' : 'absent de ta bibliothèque — ressemble à : ${(d.score * 100).round()} %',
            style: const TextStyle(color: Colors.white70),
            textAlign: TextAlign.center,
          ),
          if (d.candidates.length > 1) ...[
            const SizedBox(height: 24),
            for (final cand in d.candidates.skip(known ? 1 : 0).take(3))
              Padding(
                padding: const EdgeInsets.symmetric(vertical: 2),
                child: Text('${cand.$1.split('/').last.replaceAll('_', ' ')}  ${(cand.$2 * 100).round()}',
                    style: const TextStyle(color: Colors.white38)),
              ),
          ],
        ]),
      ),
    );
  }
}

class ListeningView extends StatefulWidget {
  final double level;
  const ListeningView({super.key, required this.level});
  @override
  State<ListeningView> createState() => _ListeningViewState();
}

class _ListeningViewState extends State<ListeningView> with SingleTickerProviderStateMixin {
  late final AnimationController _a = AnimationController(vsync: this, duration: const Duration(milliseconds: 1600))..repeat();

  @override
  void dispose() {
    _a.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final lvl = ((widget.level + 60) / 50).clamp(0.0, 1.0);
    final c = Theme.of(context).colorScheme.primary;
    return Center(
      child: Column(mainAxisSize: MainAxisSize.min, children: [
        SizedBox(
          width: 220,
          height: 220,
          child: AnimatedBuilder(
            animation: _a,
            builder: (_, __) => CustomPaint(painter: _Rings(_a.value, lvl, c)),
          ),
        ),
        const SizedBox(height: 24),
        const Text('Écoute…', style: TextStyle(fontSize: 22)),
      ]),
    );
  }
}

class _Rings extends CustomPainter {
  final double t, level;
  final Color c;
  _Rings(this.t, this.level, this.c);
  @override
  void paint(Canvas canvas, Size size) {
    final o = size.center(Offset.zero);
    for (var k = 0; k < 3; k++) {
      final p = (t + k / 3) % 1.0;
      canvas.drawCircle(o, 40 + p * 70, Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2
        ..color = c.withOpacity((1 - p) * 0.6));
    }
    canvas.drawCircle(o, 34 + 16 * level, Paint()..color = c);
    final ic = TextPainter(
        text: const TextSpan(text: '♪', style: TextStyle(fontSize: 34, color: Colors.white)), textDirection: TextDirection.ltr)
      ..layout();
    ic.paint(canvas, o - Offset(ic.width / 2, ic.height / 2));
  }

  @override
  bool shouldRepaint(_Rings old) => old.t != t || old.level != level;
}

class NowPlaying extends StatelessWidget {
  final SongInfo song;
  final EngineService svc;
  final LyricsPrefs prefs;
  const NowPlaying({super.key, required this.song, required this.svc, required this.prefs});

  @override
  Widget build(BuildContext context) {
    final st = svc.state;
    final art = song.artwork != null
        ? Image.memory(song.artwork!, width: 72, height: 72, fit: BoxFit.cover, gaplessPlayback: true)
        : Container(width: 72, height: 72, color: Colors.white12, child: const Icon(Icons.album));
    return Column(children: [
      Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 100, 8),
        child: Row(children: [
          ClipRRect(borderRadius: BorderRadius.circular(8), child: art),
          const SizedBox(width: 14),
          Expanded(
            child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
              Text(song.title, maxLines: 1, overflow: TextOverflow.ellipsis, style: const TextStyle(fontSize: 18, fontWeight: FontWeight.w700)),
              Text(song.artist, maxLines: 1, overflow: TextOverflow.ellipsis, style: const TextStyle(color: Colors.white70)),
              if (song.album.isNotEmpty)
                Text(song.album, maxLines: 1, overflow: TextOverflow.ellipsis, style: const TextStyle(color: Colors.white38, fontSize: 12)),
            ]),
          ),
        ]),
      ),
      _Progress(svc: svc, durationMs: song.durationMs, paused: st.phase == 2, uncertain: st.uncertain),
      Expanded(child: LyricsView(key: ValueKey(song.id), lines: song.lyrics, position: () => svc.positionMs, prefs: prefs)),
    ]);
  }
}

class _Progress extends StatefulWidget {
  final EngineService svc;
  final int durationMs;
  final bool paused, uncertain;
  const _Progress({required this.svc, required this.durationMs, required this.paused, required this.uncertain});
  @override
  State<_Progress> createState() => _ProgressState();
}

class _ProgressState extends State<_Progress> with SingleTickerProviderStateMixin {
  late final AnimationController _tick = AnimationController(vsync: this, duration: const Duration(seconds: 1))..repeat();
  @override
  void dispose() {
    _tick.dispose();
    super.dispose();
  }

  String _fmt(double ms) {
    final s = (ms / 1000).floor().clamp(0, 359999);
    return '${s ~/ 60}:${(s % 60).toString().padLeft(2, '0')}';
  }

  @override
  Widget build(BuildContext context) => AnimatedBuilder(
        animation: _tick,
        builder: (_, __) {
          final pos = widget.svc.positionMs;
          final d = math.max(1, widget.durationMs);
          return Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16),
            child: Row(children: [
              Text(_fmt(pos), style: const TextStyle(fontSize: 12, color: Colors.white54)),
              const SizedBox(width: 8),
              Expanded(child: LinearProgressIndicator(value: (pos / d).clamp(0.0, 1.0), minHeight: 3)),
              const SizedBox(width: 8),
              Text(_fmt(d.toDouble()), style: const TextStyle(fontSize: 12, color: Colors.white54)),
              if (widget.paused) const Padding(padding: EdgeInsets.only(left: 8), child: Icon(Icons.pause, size: 16)),
              if (widget.uncertain && !widget.paused)
                const Padding(padding: EdgeInsets.only(left: 8), child: Icon(Icons.hearing, size: 16, color: Colors.white38)),
            ]),
          );
        },
      );
}
