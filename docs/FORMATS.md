# Formats de fichiers

Tous les entiers sont little-endian. Les deux formats sont conçus pour être lus
**sans désérialisation** (mmap) et évoluer sans casser : un lecteur ignore les
sections et clés JSON qu'il ne connaît pas.

## Bibliothèque `.reis` (version 2)

```text
b"REIS" | u32 version = 2 | u32 n
n × { tag[4] | u64 offset | u64 longueur }       table des sections
… sections, chacune alignée sur 16 octets …
```

| section | contenu | présente |
|---|---|---|
| `META` | JSON : `format`, `version`, `engines`, `stats` | toujours |
| `REFS` | JSON : tableau d'objets `{name, label, file, duration, meta?, lyrics?, art?}` (l'indice = identifiant) | toujours |
| `EXCT` | `FpParams` (17 × u32) puis l'index landmark | moteur exact |
| `SEMA` | vecteurs sémantiques (voir ci-dessous) | moteur sémantique |
| `MODL` | le fichier `.rnm` qui a servi à construire `SEMA` | moteur sémantique |
| `LYRC` | paroles encodées, référencées par `REFS[i].lyrics` (décalage) | toujours (peut être vide) |
| `ARTW` | `u32 n`, `u32 offsets[n+1]`, images (WebP/JPEG), référencées par `REFS[i].art` | toujours (peut être vide) |

### `EXCT` — index landmark

```text
FpParams : sample_rate, n_fft, hop, bin_lo, bin_hi, nb_f, nb_t, peaks_per_sec (f32),
           fanout, dt_min, dt_max, df_max, min_prominence_db (f32), whiten_w, rank_raw, q_f, q_t
u32 B                               bits de seau (le seau = 24 − B bits de poids fort du hash)
u32 offsets[2^B + 1]                début de chaque seau dans DATA
DATA : par seau, entrées triées (hash bas, référence, trame) :
   varint(n) puis pour chaque entrée
     varint(Δhash_bas) ; si Δ = 0 : varint(Δréf) puis varint(Δtrame si même réf, sinon trame)
                          sinon : varint(réf), varint(trame)
```

B est choisi pour ≈ 16 entrées par seau. Mesuré : ≈ 4,3–4,6 octets par posting.

### `SEMA` — vecteurs sémantiques

```text
u32 longueur_en_tête | en-tête JSON | bourrage → 8
i8   vecteurs[n × dim]          normalisés, centrés, quantifiés
     bourrage → 4
f32  échelles[n]                valeur = i8 × échelle
u32  références[n]
f32  instants[n]                début de la fenêtre dans la référence (s)
```

En-tête JSON : `model`, `dim`, `vectors`, `quant`, `window_s`, `hop_s`,
`max_windows`, `output`, `top_m`, `silence_rel_db`, `augment`, `center`
(vecteur moyen soustrait), `calibration` :

```json
{"threshold": 0.605, "max_false_accept": 0.2, "loo_accuracy": 0.932,
 "loo_accepted_correct": 0.84, "genuine_median": 0.81, "impostor_median": 0.47,
 "samples": 1280,
 "curve": [{"false_accept": 0.01, "threshold": 0.79, "accepted_correct": 0.52}, …]}
```

### `LYRC` — paroles

```text
varint n_lignes_uniques, puis n × (varint longueur, UTF-8)
varint n_événements, puis n × (varint Δtemps en centisecondes, varint indice de ligne,
                               varint n_mots, n_mots × (varint Δcs depuis la ligne, varint fin en octets))
```

Les lignes répétées (refrains) ne sont stockées qu'une fois.

## Modèle neuronal `.rnm` (version 1)

```text
b"RNNM" | u32 version = 1 | u32 longueur_en_tête | en-tête JSON | bourrage → 16 | tenseurs
```

En-tête :

```json
{
  "format": "reis-nn", "version": 1, "name": "mn10_as",
  "source": "...", "trained_on": "AudioSet (527 classes)",
  "frontend": {"type": "logmel", "sample_rate": 32000, "n_fft": 1024, "hop": 320,
               "preemphasis": 0.97, "center": "reflect", "window": "frontend.window",
               "mel": "frontend.mel", "log_eps": 1e-5, "norm_add": 4.5, "norm_div": 5.0},
  "ops": [ {"op": "conv", "w": "f0.w", "b": "f0.b", "cin": 1, "cout": 32, "k": [3,3],
            "stride": [2,2], "pad": [1,1], "groups": 1, "act": "hswish"},
           {"op": "push"}, …, {"op": "se", "w1": …, "b1": …, "w2": …, "b2": …, "act": "relu", "gate": "sigmoid"},
           {"op": "add"}, …, {"op": "gap"}, {"op": "output", "name": "embedding"},
           {"op": "linear", "w": "c1.w", "b": "c1.b", "act": "hswish"},
           {"op": "linear", …}, {"op": "output", "name": "logits", "post": "sigmoid"} ],
  "labels": ["Speech", "Child speech, kid speaking", …],
  "tensors": {"f0.w": {"offset": 0, "shape": [32,1,3,3], "dtype": "f16"}, …}
}
```

- Front-end : pré-accentuation `y[i] = x[i+1] − 0,97·x[i]`, STFT centrée
  (réflexion), fenêtre et banc mel fournis comme tenseurs (donc n'importe quel
  banc : Kaldi, Slaney, HTK…), `(ln(mel + eps) + norm_add) / norm_div`.
- Tenseur d'activation : `[canaux][fréquence][temps]`. Poids de convolution :
  `[cout][cin/groupes][kh][kw]` (convention PyTorch). BatchNorm doit être
  replié dans les convolutions par l'exportateur.
- Opérations : voir l'en-tête de `core/src/nn.rs`. `push`/`add` réalisent les
  connexions résiduelles (pile).
- `dtype` : `f16` (poids) ou `f32` (biais, front-end).

Fichier de référence `<nom>.ref.bin` (vérification) : quatre tableaux
`u32 n | f32[n]` — signal de test, log-mel, embedding, logits — puis `u32 trames`.
