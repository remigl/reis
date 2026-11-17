# Reis

Reconnaissance musicale **100 % hors ligne** avec **paroles synchronisées**.
On ouvre l'app → le micro écoute → le morceau est reconnu en ~2–3 s → jaquette,
titre, artiste, album et paroles qui défilent à la bonne ligne.

```text
micro ─▶ 8 kHz ─▶ empreinte « constellation » ─▶ index local (mmap)
                                   │ titre + position (±5 ms)
        position = offset + temps écoulé, re-vérifiée toutes les 15 s
                                   │
         paroles (ligne / mot) · métadonnées · jaquette — même fichier .reis
```

## État

| brique | état |
|---|---|
| Cœur Rust (`core/`) : DSP, empreinte, index, matching, position, moteur temps réel, paroles, catalogue, ABI C | ✅ fonctionnel, testé, benchmarké — **0 dépendance**, `.so` de 410 Ko |
| CLI (`reis`) : build, id, bench, simulate, listen (micro réel via ffmpeg), scale | ✅ |
| Benchmarks reproductibles (landmarks vs Chromaprint, 10 conditions × 4 durées, moteur, 30 k titres) | ✅ `docs/BENCHMARKS.md` |
| Index builder (bibliothèque → tags/jaquettes → LRCLIB → `.reis`) | ✅ scripts ; testé sur le corpus synthétique, pas sur un vrai dump LRCLIB |
| App Flutter (`app/flutter/`) : écoute auto, isolate moteur, paroles fluides, réglages persistés | ⚠️ code complet mais **non compilé** : pas de SDK Flutter dans l'environnement de développement |

## Résultats clés (détails et limites : [docs/BENCHMARKS.md](docs/BENCHMARKS.md))

Mesuré sur un corpus **synthétique** (aucun enregistrement réel accessible) et
un CPU x86 ; téléphone non mesuré.

* **Landmarks retenus, Chromaprint écarté** : à 10 s via haut-parleur,
  100 % contre 5 % ; à 2 s, 55 % contre 0 % (Chromaprint ne produit aucune
  empreinte).
* Défaut, règle stricte : **top-1 55 / 76 / 90 / 99 %** à 2 / 3 / 5 / 10 s
  toutes conditions confondues, **0 faux positif** (800 négatifs) et 0 mauvais titre.
* Moteur réel simulé : résultat en **2,0–3,5 s** (p50), 20/20 même haut-parleur +
  bruit à 5 dB ; position à **7–10 ms** ; pause détectée 20/20 ; 0 faux
  verrouillage sur 60 morceaux hors catalogue écoutés 55 s.
* 30 000 titres : précision identique (79,8 % vs 80,0 %), 55 ms pour 5 s d'audio.
* Taille : **≈ 89 Ko par titre** tout compris → **≈ 22 000 titres sous 2 Go**
  (≈ 32 000 en préréglage compact).

## Important : d'où vient le catalogue

AcoustID ne peut pas servir de base de reconnaissance au micro : ses empreintes
(Chromaprint) ne survivent pas au trajet haut-parleur → micro (mesuré
ci-dessus), et l'empreinte landmark se calcule **à partir de l'audio**. Le
catalogue se construit donc depuis une bibliothèque audio (la tienne, ou un
fonds dont on a les droits). MusicBrainz / AcoustID servent aux métadonnées
(via Picard), LRCLIB aux paroles. Licences : [docs/DATA_LICENSES.md](docs/DATA_LICENSES.md)
— les paroles et jaquettes ne sont pas redistribuables par défaut.

## Démarrage

```bash
cargo build --release                       # core + CLI, aucun téléchargement de crate
cargo test --release                        # tests unitaires + bout-en-bout

# Catalogue depuis ta musique (ffmpeg requis pour MP3/FLAC/…)
python3 index-builder/scan_library.py ~/Music build/
python3 index-builder/lrclib_extract.py lrclib-dump.sqlite3 build/meta.tsv   # facultatif
target/release/reis build --meta build/meta.tsv --out catalog.reis

# Identifier un fichier
target/release/reis id catalog.reis extrait.mp3

# Écoute en direct au micro avec paroles dans le terminal
#   Linux :   arecord -f S16_LE -r 16000 -c 1 -t raw | target/release/reis listen catalog.reis
#   Windows : ffmpeg -f dshow -i audio="Microphone (…)" -ac 1 -ar 16000 -f s16le - | reis listen catalog.reis
```

App Flutter :

```bash
scripts/setup_flutter.sh        # génère android/ios/desktop + permission micro
scripts/build_android.sh        # cargo-ndk → jniLibs
cd app/flutter && flutter run   # au 1er lancement : importer catalog.reis
```

## Structure

```text
core/            Rust : audio, fft, fingerprint, index, engine, lyrics, catalog, ffi
cli/             outil `reis` (build, id, bench, simulate, listen, scale, diag)
app/flutter/     UI (remplaçable : elle ne parle au moteur que via l'ABI C)
index-builder/   construction du catalogue (outils lourds, hors app)
benchmarks/      corpus, dégradations, Chromaprint, calibration, simulation + résultats
scripts/         builds Android/desktop, reproduction des benchmarks
docs/            ARCHITECTURE, BENCHMARKS, DATA_LICENSES
```

## Prochaines étapes

1. Refaire `scripts/reproduce_benchmarks.sh` sur une **vraie** bibliothèque
   (quelques centaines de titres) et réajuster la règle d'acceptation.
2. Compiler l'app Flutter, mesurer CPU / RAM / batterie sur un vrai téléphone.
3. Benchmarker une empreinte neuronale légère en comparaison.

Licence du code : MIT ou Apache-2.0, au choix.
