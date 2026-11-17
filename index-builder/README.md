# Index builder

Outils lourds, exécutés **une fois** sur un ordinateur pour produire le
catalogue `catalog.reis`. Rien ici n'est embarqué dans l'application.

```text
audio (votre bibliothèque)      LRCLIB dump (SQLite)     jaquettes intégrées
        │                              │                        │
  scan_library.py  ── tags (ffprobe), MBID, jaquette → WebP 160 px, dédupliquée
        │
  lrclib_extract.py ── appariement artiste/titre/durée ±2 s → .lrc synchronisés
        │
  reis build  ── décodage (ffmpeg) → 8 kHz → empreintes (multi-thread)
        │        → index compact + métadonnées + paroles + jaquettes
        ▼
  catalog.reis (un seul fichier, mmap)
```

## Étapes

```bash
# 0. (recommandé) taguer la bibliothèque avec MusicBrainz Picard : il utilise
#    AcoustID pour identifier chaque fichier et écrit titre/artiste/album/MBID
#    canoniques. C'est le bon usage d'AcoustID + MusicBrainz pour Reis.

# 1. manifeste + jaquettes
python3 index-builder/scan_library.py ~/Music build/

# 2. paroles synchronisées depuis un dump LRCLIB local (facultatif)
python3 index-builder/lrclib_extract.py lrclib-db-dump.sqlite3 build/meta.tsv

# 3. catalogue
target/release/reis build --meta build/meta.tsv --out catalog.reis
target/release/reis info catalog.reis        # tailles par section
```

Préréglages de taille (mesurés, `docs/BENCHMARKS.md`) :

| préréglage | options | index / min d'audio | top-1 (seuil strict) |
|---|---|---|---|
| défaut | `--pps 30 --fanout 3` | 23,1 Ko | 87 % |
| compact | `--pps 30 --fanout 2` | ≈ 16,5 Ko | voir benchmarks |
| robuste | `--pps 30 --fanout 6` | ≈ 45,6 Ko | voir benchmarks |

Mémoire au build : ~8 octets par posting (≈ 90 postings/s d'audio) ; une
bibliothèque de 10 000 titres de 3,5 min demande ~1,5 Go de RAM au moment du
tri final. Le fingerprinting tourne ~600× plus vite que le temps réel sur 2 cœurs.

Voir `docs/DATA_LICENSES.md` avant de partager un catalogue.
