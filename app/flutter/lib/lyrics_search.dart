// "Chercher les paroles": guessed title / artist (editable), LRCLIB results,
// preview, choose. Returns the chosen candidate.
import 'package:flutter/material.dart';

import 'lyrics_online.dart';
import 'reis_ffi.dart';

class LyricsSearchPage extends StatefulWidget {
  final SongInfo song;
  final String fileName;
  const LyricsSearchPage({super.key, required this.song, required this.fileName});

  @override
  State<LyricsSearchPage> createState() => _LyricsSearchPageState();
}

class _LyricsSearchPageState extends State<LyricsSearchPage> {
  late final TrackGuess _guess = guessTrack(
    title: widget.song.title,
    artist: widget.song.artist.isEmpty ? null : widget.song.artist,
    fileName: widget.fileName.isEmpty ? widget.song.title : widget.fileName,
  );
  late final _title = TextEditingController(text: _guess.title);
  late final _artist = TextEditingController(text: _guess.artist ?? '');
  List<LyricsCandidate>? _results;
  bool _busy = false;
  String? _error;

  double get _dur => widget.song.durationMs / 1000;

  @override
  void initState() {
    super.initState();
    _search();
  }

  Future<void> _search() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final g = TrackGuess(_title.text.trim(), _artist.text.trim().isEmpty ? null : _artist.text.trim());
      final r = await LrcLib.search(g, durationS: _dur > 0 ? _dur : null);
      setState(() => _results = r);
    } catch (e) {
      setState(() => _error = '$e');
    } finally {
      setState(() => _busy = false);
    }
  }

  String _mmss(double s) => '${s ~/ 60}:${(s % 60).round().toString().padLeft(2, '0')}';

  Future<void> _preview(LyricsCandidate c) async {
    final lines = parseLrc(c.lrc).map((l) => l.text).where((t) => t.isNotEmpty).take(12).join('\n');
    final ok = await showDialog<bool>(
      context: context,
      builder: (d) => AlertDialog(
        title: Text('${c.artist} — ${c.track}'),
        content: SingleChildScrollView(child: Text(lines.isEmpty ? '(vide)' : '$lines\n…')),
        actions: [
          TextButton(onPressed: () => Navigator.pop(d, false), child: const Text('Retour')),
          FilledButton(onPressed: () => Navigator.pop(d, true), child: const Text('Utiliser ces paroles')),
        ],
      ),
    );
    if (ok == true && mounted) Navigator.pop(context, c);
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Chercher les paroles')),
      body: ListView(padding: const EdgeInsets.all(16), children: [
        Text('Détecté : ${_guess.toString()}', style: const TextStyle(color: Colors.white54)),
        const SizedBox(height: 8),
        TextField(controller: _title, decoration: const InputDecoration(labelText: 'Titre'), onSubmitted: (_) => _search()),
        TextField(controller: _artist, decoration: const InputDecoration(labelText: 'Artiste'), onSubmitted: (_) => _search()),
        const SizedBox(height: 8),
        Row(children: [
          FilledButton.icon(onPressed: _busy ? null : _search, icon: const Icon(Icons.search), label: const Text('Chercher')),
          const SizedBox(width: 12),
          if (_busy) const SizedBox(width: 20, height: 20, child: CircularProgressIndicator(strokeWidth: 2)),
        ]),
        const SizedBox(height: 4),
        const Text('Recherche sur lrclib.net (Internet) : seuls le titre, l\'artiste et la durée sont envoyés.',
            style: TextStyle(color: Colors.white38, fontSize: 12)),
        if (_error != null) Padding(padding: const EdgeInsets.only(top: 12), child: Text(_error!, style: const TextStyle(color: Colors.redAccent))),
        const Divider(height: 24),
        if (_results != null && _results!.isEmpty) const Text('Aucun résultat. Modifie le titre ou l\'artiste.'),
        for (final c in _results ?? const <LyricsCandidate>[])
          ListTile(
            contentPadding: EdgeInsets.zero,
            leading: Icon(c.isSynced ? Icons.lyrics : Icons.notes, color: c.isSynced ? Theme.of(context).colorScheme.primary : null),
            title: Text('${c.track} — ${c.artist}', maxLines: 1, overflow: TextOverflow.ellipsis),
            subtitle: Text([
              if (c.album.isNotEmpty) c.album,
              _mmss(c.duration),
              if (_dur > 0) ((c.duration - _dur).abs() <= 2 ? 'même durée' : 'écart ${(c.duration - _dur).round()} s'),
              c.isSynced ? 'synchronisées' : (c.plain != null ? 'texte seul' : 'sans paroles'),
            ].join(' · ')),
            enabled: c.hasLyrics,
            onTap: () => _preview(c),
          ),
      ]),
    );
  }
}
