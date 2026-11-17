// Lyrics display preferences, persisted with shared_preferences.
import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

class LyricsPrefs extends ChangeNotifier {
  static const fonts = ['System', 'serif', 'monospace', 'Roboto', 'Georgia'];

  String font = 'System';
  double fontSize = 26;
  bool bold = true;
  double lineSpacing = 18; // px between lines
  double lineHeight = 1.25; // multiple of font size
  TextAlign alignment = TextAlign.center;
  int activeColor = 0xFFFFFFFF;
  int inactiveOpacity = 40; // % opacity of non-active lines
  bool highContrast = false;
  bool showWords = true; // word-level highlight when available

  SharedPreferences? _p;

  Future<void> load() async {
    final p = _p = await SharedPreferences.getInstance();
    font = p.getString('lyrics.font') ?? font;
    fontSize = p.getDouble('lyrics.fontSize') ?? fontSize;
    bold = p.getBool('lyrics.bold') ?? bold;
    lineSpacing = p.getDouble('lyrics.lineSpacing') ?? lineSpacing;
    lineHeight = p.getDouble('lyrics.lineHeight') ?? lineHeight;
    alignment = TextAlign.values[p.getInt('lyrics.alignment') ?? alignment.index];
    activeColor = p.getInt('lyrics.activeColor') ?? activeColor;
    inactiveOpacity = p.getInt('lyrics.inactiveOpacity') ?? inactiveOpacity;
    highContrast = p.getBool('lyrics.highContrast') ?? highContrast;
    showWords = p.getBool('lyrics.showWords') ?? showWords;
    notifyListeners();
  }

  Future<void> _save() async {
    final p = _p ?? await SharedPreferences.getInstance();
    await p.setString('lyrics.font', font);
    await p.setDouble('lyrics.fontSize', fontSize);
    await p.setBool('lyrics.bold', bold);
    await p.setDouble('lyrics.lineSpacing', lineSpacing);
    await p.setDouble('lyrics.lineHeight', lineHeight);
    await p.setInt('lyrics.alignment', alignment.index);
    await p.setInt('lyrics.activeColor', activeColor);
    await p.setInt('lyrics.inactiveOpacity', inactiveOpacity);
    await p.setBool('lyrics.highContrast', highContrast);
    await p.setBool('lyrics.showWords', showWords);
  }

  void update(void Function(LyricsPrefs p) f) {
    f(this);
    notifyListeners();
    _save();
  }

  TextStyle style({required bool active}) {
    final c = Color(activeColor);
    final op = highContrast ? (active ? 1.0 : 0.65) : (active ? 1.0 : inactiveOpacity / 100);
    return TextStyle(
      fontFamily: font == 'System' ? null : font,
      fontSize: fontSize,
      fontWeight: bold ? FontWeight.w700 : FontWeight.w400,
      height: lineHeight,
      color: c.withOpacity(op),
    );
  }
}

class SettingsSheet extends StatelessWidget {
  final LyricsPrefs prefs;
  const SettingsSheet({super.key, required this.prefs});

  static const _colors = [0xFFFFFFFF, 0xFFFFE082, 0xFF80DEEA, 0xFFA5D6A7, 0xFFF48FB1];

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: prefs,
      builder: (context, _) => ListView(
        padding: const EdgeInsets.all(20),
        shrinkWrap: true,
        children: [
          Text('Paroles', style: Theme.of(context).textTheme.titleLarge),
          const SizedBox(height: 12),
          Text('Aperçu de la ligne active', textAlign: prefs.alignment, style: prefs.style(active: true)),
          Text('ligne suivante', textAlign: prefs.alignment, style: prefs.style(active: false)),
          const Divider(height: 32),
          _slider('Taille', prefs.fontSize, 14, 56, (v) => prefs.update((p) => p.fontSize = v)),
          _slider('Espacement des lignes', prefs.lineSpacing, 0, 48, (v) => prefs.update((p) => p.lineSpacing = v)),
          _slider('Hauteur de ligne', prefs.lineHeight, 0.9, 2.0, (v) => prefs.update((p) => p.lineHeight = v)),
          _slider('Opacité des autres lignes', prefs.inactiveOpacity.toDouble(), 10, 90,
              (v) => prefs.update((p) => p.inactiveOpacity = v.round())),
          Row(children: [
            const Expanded(child: Text('Police')),
            DropdownButton<String>(
              value: prefs.font,
              items: [for (final f in LyricsPrefs.fonts) DropdownMenuItem(value: f, child: Text(f))],
              onChanged: (v) => prefs.update((p) => p.font = v!),
            ),
          ]),
          SwitchListTile(contentPadding: EdgeInsets.zero, title: const Text('Gras'), value: prefs.bold,
              onChanged: (v) => prefs.update((p) => p.bold = v)),
          SwitchListTile(contentPadding: EdgeInsets.zero, title: const Text('Contraste élevé'), value: prefs.highContrast,
              onChanged: (v) => prefs.update((p) => p.highContrast = v)),
          SwitchListTile(contentPadding: EdgeInsets.zero, title: const Text('Surligner mot par mot (si disponible)'),
              value: prefs.showWords, onChanged: (v) => prefs.update((p) => p.showWords = v)),
          const SizedBox(height: 8),
          SegmentedButton<TextAlign>(
            segments: const [
              ButtonSegment(value: TextAlign.left, icon: Icon(Icons.format_align_left)),
              ButtonSegment(value: TextAlign.center, icon: Icon(Icons.format_align_center)),
              ButtonSegment(value: TextAlign.right, icon: Icon(Icons.format_align_right)),
            ],
            selected: {prefs.alignment},
            onSelectionChanged: (s) => prefs.update((p) => p.alignment = s.first),
          ),
          const SizedBox(height: 16),
          Wrap(spacing: 12, children: [
            for (final c in _colors)
              GestureDetector(
                onTap: () => prefs.update((p) => p.activeColor = c),
                child: CircleAvatar(
                  radius: 16,
                  backgroundColor: Color(c),
                  child: prefs.activeColor == c ? const Icon(Icons.check, size: 18, color: Colors.black) : null,
                ),
              ),
          ]),
        ],
      ),
    );
  }

  Widget _slider(String label, double v, double min, double max, ValueChanged<double> f) => Row(children: [
        SizedBox(width: 150, child: Text(label)),
        Expanded(child: Slider(value: v.clamp(min, max), min: min, max: max, onChanged: f)),
        SizedBox(width: 40, child: Text(v.toStringAsFixed(max <= 2 ? 2 : 0))),
      ]);
}
