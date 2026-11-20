# Architecture

Objectif : un moteur **petit, sans dépendance, lisible**, que l'on peut
modifier morceau par morceau. Tout le cœur tient dans `core/src/` (≈ 4 300
lignes de Rust, bibliothèque standard uniquement).

```text
 ┌──────────────── interfaces (remplaçables) ─────────────────┐
 │ cli/ (reis)      app/flutter (Android, Windows…)   ton programme │
 └───────┬──────────────────┬──────────────────────────┬──────┘
         │ Rust             │ dart:ffi                 │ C ABI (ffi.rs)
 ┌───────▼──────────────────▼──────────────────────────▼──────┐
 │ recognize.rs   Recognizer : exact + sémantique + tags → verdict │
 │ engine.rs      moteur temps réel (écoute / verrouillé / pause)  │
 ├──────────────────────────────────────────────────────────────┤
 │ library.rs     dossier → .reis (scan, build, lecture mmap)      │
 ├──────────────┬──────────────────────┬───────────────────────┤
 │ exact        │ semantic.rs           │ lyrics.rs  paroles LRC │
 │ fingerprint.rs│  fenêtres, centrage,  │ (artwork dans library) │
 │ index.rs      │  int8, kNN, calibration│                       │
 ├──────────────┼──────────────────────┴───────────────────────┤
 │              │ nn.rs  runtime CNN (.rnm) : log-mel + conv/SE/… │
 ├──────────────┴──────────────────────────────────────────────┤
 │ audio.rs (WAV, rééchantillonnage, augmentation)  fft.rs  json.rs │
 │ bytes.rs (varints, mmap)   decode.rs (ffmpeg/ffprobe, build seulement) │
 └──────────────────────────────────────────────────────────────┘
```

## Modules

| fichier | rôle | dépend de |
|---|---|---|
| `fft.rs` | FFT radix-2 itérative | — |
| `audio.rs` | lecture WAV, rééchantillonneur sinc à table, simulation haut-parleur + pièce, convolution FFT | fft |
| `json.rs` | lecture/écriture JSON (en-têtes, métadonnées, sorties) | — |
| `bytes.rs` | varints, lecture little-endian, fichier mappé en mémoire | — |
| `decode.rs` | décodage de tous formats et tags via ffmpeg/ffprobe — utilisé uniquement pour construire | json |
| `fingerprint.rs` | empreinte constellation : spectrogramme, pics, hashes ; `FpParams` | fft |
| `index.rs` | index inversé compressé + vote par décalage | bytes |
| `nn.rs` | runtime de réseaux convolutifs audio (format `.rnm`) | fft, json, audio |
| `semantic.rs` | moteur sémantique : fenêtres, embeddings, int8, recherche, calibration | nn, json |
| `lyrics.rs` | LRC (+ mot à mot) ↔ format binaire compact | bytes |
| `library.rs` | scan d'un dossier, `LibraryBuilder`, format `.reis`, `Library` | tout ce qui précède |
| `recognize.rs` | `Recognizer` : orchestration des moteurs et décision | library |
| `engine.rs` | moteur de flux pour le micro (moteur exact) | library |
| `ffi.rs` | ABI C : bibliothèque, reconnaissance JSON, moteur de flux, paroles | tout |

Le CLI (`cli/src/`) n'est qu'une couche fine : `cmds.rs` (commandes
courantes), `research.rs` (benchmarks), `util.rs`.

## Flux de données

**Construction** (`reis index`) :

```text
scan_dir(dossier) ─▶ SourceItem {chemin, label, nom, meta, .lrc, image}
      │ (threads)  decode::load → 32 kHz mono
      ▼
LibraryBuilder::prepare  ─┬─ 8 kHz → Fingerprinter::hashes ─────────▶ IndexBuilder
                          └─ Model::logmel → fenêtres → forward ────▶ SemanticBuilder
LibraryBuilder::add       ─ métadonnées, paroles (LRC→binaire), image (WebP, dédup)
LibraryBuilder::finish    ─ tri/compression de l'index, centrage+int8, calibration
                          ─▶ un fichier .reis (sections, voir FORMATS.md)
```

**Reconnaissance** (`Recognizer::recognize`) :

```text
audio (n'importe quel taux) ─┬─ 8 kHz → hashes (requête dense) → match_hashes → ExactHit
                             ├─ 32 kHz → fenêtres → vecteurs centrés → ref_scores → label_scores
                             └─ 32 kHz → réseau complet → 527 probabilités (tags)
                      decide(): Exact accepté > Similar ≥ seuil > Tag ≥ 0,3 > Unknown
```

## Choix techniques (et pourquoi)

| choix | raison | mesuré |
|---|---|---|
| Rust sans dépendance | binaire de 0,5 Mo, compile partout (Android NDK, iOS, Windows), rien à auditer | `libreis_core.so` 0,41 Mo |
| constellation plutôt que Chromaprint pour le moteur exact | Chromaprint ne survit pas au trajet haut-parleur → micro | 100 % vs 5 % à 10 s |
| réseau EfficientAT plutôt que YAMNet / VGGish | meilleur rapport qualité / calcul publié pour AudioSet (mAP 0,471 pour mn10) | 28 ms pour 3 s d'audio (x86, 2 cœurs) |
| runtime neuronal maison plutôt qu'ONNX Runtime / TFLite | 0 dépendance, + 300 lignes au lieu de + 10–20 Mo de binaire, même résultat | cosinus 0,999998 vs PyTorch |
| embeddings (960 d) plutôt que probabilités de classes | se généralise mieux aux catégories de l'utilisateur | 88,8 % vs 70,9 % |
| centrage + int8 | cosinus mieux répartis ; ¼ de la taille du float32 | seuils exploitables |
| recherche exhaustive | exacte, simple ; suffisante jusqu'à ~10⁵ fenêtres | — |
| un fichier `.reis` autonome (modèle inclus) | une seule chose à copier sur le téléphone | — |
| JSON pour les métadonnées | lisible, extensible sans changer de version | — |

## Points d'extension

### Ajouter un modèle neuronal

Aucune ligne de Rust si le réseau n'utilise que les opérations supportées
(convolutions groupées, squeeze-excitation, résiduels, pooling global, couches
denses, activations courantes) : écrire un exportateur qui produit un `.rnm`
(voir [MODELS.md](MODELS.md)). Exemple : `tools/models/export_efficientat.py`.
Une nouvelle opération = une variante de `enum Op` et son calcul dans
`Model::forward_mel` (`nn.rs`).

### Ajouter un moteur

Un moteur, c'est trois choses :
1. une étape `prepare` (calcul par référence, parallélisable) dans
   `LibraryBuilder::prepare` → un champ de `Prepared` ;
2. une section du fichier (`finish` : 4 lettres, ex. `CHRM`) ;
3. une lecture + recherche dans `Recognizer::recognize`, qui alimente
   `Recognition` et `decide`.

Exemples d'évolutions possibles : empreinte de reprises (chroma), index
approximatif (IVF / HNSW) pour très grandes bibliothèques, détection
d'événements multiples.

### Changer l'interface

L'app Flutter ne connaît que l'ABI C (`ffi.rs`) : une autre interface
(Kotlin natif, SwiftUI, web via WASM, Python via `ctypes`) peut la remplacer
sans toucher au moteur. La fonction la plus simple, `reis_recognize_f32`,
renvoie tout le résultat en JSON.

### Changer la règle de décision

`Recognizer::decide` (10 lignes). Les seuils sont dans `RecognizerConfig`
(requête) et dans la calibration de la bibliothèque (construction).

## Tests

```bash
cargo test --release      # unitaires (FFT, rééchantillonnage, index, LRC, JSON,
                          #   f16, convolutions, augmentation) + bout en bout
reis model-check models/mn10_as.rnm models/mn10_as.ref.bin   # runtime vs PyTorch
```

Les benchmarks (`benchmarks/`) sont des tests de qualité reproductibles :
voir [BENCHMARKS.md](BENCHMARKS.md).
