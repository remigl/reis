# Modèles neuronaux

Le moteur sémantique et les tags reposent sur un réseau de neurones audio
pré-entraîné, exécuté par le runtime maison de Reis (`core/src/nn.rs`). Les
modèles sont des fichiers `.rnm` ([FORMATS.md](FORMATS.md#modèle-neuronal-rnm-version-1)),
publiés dans la release [`models-v1`](https://github.com/remigl/reis/releases/tag/models-v1).

## Modèles disponibles

| fichier | architecture | paramètres | taille | AudioSet mAP¹ | embedding | 3 s d'audio² | ESC-50 propre / haut-parleur³ |
|---|---|---|---|---|---|---|---|
| `mn10_as.rnm` (défaut) | EfficientAT MobileNetV3 ×1,0 | 4,86 M | 10 Mo | 0,471 | 960 | 28 ms | 88,8 % / 75,9 % |
| `mn04_as.rnm` (mobile) | EfficientAT MobileNetV3 ×0,4 | 0,98 M | 2,3 Mo | 0,432 | 384 | 7–10 ms | 85,9 % / 68,8 % |

¹ chiffres publiés par les auteurs. ² réseau seul, x86-64, 2 cœurs (mesuré par
`reis model-check`). ³ bonne catégorie en ensemble fermé (sans rejet), 40
catégories, voir [BENCHMARKS.md](BENCHMARKS.md).

Source : F. Schmid, K. Koutini, G. Widmer, *Efficient Large-Scale Audio Tagging
via Transformer-to-CNN Knowledge Distillation*, ICASSP 2023 —
[github.com/fschmid56/EfficientAT](https://github.com/fschmid56/EfficientAT),
licence MIT. Entrée : log-mel 128 bandes à 32 kHz.

**Lequel choisir ?** `mn10` par défaut. `mn04` si l'indexation ou la
reconnaissance doit tourner sur un appareil modeste : ≈ 4× plus petit et 2 à 4×
plus rapide, ≈ 3 points de moins en son propre, 7 points de moins via
haut-parleur.

## Vérification

Chaque modèle est livré avec `<nom>.ref.bin` : un signal de test et les sorties
de PyTorch.

```text
$ reis model-check models/mn10_as.rnm models/mn10_as.ref.bin
model mn10_as  params 4864631  frames 300
front-end  max |rust - torch| = 6.77e-5   (13.3 ms)
embedding  cosine(rust, torch) = 0.999999  dim 960
logits     max |rust - torch| = 2.366e-2   network 46.4 ms for 3.0 s of audio
top-5 rust : ["Sanding", "Sawing", "Jackhammer", "Hammer", "Filing (rasp)"]
top-5 torch: ["Sanding", "Sawing", "Jackhammer", "Hammer", "Filing (rasp)"]
```

L'écart résiduel vient des poids stockés en float16 (taille ÷ 2). Le temps
affiché ici a été mesuré machine chargée ; à vide : 28 ms (mn10), 7–10 ms (mn04).
Le signal de test (bruit en rafales + balayage) explique les tags « outils ».

## Convertir un modèle

Les modèles sont convertis par `tools/models/export_efficientat.py`, exécuté
dans GitHub Actions (`.github/workflows/models.yml`), qui publie le résultat
dans la release `models-v1`. Pour convertir localement :

```bash
pip install torch torchaudio torchvision numpy
git clone https://github.com/fschmid56/EfficientAT && cd EfficientAT
python ../reis/tools/models/export_efficientat.py mn10_as ../out    # mn04_as, mn20_as, …
```

L'exportateur :
1. charge le réseau et ses poids pré-entraînés ;
2. **replie** chaque BatchNorm dans la convolution qui précède (le runtime n'a
   pas de BatchNorm) ;
3. parcourt le réseau et écrit la liste plate d'opérations (`conv`, `se`,
   `push`/`add` pour les résiduels, `gap`, `linear`, `output`) ;
4. écrit le front-end (fenêtre, banc mel) comme tenseurs — aucun calcul de
   banc de filtres n'est codé en dur dans Reis ;
5. calcule les sorties de référence sur un signal de test.

### Ajouter un autre réseau

Tout réseau convolutif 2-D sur log-mel dont les couches sont dans la liste
ci-dessous se convertit **sans modifier Reis** : écrire un exportateur sur le
modèle de `export_efficientat.py`.

| supporté | |
|---|---|
| convolutions 2-D | noyau, pas, remplissage quelconques ; groupées et *depthwise* |
| normalisation | BatchNorm (repliée par l'exportateur) |
| activations | ReLU, ReLU6, Hardswish, Hardsigmoid, Sigmoid, SiLU |
| blocs | squeeze-excitation par canal, connexions résiduelles |
| tête | pooling moyen global, couches denses |
| sorties | n'importe quel nombre, post-traitement sigmoid / softmax / L2 |

Candidats compatibles : PANNs MobileNetV1/V2, YAMNet (MobileNetV1), EfficientNet-B0
audio, autres largeurs d'EfficientAT. Non compatibles sans nouvelles opérations :
transformers (AST, BEATs, CLAP-HTSAT), réseaux récurrents. Pour ajouter une
opération : une variante dans `enum Op` et son calcul dans
`Model::forward_mel` (`nn.rs`), plus un test.

Un modèle **spécialisé** est aussi possible, par exemple un réseau entraîné
sur les oiseaux. Attention à la licence : BirdNET, par exemple, est sous
CC BY-NC-SA, donc non commercial.

## Le modèle dans la bibliothèque

`reis index` copie le modèle dans la section `MODL` de la bibliothèque, qui est
ainsi autonome : un téléphone n'a besoin que du `.reis`. Les vecteurs d'une
bibliothèque n'ont de sens qu'avec le modèle qui les a produits. Changer de
modèle oblige donc à reconstruire la bibliothèque.
