# Reis

**Reconnaissance de sons 100 % hors ligne, à partir de ta propre bibliothèque.**
Tu donnes un dossier de sons de référence (musiques, cris d'animaux, bruits,
alarmes, jingles…). Reis en fait une bibliothèque d'empreintes, puis reconnaît
ces sons dans un fichier ou au micro, sans Internet.

```text
mes-sons/                          reis index mes-sons -o sons.reis
├── animaux/chien/*.wav     ──▶    sons.reis  (un seul fichier : empreintes,
├── animaux/chat/*.mp3                         métadonnées, paroles, jaquettes, modèle)
├── maison/sonnette.wav
└── musique/*.flac (+ .lrc)        reis recognize sons.reis inconnu.wav
                                   ≈ similar: animaux/chien (score 0.71)
```

Deux moteurs complémentaires, choisis et réglés par mesure (pas par mode) :

| | moteur **exact** | moteur **sémantique** |
|---|---|---|
| question | *est-ce cet enregistrement ?* | *est-ce ce genre de son ?* |
| technique | constellation de pics temps-fréquence (type Shazam) | réseau de neurones audio EfficientAT (2023), embeddings + plus proches voisins |
| idéal pour | morceaux, jingles, annonces → **position** et **paroles synchronisées** | animaux, bruits, alarmes → reconnaît un **autre** chien que ceux de la bibliothèque |
| mesuré | 99 % à 10 s, 90 % à 5 s, position ±5 ms, 0 faux positif | 88,8 % de bonnes catégories (ESC-50, 40 classes), 75,9 % via haut-parleur |

Plus un filet de sécurité : les **527 classes AudioSet** du réseau
(« absent de ta bibliothèque, mais ressemble à : Cat 66 % »).

## En 5 minutes

1. Télécharge `reis-cli-…` dans la [release](https://github.com/remigl/reis/releases)
   (exécutable + modèles) et, pour les formats autres que WAV,
   [ffmpeg](https://ffmpeg.org).
2. Range tes sons : **un sous-dossier = une catégorie**.
3. Construis la bibliothèque, puis reconnais un fichier, puis écoute au micro :
   ```bash
   reis index mes-sons/ -o sons.reis
   reis recognize sons.reis enregistrement.wav
   ffmpeg -f dshow -i audio="Microphone" -ac 1 -ar 16000 -f s16le - | reis listen sons.reis   # Windows
   arecord -f S16_LE -r 16000 -c 1 -t raw | reis listen sons.reis                             # Linux
   ```
4. **Ou directement sur le téléphone** : installe l'APK de la release, touche
   *Créer ma bibliothèque*, coche tes dossiers de musiques ou de sons. L'app
   crée les empreintes elle-même, hors ligne, et peut chercher les paroles
   manquantes sur LRCLIB (seule fonction en ligne, facultative).

## Documentation

| | |
|---|---|
| [Guide d'utilisation](docs/GUIDE.md) | préparer les sons, toutes les commandes, seuils, recettes (musique + paroles, oiseaux, maison…), dépannage |
| [Comment ça marche](docs/CONCEPTS.md) | les deux empreintes, la décision, la calibration automatique, le temps réel |
| [Intégrer Reis](docs/API.md) | ABI C (Python, C#, Kotlin…), format JSON, bibliothèque Rust, Flutter |
| [Architecture](docs/ARCHITECTURE.md) | modules, flux de données, choix techniques, **points d'extension** |
| [Formats](docs/FORMATS.md) | fichiers `.reis` et `.rnm`, octet par octet |
| [Modèles](docs/MODELS.md) | modèles fournis, vérification contre PyTorch, convertir un autre réseau |
| [Benchmarks](docs/BENCHMARKS.md) | toutes les mesures, leurs conditions et leurs limites |
| [Données et licences](docs/DATA_LICENSES.md) | ce qui est redistribuable, et ce qui ne l'est pas |
| [Contribuer](CONTRIBUTING.md) | compiler, tester, ajouter un moteur ou un modèle |

## Structure

```text
core/          bibliothèque Rust sans dépendance : empreintes, réseau de neurones, index, décision, ABI C
cli/           l'outil `reis`
app/flutter/   application Android / Windows / Linux (ne parle au moteur que via l'ABI C)
tools/         conversion des modèles, paroles LRCLIB, exemple Python
benchmarks/    corpus, dégradations réalistes, protocoles ESC-50 et musique, résultats
docs/          documentation
```

## État et limites

- Cœur, CLI, benchmarks : fonctionnels et testés (`cargo test`). Le runtime
  neuronal reproduit PyTorch (cosinus 0,999998).
- Mesures sur ESC-50 (vrais sons) et sur un corpus musical **synthétique** ;
  pas encore sur une vraie bibliothèque musicale ni sur téléphone.
- App Flutter compilée par la CI (APK + Windows), création de bibliothèque sur
  le téléphone comprise, mais jamais testée sur un vrai appareil.
- Aucun catalogue public n'est fourni : il n'existe pas de base libre de
  musique commerciale (voir [DATA_LICENSES.md](docs/DATA_LICENSES.md)).

Licence du code : MIT ou Apache-2.0. Modèles EfficientAT : MIT.
