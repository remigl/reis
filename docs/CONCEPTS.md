# Comment Reis reconnaît un son

Reis répond à deux questions différentes, avec deux « empreintes » différentes,
plus un filet de sécurité :

| question | moteur | empreinte | exemple |
|---|---|---|---|
| *Est-ce **cet** enregistrement ?* | **exact** | constellation de pics temps-fréquence (type Shazam) | « One More Time », à 1 min 23 s |
| *Est-ce **ce genre** de son ?* | **sémantique** | vecteur produit par un réseau de neurones audio | « un chien », même un autre chien |
| *Qu'est-ce que c'est, en général ?* | **tags** | sortie de classes du même réseau | « Dog 46 % » (527 classes AudioSet) |

```text
                     ┌─ exact ───────── même enregistrement ?  ──┐
 audio ─ 0,5 à 10 s ─┼─ sémantique ──── même catégorie ?        ─┼─▶ verdict
                     └─ tags ────────── quoi, en général ?      ─┘
        verdict = exact (si accepté) > semblable (si ≥ seuil) > tag > inconnu
```

## 1. Moteur exact : la constellation

C'est la méthode de Shazam (A. Wang, *An Industrial-Strength Audio Search
Algorithm*, 2003), réimplémentée et réglée par mesure.

1. Le son est ramené à 8 kHz, découpé en trames de 128 ms toutes les 16 ms
   (transformée de Fourier).
2. On garde les **pics** : les points temps-fréquence plus forts que tous leurs
   voisins (±94 Hz, ±128 ms), au plus 30 par seconde, entre 250 Hz et 3,75 kHz
   (ce que restitue un haut-parleur de téléphone).
3. Chaque pic est relié aux 3 pics suivants. Une paire donne un **hash** de
   24 bits (fréquence 1, écart de fréquence, écart de temps) et l'instant du
   premier pic.
4. Index : hash → liste de (référence, instant). Un fichier compressé
   (varints), lu directement sur le disque (mmap).
5. Requête : chaque hash de l'extrait vote pour (référence, décalage =
   instant dans la référence − instant dans l'extrait). Un vrai match accumule
   des votes **au même décalage**, le hasard les disperse. Le décalage gagnant
   **est la position** dans la référence, à 16 ms près.

Propriétés : insensible au volume, robuste au bruit, au haut-parleur, aux codecs ;
mais il ne reconnaît que **le même enregistrement** (une autre interprétation,
un autre chien : rien à voir pour lui).

## 2. Moteur sémantique : l'embedding neuronal

Un réseau de neurones pré-entraîné sur [AudioSet](https://research.google.com/audioset/)
(2 millions d'extraits YouTube, 527 classes) transforme quelques secondes de son
en un **vecteur de 960 nombres** qui résume « ce que c'est ». Deux aboiements
de chiens différents donnent des vecteurs proches ; un aboiement et une perceuse,
des vecteurs éloignés.

Modèle par défaut : **EfficientAT mn10** (F. Schmid et al., ICASSP 2023,
licence MIT), une MobileNetV3 de 4,9 M de paramètres, parmi les meilleurs
compromis qualité / coût publiés pour l'audio. Reis l'exécute avec son propre
moteur d'inférence en Rust (aucune bibliothèque d'IA à installer), vérifié
contre PyTorch : cosinus 0,999998 entre les deux sorties.

Construction :
1. chaque référence est découpée en fenêtres de 3 s (pas de 1,5 s, 32 max),
   les fenêtres quasi silencieuses sont écartées ;
2. chaque fenêtre → vecteur → **centré** (on soustrait la moyenne de la
   bibliothèque, sinon tous les sons se ressemblent un peu) → normalisé → stocké
   en int8 (≈ 1 octet par nombre).

Requête :
1. l'extrait est découpé et vectorisé de la même façon ;
2. **score d'une référence** = meilleure similarité cosinus entre une fenêtre de
   l'extrait et une fenêtre de la référence ;
3. **score d'une catégorie** = moyenne de ses 3 meilleures références (un vote
   de type *k* plus proches voisins, qui tolère une référence ratée) ;
4. accepté si le score dépasse le **seuil calibré** de la bibliothèque.

### Le seuil, calculé tout seul

Reconnaître parmi des catégories connues est facile ; dire « je ne connais pas »
l'est moins. À la construction, Reis simule des sons inconnus avec la
bibliothèque elle-même : pour une référence, il cherche la meilleure catégorie
**en retirant toute la sienne**. Ce score d'« imposteur » est ce qu'obtiendrait
un son absent de la bibliothèque. Le seuil est fixé pour qu'au plus 20 % des
imposteurs passent (`--false-accept`). La courbe complète est stockée : on peut
changer de compromis à la requête sans reconstruire.

## 3. Tags généraux

La dernière couche du même réseau donne une probabilité pour chacune des 527
classes AudioSet. Aucune bibliothèque n'est nécessaire. Reis s'en sert quand
rien n'est reconnu dans ta bibliothèque (« absent, mais ressemble à : Cat
66 % »). Les entrées hors domaine (bruit stationnaire, silence numérique) font
saturer ces modèles : Reis les détecte (≥ 5 classes à plus de 90 % en même
temps) et n'affiche alors aucun tag.

## 4. Pourquoi deux moteurs et pas un seul ?

Mesuré sur ESC-50 ([BENCHMARKS.md](BENCHMARKS.md)) :

| tâche | exact | sémantique |
|---|---|---|
| retrouver l'enregistrement d'origine d'un extrait propre de 3 s | **89,6 %**, 0 erreur | trouve la bonne catégorie, pas le fichier ni la position |
| reconnaître un **autre** enregistrement de la même catégorie | 0,6 % | **88,8 %** (sans rejet) |
| position dans le morceau | **±16 ms** | non |
| taille | ≈ 23 Ko / min d'audio | ≈ 1 Ko par fenêtre de 3 s (960 octets + 12) |

Chacun excelle là où l'autre échoue ; ensemble, ils couvrent « ce morceau-là »
et « ce genre de son ».

## 5. Temps réel (moteur de flux)

Pour le micro, `engine.rs` enchaîne trois états :

- **écoute** : toutes les 0,5 s, tentative sur les 2 à 10 dernières secondes ;
- **verrouillé** : position = position mesurée + temps écoulé ; nouvelle
  vérification toutes les 15 s pour corriger une dérive, un saut ou un
  changement de morceau ;
- **pause** : 1,5 s de silence fige la position, le retour du son la relance.

Le moteur sémantique, lui, analyse la dernière fenêtre de 3 s chaque seconde.

## 6. Ce que Reis ne fait pas (encore)

- reconnaître une **reprise** ou une **version live** d'un morceau (il faudrait
  une empreinte de « cover detection ») ;
- séparer **plusieurs sons simultanés** (le moteur sémantique donne le plus
  marquant ; les tags en listent plusieurs) ;
- un index approximatif pour des millions de fenêtres sémantiques : la
  recherche est exhaustive, linéaire dans le nombre de fenêtres (mesurée
  jusqu'à 7 300 fenêtres, où elle reste négligeable devant le réseau) ;
  l'interface est prête pour en ajouter un (voir [ARCHITECTURE.md](ARCHITECTURE.md#points-dextension)).
