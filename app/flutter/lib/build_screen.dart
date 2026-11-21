// "Créer ma bibliothèque": pick sounds on the device, choose what to
// recognise, build the .reis file with a progress bar.
import 'dart:io';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';

import 'library_builder.dart';

class BuildScreen extends StatefulWidget {
  /// Where the finished library is written.
  final String outPath;
  const BuildScreen({super.key, required this.outPath});

  @override
  State<BuildScreen> createState() => _BuildScreenState();
}

class _BuildScreenState extends State<BuildScreen> {
  Map<String, List<SoundSource>>? _folders; // phone audio by folder
  final Set<String> _selected = {};
  final List<SoundSource> _picked = [];
  final _category = TextEditingController(text: 'sons');
  LibraryKind _kind = LibraryKind.music;
  bool _augment = false;
  bool _lyrics = false;
  bool _loading = false;
  String? _error;
  LibraryBuildJob? _job;
  BuildProgress? _progress;

  @override
  void initState() {
    super.initState();
    if (hasNativeDecoder) _loadPhone();
  }

  Future<void> _loadPhone() async {
    setState(() => _loading = true);
    try {
      if (!await requestAudioPermission()) {
        setState(() => _error = 'Reis a besoin de l\'accès à tes fichiers audio pour les lire.');
      } else {
        final f = await listPhoneAudio();
        setState(() => _folders = f);
      }
    } catch (e) {
      setState(() => _error = '$e');
    } finally {
      setState(() => _loading = false);
    }
  }

  /// Files chosen with the system picker (any platform). Matching .lrc files
  /// picked at the same time become the lyrics of the sound with the same name.
  Future<void> _pickFiles() async {
    final r = await FilePicker.platform.pickFiles(allowMultiple: true, type: FileType.any, dialogTitle: 'Sons et paroles (.lrc)');
    if (r == null) return;
    final lrc = <String, String>{};
    for (final f in r.files) {
      if (f.path != null && f.name.toLowerCase().endsWith('.lrc')) {
        lrc[_stem(f.name)] = await File(f.path!).readAsString();
      }
    }
    final label = _category.text.trim().isEmpty ? 'sons' : _category.text.trim();
    setState(() {
      for (final f in r.files) {
        final n = f.name.toLowerCase();
        if (f.path == null || n.endsWith('.lrc') || n.endsWith('.jpg') || n.endsWith('.png') || n.endsWith('.webp')) continue;
        _picked.add(SoundSource(source: f.path!, path: f.path, name: _stem(f.name), label: label, lyrics: lrc[_stem(f.name)]));
      }
    });
  }

  String _stem(String n) => n.contains('.') ? n.substring(0, n.lastIndexOf('.')) : n;

  List<SoundSource> get _sources => [
        for (final k in _selected) ...?_folders?[k],
        ..._picked,
      ];

  Future<void> _build() async {
    final sources = _sources;
    if (sources.isEmpty) return;
    final job = LibraryBuildJob(
        sources: sources, kind: _kind, outPath: widget.outPath, augmentSpeaker: _augment, downloadLyrics: _lyrics && _kind != LibraryKind.sounds);
    setState(() {
      _job = job;
      _error = null;
    });
    job.progress.listen((p) => setState(() => _progress = p));
    try {
      final r = await job.run();
      if (!mounted) return;
      if (r.added == 0) {
        setState(() {
          _job = null;
          _error = 'Aucun son n\'a pu être lu.\n${r.errors.take(3).join('\n')}';
        });
        return;
      }
      await showDialog<void>(
        context: context,
        builder: (c) => AlertDialog(
          title: const Text('Bibliothèque prête'),
          content: Text('${r.added} sons ajoutés${r.failed > 0 ? ', ${r.failed} ignorés' : ''}'
              '${r.lyricsDownloaded > 0 ? ', paroles téléchargées pour ${r.lyricsDownloaded}' : ''}.'
              '${r.errors.isEmpty ? '' : '\n\n${r.errors.take(3).join('\n')}'}'),
          actions: [TextButton(onPressed: () => Navigator.pop(c), child: const Text('Écouter'))],
        ),
      );
      if (mounted) Navigator.pop(context, true);
    } catch (e) {
      setState(() {
        _job = null;
        _error = '$e';
      });
    }
  }

  String _fmt(Duration? d) {
    if (d == null) return '…';
    final s = d.inSeconds;
    return s < 60 ? '$s s' : '${s ~/ 60} min ${s % 60} s';
  }

  @override
  Widget build(BuildContext context) {
    final building = _job != null;
    return Scaffold(
      appBar: AppBar(title: const Text('Créer ma bibliothèque')),
      body: building ? _buildingView() : _chooseView(),
      bottomNavigationBar: building
          ? null
          : SafeArea(
              child: Padding(
                padding: const EdgeInsets.all(16),
                child: FilledButton.icon(
                  onPressed: _sources.isEmpty ? null : _build,
                  icon: const Icon(Icons.fingerprint),
                  label: Text('Créer les empreintes (${_sources.length} sons)'),
                ),
              ),
            ),
    );
  }

  Widget _buildingView() {
    final p = _progress;
    final frac = (p == null || p.total == 0) ? null : p.done / p.total;
    return Padding(
      padding: const EdgeInsets.all(24),
      child: Column(mainAxisAlignment: MainAxisAlignment.center, children: [
        const Icon(Icons.fingerprint, size: 64),
        const SizedBox(height: 24),
        LinearProgressIndicator(value: frac),
        const SizedBox(height: 12),
        Text(p == null ? 'Préparation…' : '${p.done} / ${p.total}', style: const TextStyle(fontSize: 18)),
        if (p != null) Text(p.current, maxLines: 1, overflow: TextOverflow.ellipsis, style: const TextStyle(color: Colors.white60)),
        if (p != null) Text('temps restant ≈ ${_fmt(p.remaining)}', style: const TextStyle(color: Colors.white38)),
        const SizedBox(height: 24),
        TextButton(onPressed: () => _job?.cancel(), child: const Text('Arrêter et garder ce qui est fait')),
      ]),
    );
  }

  Widget _chooseView() {
    final folders = _folders?.keys.toList() ?? [];
    folders.sort();
    return ListView(padding: const EdgeInsets.all(16), children: [
      const Text('Que doit reconnaître Reis ?', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w600)),
      const SizedBox(height: 8),
      SegmentedButton<LibraryKind>(
        segments: const [
          ButtonSegment(value: LibraryKind.music, label: Text('Musiques'), icon: Icon(Icons.music_note)),
          ButtonSegment(value: LibraryKind.sounds, label: Text('Types de sons'), icon: Icon(Icons.pets)),
          ButtonSegment(value: LibraryKind.both, label: Text('Les deux')),
        ],
        selected: {_kind},
        onSelectionChanged: (s) => setState(() => _kind = s.first),
      ),
      const SizedBox(height: 6),
      Text(
        switch (_kind) {
          LibraryKind.music => 'Le morceau exact, sa position et ses paroles synchronisées. Rapide (≈ 0,1 s par morceau).',
          LibraryKind.sounds => 'Le même genre de son, même un autre enregistrement : un autre chien, une autre sonnette. '
              'Chaque dossier devient une catégorie.',
          LibraryKind.both => 'Les deux moteurs. Plus lent à construire pour de longues musiques.',
        },
        style: const TextStyle(color: Colors.white60),
      ),
      if (_kind != LibraryKind.sounds)
        SwitchListTile(
          contentPadding: EdgeInsets.zero,
          value: _lyrics,
          onChanged: (v) => setState(() => _lyrics = v),
          title: const Text('Télécharger les paroles manquantes'),
          subtitle: const Text('Paroles synchronisées depuis lrclib.net (Internet). Titre et artiste déduits des tags '
              'ou du nom du fichier ; seuls ceux-ci et la durée sont envoyés.'),
        ),
      if (_kind != LibraryKind.music)
        SwitchListTile(
          contentPadding: EdgeInsets.zero,
          value: _augment,
          onChanged: (v) => setState(() => _augment = v),
          title: const Text('Sons entendus à travers un haut-parleur'),
          subtitle: const Text('Meilleure reconnaissance via enceinte / télé, construction 2× plus longue'),
        ),
      const Divider(height: 32),
      if (_error != null) Padding(padding: const EdgeInsets.only(bottom: 12), child: Text(_error!, style: const TextStyle(color: Colors.redAccent))),
      if (hasNativeDecoder) ...[
        const Text('Sons du téléphone', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w600)),
        const Text('Coche les dossiers à inclure. Le nom du dossier devient la catégorie.', style: TextStyle(color: Colors.white60)),
        if (_loading) const Padding(padding: EdgeInsets.all(16), child: Center(child: CircularProgressIndicator())),
        if (!_loading && folders.isEmpty) const Padding(padding: EdgeInsets.all(8), child: Text('Aucun fichier audio trouvé.')),
        for (final f in folders)
          CheckboxListTile(
            contentPadding: EdgeInsets.zero,
            value: _selected.contains(f),
            onChanged: (v) => setState(() => v == true ? _selected.add(f) : _selected.remove(f)),
            title: Text(f.isEmpty ? '(racine)' : f.split('/').last),
            subtitle: Text('${_folders![f]!.length} sons · $f', maxLines: 1, overflow: TextOverflow.ellipsis),
          ),
        if (folders.length > 1)
          Align(
            alignment: Alignment.centerLeft,
            child: TextButton(
              onPressed: () => setState(() => _selected.length == folders.length ? _selected.clear() : _selected.addAll(folders)),
              child: Text(_selected.length == folders.length ? 'Tout décocher' : 'Tout cocher'),
            ),
          ),
        const Divider(height: 32),
      ],
      const Text('Ou choisir des fichiers', style: TextStyle(fontSize: 18, fontWeight: FontWeight.w600)),
      const SizedBox(height: 8),
      TextField(controller: _category, decoration: const InputDecoration(labelText: 'Catégorie de ces fichiers', hintText: 'ex. chien, sonnette, musique')),
      const SizedBox(height: 8),
      OutlinedButton.icon(onPressed: _pickFiles, icon: const Icon(Icons.add), label: const Text('Ajouter des fichiers (et leurs .lrc)')),
      if (!hasNativeDecoder)
        const Padding(
          padding: EdgeInsets.only(top: 6),
          child: Text('Sur ordinateur : WAV, ou tous formats si ffmpeg est installé.', style: TextStyle(color: Colors.white38)),
        ),
      for (final s in _picked)
        ListTile(
          dense: true,
          contentPadding: EdgeInsets.zero,
          leading: Icon(s.lyrics != null ? Icons.lyrics : Icons.audio_file),
          title: Text(s.name, maxLines: 1, overflow: TextOverflow.ellipsis),
          subtitle: Text(s.label),
          trailing: IconButton(icon: const Icon(Icons.close), onPressed: () => setState(() => _picked.remove(s))),
        ),
      const SizedBox(height: 80),
    ]);
  }
}
