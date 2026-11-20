# Guide d'utilisation

Ce guide va de l'installation à la reconnaissance au micro. Pour comprendre
*comment* Reis reconnaît un son, lis [CONCEPTS.md](CONCEPTS.md) ; pour intégrer
Reis dans un autre programme, [API.md](API.md).

- [1. Installation](#1-installation)
- [2. Préparer une bibliothèque de sons](#2-préparer-une-bibliothèque-de-sons)
- [3. Construire la bibliothèque d'empreintes : `reis index`](#3-construire-la-bibliothèque-dempreintes--reis-index)
- [4. Reconnaître : `reis recognize`](#4-reconnaître--reis-recognize)
- [5. Écouter en direct : `reis listen`](#5-écouter-en-direct--reis-listen)
- [6. Sans bibliothèque : `reis tag`](#6-sans-bibliothèque--reis-tag)
- [7. Mesurer la qualité : `reis eval`](#7-mesurer-la-qualité--reis-eval)
- [8. Régler les seuils](#8-régler-les-seuils)
- [9. Recettes](#9-recettes)
- [10. L'application mobile / Windows](#10-lapplication-mobile--windows)
- [11. Problèmes fréquents](#11-problèmes-fréquents)

---

## 1. Installation

**Le plus simple** : télécharge l'archive de la [dernière release](https://github.com/remigl/reis/releases)
(`reis-cli-…-windows-x64.zip` ou `…-linux-x86_64.tar.gz`). Elle contient :

```text
reis(.exe)
models/mn10_as.rnm   modèle neuronal par défaut (10 Mo)
models/mn04_as.rnm   modèle léger (2,3 Mo)
```

Reis cherche le modèle dans cet ordre : option `--model`, variable
`REIS_MODEL`, `./models/`, dossier `models/` à côté de l'exécutable,
`~/.reis/models/`.

**ffmpeg** (recommandé) : Reis lit le WAV tout seul ; pour MP3, FLAC, OGG,
Opus, M4A… il appelle `ffmpeg` s'il est installé (`winget install ffmpeg`,
`sudo apt install ffmpeg`, `brew install ffmpeg`). ffmpeg sert aussi à lire les
tags et à réduire les jaquettes.

**Depuis les sources** (Rust ≥ 1.80, aucune dépendance à télécharger) :

```bash
git clone https://github.com/remigl/reis && cd reis
cargo build --release          # → target/release/reis
cargo test --release
```

---

## 2. Préparer une bibliothèque de sons

Une bibliothèque, c'est **un dossier**. Règle unique : **le chemin du dossier
parent d'un fichier est sa catégorie** (son *label*).

```text
mes-sons/
├── reis.json                         (facultatif) réglages, voir plus bas
├── animaux/
│   ├── chien/
│   │   ├── aboiement-01.wav          label « animaux/chien »
│   │   ├── aboiement-02.mp3
│   │   └── grognement.flac
│   └── chat/miaou.ogg                label « animaux/chat »
├── oiseaux/
│   ├── merle.flac                    label « oiseaux »
│   └── merle.json                    (facultatif) métadonnées de merle.flac
├── maison/
│   ├── sonnette.wav                  label « maison »
│   └── alarme-incendie.wav
└── musique/
    ├── One More Time.mp3             label « musique »
    ├── One More Time.lrc             (facultatif) paroles synchronisées
    └── cover.jpg                     (facultatif) jaquette de tout le dossier
```

- **Formats** : wav, mp3, flac, ogg, oga, opus, m4a, aac, aif(f), wma, ape, wv, mka, webm, mp4 (piste audio).
- **Fichiers à la racine** : leur label est leur propre nom (utile pour « un morceau = une réponse »).
- **Fichiers cachés** (`.xxx`) ignorés.
- **Combien d'exemples ?** Pour reconnaître *un type de son* (chien, perceuse),
  5 à 40 enregistrements variés par catégorie : différents individus,
  distances, pièces. Pour reconnaître *un enregistrement précis* (un morceau,
  un jingle), un seul fichier suffit.
- **Durée** : 1 s minimum. Les sons longs sont échantillonnés (32 fenêtres
  maximum par fichier pour le moteur sémantique, tout le fichier pour le moteur
  exact).
- **Silence** : inutile de couper, les passages silencieux sont ignorés
  automatiquement.

### Métadonnées : `<même nom>.json`

N'importe quelles clés, conservées dans la bibliothèque et renvoyées à la
reconnaissance. Deux clés spéciales :

```json
{
  "label": "oiseaux/merle noir",      // remplace le label déduit du dossier
  "name":  "Merle noir — chant matinal",  // nom affiché
  "espece": "Turdus merula",
  "lieu": "Bruxelles",
  "url": "https://…"
}
```

Pour la musique, `title`, `artist` et `album` sont affichés par l'app. Avec
`reis index --tags`, ils sont lus automatiquement dans les tags du fichier
(via ffprobe) ; le `.json` reste prioritaire.

### Paroles : `<même nom>.lrc`

Format LRC standard (`[mm:ss.xx]ligne`), y compris le LRC « enhanced » mot à
mot (`<mm:ss.xx>mot`) et la balise `[offset:±ms]`. Pour en récupérer
automatiquement depuis un dump LRCLIB local : `tools/lrclib_extract.py`
(attention aux droits, voir [DATA_LICENSES.md](DATA_LICENSES.md)).

### Images : `<même nom>.jpg|png|webp` ou `cover.jpg` / `folder.jpg`

Réduites à 160 × 160 px en WebP (≈ 4 Ko) si ffmpeg est là, et dédupliquées :
une pochette d'album partagée par 12 morceaux n'est stockée qu'une fois.

### Réglages : `reis.json` à la racine (facultatif)

```json
{
  "engines": ["exact", "semantic"],
  "exact":    { "peaks_per_sec": 30, "fanout": 3 },
  "semantic": { "window_s": 3, "hop_s": 1.5, "max_windows": 32,
                "max_false_accept": 0.2, "top_m": 3 },
  "augment": ["speaker"],
  "use_tags": true,
  "max_secs": 0
}
```

Les options de la ligne de commande l'emportent sur ce fichier. Signification
de chaque réglage : section suivante.

---

## 3. Construire la bibliothèque d'empreintes : `reis index`

```bash
reis index mes-sons/ -o sons.reis
```

Exemple réel (ESC-50 : 1 280 sons de 5 s, 40 catégories) :

```text
model models/mn10_as.rnm
1280 reference files, 40 labels, engines: exact + semantic
  fingerprinted 1280/1280
wrote sons.reis: 15.85 MB, 107 min of audio in 139.5 s (46x real time)
library: 1280 references, 40 labels, engines exact + semantic, 15.85 MB
  semantic: model mn10_as, 3603 vectors x 960 dims, window 3 s; threshold 0.605 (≤ 20% unknown sounds accepted), leave-one-out accuracy 93.2%
  labels: breathing (32), brushing_teeth (32), can_opening (32), car_horn (32), …
```

Le résultat est **un seul fichier** : empreintes, métadonnées, paroles,
jaquettes **et le modèle neuronal**. On le copie tel quel sur un téléphone.

| option | défaut | rôle |
|---|---|---|
| `-o FICHIER` | `library.reis` | fichier produit |
| `--engines exact,semantic` | les deux | moteurs à construire (voir [CONCEPTS.md](CONCEPTS.md)) |
| `--model FICHIER.rnm` | recherche auto | modèle du moteur sémantique |
| `--tags` | non | lire titre/artiste/album dans les tags (musique) |
| `--augment speaker` | non | ajoute pour chaque référence une version « jouée sur un petit haut-parleur dans une pièce » (voir §8) |
| `--window S` / `--hop S` | 3 / 1,5 | découpage en fenêtres du moteur sémantique |
| `--max-windows N` | 32 | fenêtres max par fichier (les longs fichiers sont échantillonnés régulièrement) |
| `--false-accept F` | 0,2 | taux de sons inconnus acceptés visé par le seuil automatique (estimation prudente, voir §8) |
| `--pps N` / `--fanout N` | 30 / 3 | densité du moteur exact (taille ↔ robustesse, voir [BENCHMARKS.md](BENCHMARKS.md)) |
| `--max-secs S` | 0 (tout) | n'analyser que le début des fichiers |
| `--threads N` | tous les cœurs | parallélisme |

**Quel(s) moteur(s) choisir ?**

| ta bibliothèque | moteurs | pourquoi |
|---|---|---|
| des morceaux, jingles, annonces précis | `exact` | identifie l'enregistrement **et la position** (paroles synchronisées), index petit, très rapide |
| des *types* de sons (animaux, bruits, alarmes) | `semantic` | reconnaît un **autre** chien que ceux de la bibliothèque |
| un peu des deux | `exact,semantic` (défaut) | Reis prend la réponse la plus précise |

Pour info : `reis info sons.reis [--refs]` affiche le contenu, la taille par
section, la calibration et la liste des références.

---

## 4. Reconnaître : `reis recognize`

```bash
reis recognize sons.reis enregistrement.wav autre.mp3
```

```text
enregistrement.wav
  ≈ similar: animaux/chien (score 0.71, threshold 0.61)
      labels: animaux/chien 0.71 · animaux/cochon 0.25 · maison 0.24
      nearest: aboiement-02.mp3 0.73 · grognement.flac 0.70 · aboiement-01.wav 0.70
      exact: best "sonnette" score 3 (second 2) — not accepted
      tags: Animal 58% · Dog 46% · Domestic animals, pets 39%
      (exact 16 ms, semantic 115 ms, tags 56 ms)
```

**Le verdict** (première ligne) est l'un de :

| symbole | verdict | signification |
|---|---|---|
| `✓ exact` | même enregistrement qu'une référence | avec la position dans la référence (`at 23.39 s`) |
| `≈ similar` | même **type** de son qu'une catégorie | score ≥ seuil de la bibliothèque |
| `?` | absent de la bibliothèque | mais le modèle généraliste pense à … (527 classes AudioSet) |
| `·` | silence | niveau trop faible pour analyser |
| `✗` | inconnu | rien de fiable |

Les lignes suivantes donnent le détail : meilleures catégories, références les
plus proches, meilleur candidat exact même refusé, tags généraux, temps de calcul.

| option | rôle |
|---|---|
| `--json` | une ligne JSON par résultat (schéma : [API.md](API.md#format-json)) |
| `--segments S` | découpe un long fichier en tranches de S secondes et reconnaît chacune → chronologie |
| `--start S --duration D` | n'analyser qu'un passage |
| `--engines exact,semantic,tags` | moteurs utilisés à la requête |
| `--threshold T` | seuil sémantique fixe (remplace la calibration) |
| `--false-accept F` | seuil choisi par taux de faux positifs, sur la courbe de calibration |
| `--top N` | nombre de candidats affichés (5) |

La sortie ci-dessus est réelle (un aboiement d'ESC-50 joué sur un haut-parleur
simulé) ; seuls les noms de fichiers ont été adaptés à l'exemple.

Exemple : quels sons dans une heure d'enregistrement de jardin ?

```bash
reis recognize oiseaux.reis jardin.flac --segments 5 | grep -v "silence"
```

---

## 5. Écouter en direct : `reis listen`

`reis listen` lit du PCM brut 16 bits mono sur l'entrée standard : n'importe
quel programme de capture peut l'alimenter.

```bash
# Linux
arecord -f S16_LE -r 16000 -c 1 -t raw | reis listen sons.reis
# Windows (nom du micro : ffmpeg -list_devices true -f dshow -i dummy)
ffmpeg -f dshow -i audio="Microphone (Realtek Audio)" -ac 1 -ar 16000 -f s16le - | reis listen sons.reis
# macOS
ffmpeg -f avfoundation -i ":0" -ac 1 -ar 16000 -f s16le - | reis listen sons.reis
```

- bibliothèque avec moteur **exact** : verrouillage en ~2–3 s, position suivie
  en continu, paroles ligne par ligne, détection des pauses ;
- moteur **sémantique** : une ligne à chaque fois que le son détecté change
  (analyse de la dernière fenêtre de 3 s, toutes les `--every` secondes).

Sortie réelle (trois sons d'ESC-50 joués à la suite : chien, coq, cloches) :

```text
listening… (Ctrl-C to quit)
[   2.8s] ≈ dog (0.69)
[   6.9s] ≈ rooster (0.73)
[   9.9s] ? Gargling (53%)          ← transition entre deux sons
[  11.0s] ≈ church_bells (0.75)
```

Options : `--rate` (16000), `--every` (1 s), `--no-exact`, `--no-semantic`,
`--battery` (micro coupé entre deux vérifications du moteur exact).

---

## 6. Sans bibliothèque : `reis tag`

```bash
reis tag inconnu.wav
inconnu.wav: Cat 66% · Domestic animals, pets 62% · Animal 60% · Meow 43% · Speech 13%   (65 ms)
```

Le modèle seul, avec ses 527 classes [AudioSet](https://research.google.com/audioset/ontology/index.html)
(animaux, instruments, véhicules, outils, voix, environnements…). Pratique pour
explorer, ou en secours quand un son n'est pas dans ta bibliothèque. Les sons
stationnaires (bruit blanc, souffle) font « saturer » ce type de modèle ; Reis
le détecte et n'affiche alors aucun tag plutôt que des tags absurdes.

---

## 7. Mesurer la qualité : `reis eval`

Prépare un **dossier de test** organisé comme la bibliothèque (mêmes noms de
catégories), avec des enregistrements **qui ne sont pas dans la bibliothèque**.
Mets-y aussi des catégories absentes de la bibliothèque : elles mesurent les
faux positifs.

```bash
reis eval sons.reis test/ --per-label --csv resultats.csv
```

```text
queries: 320 with a known label, 80 with a label absent from the library
top-1 (accepted and correct): 56.9%
top-5 (correct label among candidates): 97.8%
wrong label accepted: 3.1%
false accepts on unknown sounds: 7.5%
time per query p50 104.8 ms, p95 133.9 ms
  breathing                      75% (6/8)
  brushing_teeth                 12% (1/8)
  …
```

(ESC-50, modèle mn10, seuil calibré par défaut. Sans rejet, `--threshold -1`,
la bonne catégorie arrive en tête dans 88,8 % des cas.)

- `top-1` = accepté **et** juste ; `top-5` = la bonne catégorie est parmi les
  candidats même si le seuil l'a refusée.
- `--threshold -1` désactive le rejet : tu obtiens la précision « en ensemble
  fermé » et, dans le CSV, tous les scores pour choisir ton seuil
  (`benchmarks/openset.py` trace le compromis).

---

## 8. Régler les seuils

Le moteur sémantique doit trancher entre « c'est un chien » et « ce n'est rien
que je connais ». Reis choisit le seuil **tout seul**, à la construction :
pour chaque référence, il cherche la meilleure correspondance en retirant
toute sa catégorie de la bibliothèque — c'est le score d'un « imposteur »,
d'un son inconnu. Le seuil laisse passer au plus `--false-accept` de ces
imposteurs (20 % par défaut).

Cette estimation est **prudente** : sur ESC-50, 20 % estimés correspondent à
≈ 7 % de sons réellement inconnus acceptés. Pour ajuster sans reconstruire :

```bash
reis recognize sons.reis x.wav --false-accept 0.3    # plus permissif
reis recognize sons.reis x.wav --false-accept 0.05   # plus strict
reis recognize sons.reis x.wav --threshold 0.55      # valeur fixe
```

Repères (ESC-50, 40 catégories, modèle mn10) :

| priorité | réglage | effet mesuré |
|---|---|---|
| ne jamais se tromper | `--false-accept 0.05` | ~1 % d'inconnus acceptés, mais beaucoup de « ? » |
| équilibre (défaut) | `--false-accept 0.2` | ~7 % d'inconnus acceptés |
| toujours une réponse | `--threshold -1` | 88,8 % de bonnes réponses en son propre, aucun rejet |

**Augmentation `--augment speaker`** : si les sons seront entendus *à travers
un haut-parleur* (enceinte, téléphone, télé), indexer aussi une version
simulée « haut-parleur + pièce + micro » de chaque référence. Mesuré sur
ESC-50 : 75,9 % → 81,9 % de bonnes réponses via haut-parleur. Contrepartie :
la bibliothèque double de volume pour ce moteur et le rejet des sons inconnus
devient un peu plus difficile.

**Moteur exact** : règle `score ≥ 13` et `score ≥ 2,2 × (second + 1)`, calibrée
pour 0 faux positif sur 6 000 tentatives (voir [BENCHMARKS.md](BENCHMARKS.md)).
Ces valeurs sont dans `RecognizerConfig` / `EngineConfig`.

---

## 9. Recettes

### Musique avec paroles synchronisées (l'usage « Shazam hors ligne »)

```bash
python3 tools/lrclib_extract.py lrclib-dump.sqlite3 ~/Musique   # .lrc à côté des fichiers
reis index ~/Musique -o musique.reis --engines exact --tags
reis listen musique.reis < micro       # ou l'app : importer musique.reis
```

Moteur exact seul : ≈ 23 Ko par minute de musique (≈ 22 000 titres dans 2 Go).

### Oiseaux du jardin

```text
oiseaux/merle/…  oiseaux/mésange charbonnière/…  oiseaux/rouge-gorge/…
```
```bash
reis index oiseaux/ -o oiseaux.reis --engines semantic
reis recognize oiseaux.reis matin.flac --segments 3
```
10 à 30 enregistrements par espèce (xeno-canto en propose sous licences Creative Commons).

### Sons de la maison (sonnette, alarme, bébé, chien)

```bash
reis index maison/ -o maison.reis --engines semantic --augment speaker
reis listen maison.reis --every 0.5 --no-exact | grep "≈"   # → déclencher une action
```

### Identifier les morceaux d'une émission ou d'un mix

```bash
reis recognize musique.reis emission.mp3 --segments 10 --json > chronologie.jsonl
```

### Intégrer dans ton programme

Bibliothèque Rust (`reis-core`), ABI C (Python, C#, Kotlin, Swift…) ou
Dart/Flutter : voir [API.md](API.md).

---

## 10. L'application mobile / Windows

1. Construis ta bibliothèque sur ordinateur (`reis index …`).
2. Installe l'APK / dézippe l'app Windows depuis la release.
3. Premier lancement : **Importer un catalogue** → choisis ton `.reis`.
4. L'écoute démarre toute seule :
   - bibliothèque **exact** : jaquette, titre, paroles synchronisées ;
   - bibliothèque **sémantique** : le nom du son détecté, en grand, et les
     catégories proches.

Réglages des paroles : bouton `Aa` (police, taille, gras, espacements,
alignement, couleur, contraste), conservés après redémarrage.

---

## 11. Problèmes fréquents

| symptôme | cause / solution |
|---|---|
| `no neural model found` | télécharge `mn10_as.rnm` (release `models-v1`) dans `models/`, ou `--model`, ou indexe avec `--engines exact` |
| `cannot run ffmpeg` | installe ffmpeg, ou convertis tes fichiers en WAV |
| `library format v1 …` | bibliothèque créée par Reis 0.1 : reconstruis-la avec `reis index` |
| tout est « ? » | seuil trop strict pour tes données : `--false-accept 0.3`, ou plus d'exemples par catégorie |
| deux catégories se confondent | `reis eval --per-label` les montre ; ajoute des exemples variés, ou fusionne-les si elles sont réellement indiscernables |
| son entendu via haut-parleur mal reconnu | reconstruis avec `--augment speaker` |
| musique reconnue mais position fausse | une référence trop courte ou un remix : vérifie que la référence est bien le même enregistrement |
| indexation lente | `--engines exact` (≈ 350× temps réel), modèle `mn04_as.rnm` (≈ 3× plus rapide), `--max-windows 8` |
