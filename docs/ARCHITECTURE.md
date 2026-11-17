# Architecture

```text
                Flutter UI  (app/flutter — remplaçable sans toucher au moteur)
                    │  dart:ffi  (core/src/ffi.rs, ABI C stable)
                    ▼
               reis-core (Rust, 0 dépendance)
        ┌───────────┴────────────┐
        ▼                        ▼
  engine.rs  Listening ⇄ Locked ⇄ Paused        lyrics.rs  LRC → binaire compact
   │   push_pcm(clock) / tick(now)                  │       line_at(pos) en O(log n)
   ▼                                                ▼
 audio.rs (rééchantillonnage 8 kHz)          catalog.rs (fichier .reis mmap)
   ▼                                           ├─ HIDX  index d'empreintes
 fingerprint.rs (constellation)                ├─ SONG  métadonnées 32 o/titre
   ▼                                           ├─ STRS  chaînes dédupliquées
 index.rs (hash → (titre, temps), vote d'offset) ├─ LYRC paroles
                                               └─ ARTW  jaquettes WebP dédupliquées
```

Le moteur ne connaît ni l'interface ni l'horloge système : l'hôte pousse du PCM
avec un horodatage monotone et appelle `tick(now)`. C'est ce qui rend le moteur
testable (`reis simulate`, `core/tests/end_to_end.rs`) et portable (Android,
iOS, desktop, CLI).

## Empreinte (fingerprint.rs)

1. Mono 8 kHz, STFT Hann 1024 points, pas 128 (16 ms).
2. Log-puissance avec **plancher relatif** (max de la trame − 60 dB) : invariant
   au gain et insensible au bruit de fond d'enregistrement.
3. Pics = maxima locaux de la log-puissance brute sur un voisinage
   ±12 bins × ±8 trames, avec proéminence ≥ 3 dB au-dessus de la moyenne locale
   (spectre blanchi), bande 250 Hz – 3,75 kHz (ce qu'un haut-parleur de
   téléphone restitue).
4. Densité bornée : les 30 pics les plus énergétiques par seconde (référence).
5. Chaque pic ancre est apparié aux 3 pics suivants (zone 2–100 trames) ;
   hash 24 bits = f1 | Δf | Δt, quantifiés (×2) pour tolérer ±1 bin / ±1 trame.
6. **Asymétrie requête / référence** : la requête utilise 2× plus de pics et
   un fan-out ×4, de sorte que les paires de la référence (plus clairsemées)
   restent présentes malgré le bruit qui réordonne les pics. L'index reste
   petit, seule la requête coûte un peu plus de CPU.

Chacun de ces choix a été mesuré (voir `docs/BENCHMARKS.md`, section
« Itérations ») : par exemple, détecter les pics sur le spectre brut avec
plancher relatif au lieu du spectre blanchi a fait passer la proportion de pics
de référence retrouvés sous bruit à 10 dB de 21 % à 59 %, et en volume faible
de 20 % à 64 %.

## Index (index.rs)

Entrées `(hash 24 bits, titre 24 bits, trame 16 bits)` triées. Répertoire de
2^B seaux (B choisi pour ~16 entrées/seau) puis, par seau, entrées codées en
varints delta (hash bas, titre, temps). Lecture directe dans le fichier mappé
en mémoire : aucune désérialisation au démarrage, RAM ≈ pages réellement
touchées. ≈ 4,3 octets par posting mesurés.

Matching : chaque hash de la requête vote pour `(titre, offset = t_ref − t_req)`,
au plus un vote par trame de requête et par alignement (les sons tenus ne
gonflent pas les scores), fusion ±1 trame. Le meilleur offset donne la
**position dans le morceau** à 16 ms près (erreur médiane mesurée 5 ms).

Règle d'acceptation (calibrée, `benchmarks/calibrate.py` + `sim_engine.py`) :
`score ≥ 13` et `score ≥ 2,2 × (second + 1)`.

## Moteur temps réel (engine.rs)

| état | micro | traitement |
|---|---|---|
| Listening | ouvert | tentative toutes les 0,5 s sur la fenêtre 2 → 10 s |
| Locked | ouvert, **niveau seulement** (ou fermé en mode batterie) | position = ancre + temps écoulé ; vérification 4–10 s toutes les 15 s |
| Paused | ouvert | silence > 1,5 s → position figée ; reprise du son → reprise + vérification immédiate |

Vérification : si le même titre est retrouvé, l'écart à la position prédite
(> 60 ms) est corrigé ; un autre titre fortement reconnu remplace le courant ;
4 échecs consécutifs → retour en écoute continue **sans effacer l'affichage**
(les paroles continuent de défiler, icône « incertain »).

Les discontinuités du flux audio (micro coupé, perte de paquets > 250 ms)
réinitialisent la fenêtre : une fenêtre d'analyse est toujours contiguë, donc
son horodatage est exact.

## Format `.reis`

En-tête `REIS | version | n | (tag, offset, len)*`, sections alignées sur 8 octets :
`PARM` (paramètres d'empreinte — un index n'est jamais interrogé avec d'autres
paramètres), `HIDX`, `SONG`, `STRS`, `LYRC`, `ARTW`. Détails dans
`core/src/catalog.rs`.

Paroles (`lyrics.rs`) : lignes uniques stockées une fois (les refrains
répétés ne coûtent qu'un index), temps en centisecondes delta-codés, mots
optionnels (LRC « enhanced » `<mm:ss.xx>`).

## Interface (app/flutter)

* `engine_service.dart` : moteur dans un **isolate dédié** (le calcul ne bloque
  jamais l'UI), micro via `record` (PCM 16 bits 16 kHz, AGC / suppression de
  bruit désactivés — ils détruisent exactement le signal utile), ouverture /
  fermeture du micro pilotée par `wantsAudio`.
* `lyrics_view.dart` : ligne active trouvée par recherche dichotomique à chaque
  frame (Ticker), position extrapolée entre deux ticks → défilement fluide.
* `settings.dart` : police, taille, gras, espacement, hauteur de ligne,
  alignement, couleur, contraste — persistés (`shared_preferences`, clés
  `lyrics.*`).
