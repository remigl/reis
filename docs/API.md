# Intégrer Reis dans un programme

Trois niveaux, du plus simple au plus fin :

1. **ABI C + JSON** : 4 fonctions, depuis n'importe quel langage (Python, C#, Kotlin, Swift, Go, Dart…) ;
2. **bibliothèque Rust** `reis-core` ;
3. **moteur de flux** (micro, temps réel) : ABI C ou Rust.

## 1. ABI C

Compiler : `cargo build --release -p reis-core` →
`target/release/libreis_core.so` (Linux, Android), `reis_core.dll` (Windows),
`libreis_core.dylib` (macOS), `libreis_core.a` (iOS, statique).

```c
typedef struct ReisCatalog ReisCatalog;        // bibliothèque .reis ouverte (mmap)
typedef struct ReisRecognizer ReisRecognizer;  // tous les moteurs

ReisCatalog*    reis_catalog_open(const char* path);           // NULL si erreur
void            reis_catalog_free(ReisCatalog*);
ReisRecognizer* reis_recognizer_new(const ReisCatalog*);       // NULL si erreur
void            reis_recognizer_free(ReisRecognizer*);
char*           reis_recognize_f32(const ReisRecognizer*, const float* pcm, uint32_t frames, uint32_t rate);
char*           reis_recognize_i16(const ReisRecognizer*, const int16_t* pcm, uint32_t frames,
                                   uint32_t channels, uint32_t rate);   // entrelacé, mixé en mono
void            reis_string_free(char*);                       // libère le JSON
```

Le PCM est de n'importe quel taux d'échantillonnage (rééchantillonné en
interne) ; 1 à 10 s d'audio donnent les meilleurs résultats.

Accès aux références (chaînes UTF-8 `(pointeur, longueur)` **empruntées** au
fichier mappé : valides tant que la bibliothèque est ouverte, ne pas libérer) :

```c
uint32_t       reis_catalog_song_count(const ReisCatalog*);
uint8_t        reis_catalog_has(const ReisCatalog*, uint32_t which);  // 0 exact, 1 sémantique
const uint8_t* reis_ref_label(const ReisCatalog*, uint32_t id, uint32_t* len);
const uint8_t* reis_ref_meta(const ReisCatalog*, uint32_t id, const char* key, uint32_t* len);
const uint8_t* reis_song_str(const ReisCatalog*, uint32_t id, uint32_t field, uint32_t* len); // 0 titre, 1 artiste, 2 album
uint32_t       reis_song_duration_ms(const ReisCatalog*, uint32_t id);
const uint8_t* reis_song_artwork(const ReisCatalog*, uint32_t id, uint32_t* len);  // NULL si aucune
uint32_t       reis_lyrics_count(const ReisCatalog*, uint32_t id);
const uint8_t* reis_lyrics_line(const ReisCatalog*, uint32_t id, uint32_t i, uint32_t* t_ms, uint32_t* len);
uint32_t       reis_lyrics_word_count(const ReisCatalog*, uint32_t id, uint32_t i);
uint8_t        reis_lyrics_word(const ReisCatalog*, uint32_t id, uint32_t i, uint32_t j, uint32_t* t_ms, uint32_t* byte_end);
int32_t        reis_lyrics_index_at(const ReisCatalog*, uint32_t id, uint32_t pos_ms);   // -1 avant la 1re ligne
```

### Exemple Python (ctypes, aucune installation)

```python
import ctypes, json, wave, array

lib = ctypes.CDLL("./libreis_core.so")
lib.reis_catalog_open.restype = ctypes.c_void_p
lib.reis_recognizer_new.restype = ctypes.c_void_p
lib.reis_recognizer_new.argtypes = [ctypes.c_void_p]
lib.reis_recognize_i16.restype = ctypes.c_void_p
lib.reis_recognize_i16.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32]
lib.reis_string_free.argtypes = [ctypes.c_void_p]

cat = lib.reis_catalog_open(b"sons.reis")
rec = lib.reis_recognizer_new(cat)

w = wave.open("extrait.wav")                       # PCM 16 bits
pcm = array.array("h", w.readframes(w.getnframes()))
buf = (ctypes.c_int16 * len(pcm)).from_buffer(pcm)
p = lib.reis_recognize_i16(rec, buf, len(pcm) // w.getnchannels(), w.getnchannels(), w.getframerate())
result = json.loads(ctypes.string_at(p).decode())
lib.reis_string_free(p)
print(result["verdict"])          # {'kind': 'similar', 'label': 'dog', 'score': 0.71}
```

Version complète et testée : `tools/example_python.py`. Exemples Rust compilés : `core/examples/`.

## Format JSON

Le même objet est produit par `reis recognize --json`, `reis_recognize_*` et
`Recognizer::to_json`. Les champs absents ne s'appliquent pas.

```json
{
  "verdict": {"kind": "similar", "label": "dog", "score": 0.71},
  "exact": {"reference": {"id": 430, "name": "2-66637-B-20", "label": "crying_baby", "file": "2-66637-B-20.wav"},
            "offset_s": -1.568, "score": 3, "second": 2, "accepted": false},
  "similar": [{"label": "dog", "score": 0.71, "references": 32},
              {"label": "pig", "score": 0.254, "references": 32}],
  "semantic_threshold": 0.605,
  "neighbours": [{"id": 462, "name": "2-122104-A-0", "label": "dog", "file": "2-122104-A-0.wav", "score": 0.725, "at_s": 2}],
  "tags": [{"label": "Animal", "probability": 0.58}, {"label": "Dog", "probability": 0.46}],
  "level_dbfs": -21.4,
  "timings_ms": {"exact": 16, "semantic": 115, "tags": 56}
}
```

| `verdict.kind` | autres champs | sens |
|---|---|---|
| `exact` | `reference`, `offset_s` | même enregistrement ; position du début de l'extrait dans la référence |
| `similar` | `label`, `score` | même catégorie (score cosinus ≥ `semantic_threshold`) |
| `tag` | `label`, `probability` | absent de la bibliothèque ; classe AudioSet la plus probable |
| `silence` | — | niveau < −65 dBFS |
| `unknown` | — | rien de fiable |

`tags_suppressed: true` apparaît quand le modèle a saturé (bruit stationnaire).

## 2. Bibliothèque Rust

```toml
[dependencies]
reis-core = { git = "https://github.com/remigl/reis" }
```

```rust
use reis_core::{Library, Recognizer, RecognizerConfig, Verdict};
use std::sync::Arc;

let lib = Arc::new(Library::open("sons.reis")?);
let rec = Recognizer::new(lib.clone(), RecognizerConfig::default())?;
let (pcm, rate) = reis_core::audio::read_wav("extrait.wav").map_err(|e| e.to_string())?;
let r = rec.recognize(&pcm, rate);
match r.verdict.unwrap() {
    Verdict::Exact { ref_id, offset_s } => println!("{} à {offset_s:.2} s", lib.refs[ref_id as usize].name),
    Verdict::Similar { label, score } => println!("{label} ({score:.2})"),
    Verdict::Tag { label, prob } => println!("ressemble à {label} ({prob:.2})"),
    _ => println!("rien"),
}
```

Construire une bibliothèque par programme (par ex. depuis une base de données
plutôt qu'un dossier) :

```rust
use reis_core::library::{LibraryBuilder, LibraryConfig, SourceItem};

let model = std::fs::read("models/mn10_as.rnm").ok();
let mut b = LibraryBuilder::new(LibraryConfig::default(), model)?;
for item in reis_core::library::scan_dir(std::path::Path::new("mes-sons"), false)? {   // ou tes propres SourceItem
    let prepared = b.prepare_file(&item.path)?;   // parallélisable (Sync)
    b.add(&item, prepared);
}
std::fs::write("sons.reis", b.finish())?;
```

`LibraryBuilder::prepare(&samples, rate)` accepte aussi de l'audio déjà en
mémoire.

## 3. Moteur de flux (micro)

Pour suivre un morceau en temps réel (bibliothèque avec moteur exact) :

```c
typedef struct { uint32_t phase; int32_t song; double position_ms; float confidence;
                 uint8_t wants_audio; uint8_t uncertain; float level_dbfs;
                 uint32_t last_score; uint64_t last_match_us; } ReisState;

void* reis_engine_new(const ReisCatalog*);          // NULL sans moteur exact
void  reis_engine_push_i16(void*, const int16_t* pcm, uint32_t frames, uint32_t channels,
                           uint32_t rate, double clock_end_ms);   // horloge monotone de fin du bloc
void  reis_engine_tick(void*, double now_ms, ReisState* out);     // ~20 fois / s
void  reis_engine_reset(void*);
void  reis_engine_free(void*);
```

- `phase` : 0 écoute, 1 verrouillé, 2 pause ; `song` : −1 ou l'identifiant ;
  `position_ms` : position dans le morceau à `now_ms`.
- `wants_audio` : l'hôte peut couper le micro quand il vaut 0 (économie de batterie).
- L'horloge est fournie par l'hôte : le moteur est déterministe et testable
  (`reis simulate`).

## 4. Dart / Flutter

`app/flutter/lib/reis_ffi.dart` enveloppe toute l'ABI : `ReisCatalog`,
`ReisRecognizer.recognize(Int16List, rate) → String JSON`, `ReisEngine`. Le
moteur tourne dans un isolate dédié (`engine_service.dart`) pour ne jamais
bloquer l'interface.
