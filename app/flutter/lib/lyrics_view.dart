// Synced lyrics: active line centred, previous/next lines faded, optional
// word-by-word highlight. Repaints from a Ticker at display rate; the active
// line is found by binary search, so the cost per frame is O(log n).
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';

import 'reis_ffi.dart';
import 'settings.dart';

int activeLineAt(List<LyricLine> lines, double posMs) {
  var lo = 0, hi = lines.length;
  while (lo < hi) {
    final mid = (lo + hi) >> 1;
    if (lines[mid].tMs <= posMs) {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }
  return lo - 1; // -1 before the first line
}

class LyricsView extends StatefulWidget {
  final List<LyricLine> lines;
  final double Function() position; // ms, extrapolated
  final LyricsPrefs prefs;
  const LyricsView({super.key, required this.lines, required this.position, required this.prefs});

  @override
  State<LyricsView> createState() => _LyricsViewState();
}

class _LyricsViewState extends State<LyricsView> with SingleTickerProviderStateMixin {
  late final Ticker _ticker;
  final _scroll = ScrollController();
  final _keys = <int, GlobalKey>{};
  int _active = -2;
  double _pos = 0;

  @override
  void initState() {
    super.initState();
    _ticker = createTicker((_) {
      if (!_synced) return;
      final p = widget.position();
      final a = activeLineAt(widget.lines, p);
      final hasWords = widget.prefs.showWords && a >= 0 && widget.lines[a].words.isNotEmpty;
      if (a != _active || hasWords) {
        setState(() {
          _pos = p;
          if (a != _active) {
            _active = a;
            WidgetsBinding.instance.addPostFrameCallback((_) => _scrollToActive());
          }
        });
      }
    })
      ..start();
  }

  void _scrollToActive() {
    final ctx = _keys[_active.clamp(0, widget.lines.length - 1)]?.currentContext;
    if (ctx != null) {
      Scrollable.ensureVisible(ctx, alignment: 0.4, duration: const Duration(milliseconds: 350), curve: Curves.easeOutCubic);
    }
  }

  @override
  void dispose() {
    _ticker.dispose();
    _scroll.dispose();
    super.dispose();
  }

  Widget _line(int i) {
    final l = widget.lines[i];
    final p = widget.prefs;
    final active = i == _active;
    final text = l.text.isEmpty ? '♪' : l.text;
    Widget child;
    if (active && p.showWords && l.words.isNotEmpty) {
      // bytes sung so far -> split the UTF-8 string
      var sung = 0;
      for (final w in l.words) {
        if (w.tMs <= _pos) sung = w.byteEnd;
      }
      final bytes = utf8.encode(l.text);
      final a = utf8.decode(bytes.sublist(0, sung.clamp(0, bytes.length)), allowMalformed: true);
      final b = utf8.decode(bytes.sublist(sung.clamp(0, bytes.length)), allowMalformed: true);
      child = Text.rich(
        TextSpan(children: [
          TextSpan(text: a, style: p.style(active: true)),
          TextSpan(text: b, style: p.style(active: false)),
        ]),
        textAlign: p.alignment,
      );
    } else {
      child = AnimatedDefaultTextStyle(
        duration: const Duration(milliseconds: 250),
        style: p.style(active: active),
        textAlign: p.alignment,
        child: Text(text, textAlign: p.alignment),
      );
    }
    return Padding(
      key: _keys.putIfAbsent(i, () => GlobalKey()),
      padding: EdgeInsets.symmetric(vertical: p.lineSpacing / 2, horizontal: 24),
      child: SizedBox(width: double.infinity, child: child),
    );
  }

  bool get _synced => widget.lines.isNotEmpty && widget.lines.first.tMs >= 0;

  @override
  Widget build(BuildContext context) {
    if (widget.lines.isEmpty) {
      return Center(child: Text('Pas de paroles pour ce morceau', style: widget.prefs.style(active: false)));
    }
    if (!_synced) {
      // plain lyrics (no timestamps): readable text, no highlighting
      return ListenableBuilder(
        listenable: widget.prefs,
        builder: (context, _) => ListView(padding: const EdgeInsets.fromLTRB(24, 16, 24, 80), children: [
          Text('Paroles non synchronisées', textAlign: widget.prefs.alignment, style: const TextStyle(color: Colors.white38, fontSize: 12)),
          const SizedBox(height: 12),
          for (final l in widget.lines)
            Padding(
              padding: EdgeInsets.symmetric(vertical: widget.prefs.lineSpacing / 4),
              child: Text(l.text.isEmpty ? ' ' : l.text, textAlign: widget.prefs.alignment,
                  style: widget.prefs.style(active: true).copyWith(fontSize: widget.prefs.fontSize * 0.8)),
            ),
        ]),
      );
    }
    return ListenableBuilder(
      listenable: widget.prefs,
      builder: (context, _) => ShaderMask(
        shaderCallback: (r) => const LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [Colors.transparent, Colors.white, Colors.white, Colors.transparent],
          stops: [0, 0.12, 0.85, 1],
        ).createShader(r),
        blendMode: BlendMode.dstIn,
        child: SingleChildScrollView(
          controller: _scroll,
          padding: const EdgeInsets.symmetric(vertical: 200),
          child: Column(children: [for (var i = 0; i < widget.lines.length; i++) _line(i)]),
        ),
      ),
    );
  }
}
