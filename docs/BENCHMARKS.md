# Benchmarks

Tout est reproductible : `scripts/reproduce_benchmarks.sh` (≈ 15 min sur 2 cœurs).
Résultats bruts : `benchmarks/results/`.

## Conditions de mesure — à lire avant les chiffres

* **Machine** : conteneur Linux x86-64, 2 cœurs, mesures mono-cœur pour les
  requêtes. Aucun téléphone n'était disponible : le CPU, la RAM et la batterie
  sur mobile **ne sont pas mesurés** ; un ARM milieu de gamme est typiquement
  2 à 4× plus lent par cœur.
* **Corpus synthétique** : aucun enregistrement réel n'était accessible
  (pas de réseau, dossier Musique non partagé). `benchmarks/gen_corpus.py`
  génère 300 morceaux catalogués + 60 morceaux *hors catalogue* (1 min chacun)
  avec accords, basse, mélodie, batterie, tempo/tonalité/timbres aléatoires et
  des briques partagées (mêmes gammes, mêmes kits) : un piège à faux positifs
  plutôt pessimiste. La musique réelle (voix, mixages denses) est différente :
  **refaire le benchmark sur une vraie bibliothèque est la prochaine étape**
  (`reis build` + `make_queries.py` fonctionnent tels quels sur de vrais fichiers).
* **Requêtes** (`make_queries.py`) : 3 200 extraits à position aléatoire,
  16 kHz mono comme un micro ; 10 conditions × 4 durées (2/3/5/10 s) × 60
  positifs + 20 négatifs.

| condition | simulation |
|---|---|
| clean | extrait direct |
| speaker | haut-parleur de téléphone (passe-bande 300–6 500 Hz, résonance 1,8 kHz, saturation douce) → pièce (RT60 0,3–0,6 s, rapport direct/réverbéré 0–6 dB) → micro (passe-haut, bruit propre −60 dBFS) |
| noise10 / noise0 | bruit rose + **une autre musique** (hors catalogue) à SNR 10 / 0 dB |
| speaker_noise5 | haut-parleur + pièce + bruit/autre musique à 5 dB — le cas « bar » |
| lowvol | −38 dB, bruit de micro −70 dBFS, quantification 16 bits |
| highvol | +18 dB, écrêtage dur |
| mp3_64k / aac_48k / opus_16k | aller-retour codec (délai codec non compensé) |

* **top-1** = accepté par la règle stricte **et** bon titre ; **top-5** = bon
  titre parmi les 5 premiers candidats (sans seuil) ; **wrong** = accepté mais
  mauvais titre ; **FP** = négatif accepté.

## 1. Choix du moteur : landmarks vs Chromaprint

Chromaprint = l'empreinte stockée par AcoustID, calculée par la vraie
libchromaprint (muxer ffmpeg, algorithme 1), recherche **exhaustive** de
l'alignement au taux d'erreur binaire minimal sur toutes les références : c'est
une borne supérieure de ce que permettrait un index AcoustID. Règle calibrée
pour ≤ 0,5 % d'erreurs.

| top-1 | 2 s | 3 s | 5 s | 10 s | speaker 10 s | speaker+bruit 10 s |
|---|---|---|---|---|---|---|
| **Chromaprint** (FP ≤ 0,5 %) | **0 %** (aucune empreinte produite) | 61 % | 71 % | 79 % | 5 % | 17 % |
| **Landmarks Reis** (règle stricte, FP = 0) | 55 % | 76 % | 90 % | 99 % | 100 % | 100 % |

Taille : Chromaprint 1,85 Ko/min (brut) contre 23 Ko/min pour les landmarks ;
précision de position Chromaprint ≈ 30–40 ms (pas de 124 ms) contre 5 ms.

**Décision : landmarks.** Chromaprint est 12× plus compact mais ne survit pas
au trajet haut-parleur → pièce → micro, et ne produit rien sous ~2,5 s. Une
approche neuronale (type « Neural Audio Fingerprint ») n'a pas été
benchmarkée : pas de framework d'apprentissage ni de GPU disponibles ici, et
elle imposerait un modèle de plusieurs Mo + un index de vecteurs (≥ 128 octets
par segment de 1 s, soit ≈ 7 Ko/min en int8 — compétitif en taille, mais coût
CPU d'inférence permanent). C'est la piste de comparaison suivante, à mesurer
sur vraie musique.

## 2. Moteur par défaut — détail (300 titres)

`benchmarks/results/landmark_default.md` ; règle : `score ≥ 13` et
`score ≥ 2,2 × (second + 1)`.

| condition | 2 s | 3 s | 5 s | 10 s |
|---|---|---|---|---|
| clean | 100 % | 100 % | 100 % | 100 % |
| mp3 64k | 98 % | 100 % | 100 % | 100 % |
| aac 48k | 88 % | 100 % | 100 % | 100 % |
| opus 16k | 60 % | 98 % | 100 % | 100 % |
| lowvol | 73 % | 93 % | 100 % | 100 % |
| noise10 | 58 % | 87 % | 100 % | 100 % |
| speaker | 47 % | 92 % | 98 % | 100 % |
| speaker+noise5 | 10 % | 43 % | 82 % | 100 % |
| noise0 | 8 % | 15 % | 55 % | 95 % |
| highvol (écrêtage +18 dB) | 8 % | 27 % | 63 % | 98 % |
| **toutes conditions** | **55 %** | **76 %** | **90 %** | **99 %** |

* top-5 : 98,2 % ; **0 mauvais titre accepté, 0 faux positif / 800 négatifs**.
* Position dans le morceau : erreur **médiane 5 ms, p95 14 ms**.
* Temps de reconnaissance (1 cœur x86) : 7 ms pour 2 s, 19 ms pour 5 s, 39 ms pour 10 s.
* Mémoire : pic 17 Mo (dont 9,6 Mo de pages du catalogue mappé).

## 3. Moteur en conditions réelles d'usage (`reis simulate`)

La vraie machine à états, micro simulé par callbacks de 20 ms, 20 titres par
scénario, départ à une position aléatoire (`results/engine_simulation.md`) :

| scénario | verrouillés | temps jusqu'au résultat p50 / p90 | erreur de position p50 / p95 | CPU |
|---|---|---|---|---|
| clean | 20/20 | 2,0 / 2,0 s | 7 / 7 ms | 5,8 ms par s |
| speaker | 20/20 | 2,5 / 3,0 s | 10 / 10 ms | 5,8 ms/s |
| speaker + bruit 5 dB | 20/20 | 3,5 / 5,1 s | 7 / 7 ms | 7,2 ms/s |
| pause de 4 s à t = 20 s | 20/20 (pause détectée 20/20) | 2,5 / 3,5 s | 8 / 17 ms (max 974 ms pendant la détection) | 4,7 ms/s |
| dérive d'horloge +0,2 % | 20/20 | 3,5 / 4,6 s | 46 / 76 ms (corrigée toutes les 15 s) | 5,7 ms/s |

* **Faux verrouillages : 0** sur 60 morceaux hors catalogue écoutés 55 s en
  continu (≈ 100 tentatives chacun). La règle a été durcie pour ça : avec
  `ratio ≥ 1,5`, 9/60 ; avec une « confirmation par deux fenêtres » 7/60
  (des fenêtres qui se chevauchent ne sont pas des preuves indépendantes —
  mécanisme désactivé).
* CPU ≈ 0,6 % d'un cœur x86 en moyenne ; une fois verrouillé, le coût est
  dominé par une vérification de ~40 ms toutes les 15 s.
* Mode batterie (`duty_cycle_mic`) : micro ouvert 27 % du temps, mais une
  pause n'est alors détectée qu'à la vérification suivante (erreur jusqu'à la
  durée de la pause pendant ≤ 15 s).

## 4. Passage à l'échelle (30 000 titres)

`reis scale --copies 100` réplique les 300 titres avec transposition de
fréquence (1–40 bins) et décalage temporel : 30 000 titres distincts aux
statistiques de hash réalistes, 30 000 min d'audio (≈ 8 600 titres de 3,5 min),
catalogue **mappé en mémoire** (`results/landmark_30k_songs.md`).

| | 300 titres | 30 000 titres |
|---|---|---|
| index | 6,9 Mo | 734,5 Mo |
| octets par posting | 4,33 | 4,59 |
| top-1 / FP | 80,0 % / 0 | **79,8 % / 0** |
| temps p50 (5 s) | 19 ms | 55 ms |
| temps p50 (10 s) | 39 ms | 118 ms |
| tas (RssAnon) | 7,7 Mo | ≈ 90 Mo au pire (requête 10 s) |

La précision ne se dégrade pas avec 100× plus de titres. Le moteur espace
automatiquement ses tentatives (≤ 1/3 du temps à chercher) sur appareil lent.
Le passage HashMap → tri des votes a divisé le temps par 3 à cette échelle.

## 5. Taille : compromis densité / robustesse

Même jeu de requêtes, même règle stricte (FP = 0 partout) :

| pics/s × fan-out | index / min d'audio | top-1 | 2 s | 5 s | speaker+bruit | temps p50 |
|---|---|---|---|---|---|---|
| 45 × 3 | 34,6 Ko | 82,2 % | 62 % | 90 % | 62 % | 24 ms |
| 30 × 6 | 45,6 Ko | 81,3 % | 54 % | 94 % | 62 % | 41 ms |
| **30 × 3 (défaut)** | **23,1 Ko** | **80,0 %** | 55 % | 90 % | 59 % | 19 ms |
| 40 × 2 | 21,2 Ko | 78,6 % | 56 % | 87 % | 55 % | 16 ms |
| 30 × 2 (compact) | 16,8 Ko | 76,0 % | 49 % | 87 % | 50 % | 15 ms |
| 20 × 5 | 25,9 Ko | 74,6 % | 39 % | 91 % | 54 % | 27 ms |
| 15 × 4 | 16,5 Ko | 68,4 % | 27 % | 85 % | 46 % | 19 ms |
| 10 × 4 | 10,8 Ko | 58,5 % | 11 % | 77 % | 36 % | 14 ms |

## 6. Budget 2 Go

| poste | mesuré / estimé | par titre (3,5 min) |
|---|---|---|
| index d'empreintes (défaut) | mesuré : 23,1–24,5 Ko/min | ≈ 86 Ko |
| paroles | synthétiques mesurées 0,6 Ko ; **réelles estimées** 1,5–2,5 Ko (non mesuré, pas de dump LRCLIB ici) | ≈ 2 Ko |
| jaquette 160 px WebP q60 | mesuré sur une photo : 3,9 Ko, partagée par album | 0,4–3,9 Ko |
| métadonnées | mesuré : 32 o + chaînes dédupliquées | ≈ 0,1 Ko |
| moteur natif | mesuré : `libreis_core.so` 0,41 Mo | — |
| application Flutter | **estimée** 15–25 Mo (non compilée ici) | — |

**Capacité sous 2 Go : ≈ 22 000 titres (défaut), ≈ 32 000 (compact),
≈ 50 000 (10 × 4, nettement moins robuste).** Un catalogue de type Shazam
(dizaines de millions de titres) ne tient dans 2 Go avec *aucune* empreinte
robuste au micro mesurée ici ; le bon périmètre est « votre bibliothèque » ou
un catalogue thématique (radio, festival, top N d'un pays…).

## Itérations (ce qui a été essayé, mesuré, gardé ou jeté)

Mesures de répétabilité (`reis diag`) : part des pics de référence retrouvés
dans la requête alignée, 5 s, 12 requêtes par condition.

| étape | noise10 | lowvol | speaker | clean | décision |
|---|---|---|---|---|---|
| v1 : pics sur spectre blanchi, classement par proéminence | 21 % | 20 % | 21 %* | 77 % | — |
| pics sur log-puissance brute + plancher relatif −60 dB | 59 % | 64 % | 22 %* | 90 % | gardé |
| bande basse 125 → 250 Hz | ≈ | ≈ | +2 pts | ≈ | gardé (réalisme téléphone) |
| quantification des hash (f, Δf, Δt ÷ 2) | hash alignés ×1,1–1,6 | | | | gardé |
| requête 2× plus dense, fan-out ×4 | hash alignés ×1,2–1,4 | | | | gardé |
| un vote par trame de requête et par alignement | top-5 +1,4 pt | | | | gardé |
| confirmation par 2 fenêtres successives | | | | | jeté (FP 7/60) |

\* avant correction du simulateur de pièce : la première version avait un
rapport direct/réverbéré de −17 dB (cathédrale), corrigé à 0–6 dB ; tous les
chiffres finaux utilisent le simulateur corrigé.
