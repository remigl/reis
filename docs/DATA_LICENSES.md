# Données : sources, licences, ce qui peut être redistribué

Règle du projet : **le dépôt ne contient aucune donnée tierce** (ni sons, ni
paroles, ni images, ni poids de modèles ; les modèles convertis sont publiés à
part, dans la release `models-v1`, sous leur licence MIT d'origine). Il contient du
code et des outils qui construisent un catalogue `.reis` localement. Qui
publie un catalogue est responsable de ce qu'il y met.

| Source | Ce qu'on en tire | Licence | Redistribuable dans un catalogue public ? |
|---|---|---|---|
| **Tes propres enregistrements** (animaux, bruits, voix…) | toutes les empreintes | les tiens | oui |
| EfficientAT (modèles `mn04_as`, `mn10_as`) | réseau du moteur sémantique, embarqué dans chaque bibliothèque | code et poids sous licence MIT ; entraînés sur AudioSet (annotations CC BY 4.0, Google) | oui, avec la mention de licence MIT |
| ESC-50 (benchmark uniquement) | 2 000 sons de 5 s | CC BY-NC 3.0 | non commercial, **jamais redistribué ici** (téléchargé par l'utilisateur) |
| xeno-canto, Freesound (exemples de sources de sons) | enregistrements de référence | licence choisie par chaque auteur (souvent CC BY, CC BY-NC, CC0) | selon chaque fichier : conserver l'attribution dans les métadonnées `.json` |
| Votre audio (bibliothèque) | empreintes landmark | droits de l'œuvre | les empreintes sont des données dérivées non réversibles ; statut juridique à apprécier selon le pays — usage personnel sans difficulté |
| MusicBrainz | titres, artistes, albums, MBID | cœur des données CC0 (domaine public), le reste CC BY-NC-SA | oui pour les données CC0 |
| AcoustID | empreintes Chromaprint + liens vers MBID | CC BY-SA 3.0 ; correspondance MBID–AcoustID assimilée au domaine public | oui avec attribution + partage à l'identique — mais voir ci-dessous |
| LRCLIB | paroles synchronisées | code MIT ; **le texte des paroles appartient aux auteurs/éditeurs**, aucune licence de redistribution | **non** sans accord des ayants droit ; usage personnel uniquement |
| Cover Art Archive | jaquettes | chaque image reste sous le copyright de son titulaire | **non** par défaut |
| Jaquettes intégrées aux fichiers | jaquettes | idem | usage personnel |

## Pourquoi AcoustID ne sert pas de base de reconnaissance

AcoustID stocke des empreintes **Chromaprint**, conçues pour identifier un
fichier entier propre. Le benchmark (`docs/BENCHMARKS.md`) montre, avec la vraie
libchromaprint et une recherche exhaustive (borne supérieure de ce qu'un index
AcoustID permettrait) :

* 2 s d'audio : aucune empreinte n'est produite (0 %) ;
* son passé par un haut-parleur + pièce : ≤ 5 % de reconnaissance même à 10 s.

Or le cas d'usage de Reis est précisément « quelques secondes captées par un
micro ». Les empreintes landmark doivent être calculées à partir de l'audio de
référence ; AcoustID ne fournit pas d'audio. AcoustID reste utile pour :
relier un fichier de votre bibliothèque à son MBID (tags propres), et un mode
« écoute longue » de secours n'est pas retenu vu les chiffres.

## Paroles

`tools/lrclib_extract.py` lit un dump SQLite LRCLIB **local** et
apparie artiste/titre/durée (± 2 s) et écrit un `.lrc` à côté de chaque
fichier. La bibliothèque produite est destinée à votre appareil. Pour un catalogue public, n'incluez que des paroles dont vous avez les
droits (paroles libres, domaine public, accord d'éditeur).

## Paroles téléchargées dans l'app

L'app peut chercher des paroles sur l'API publique de LRCLIB (option
désactivée par défaut). Elles sont stockées uniquement sur ton téléphone,
dans la bibliothèque ou dans `lyrics.json`, pour ton usage personnel. Reis ne
les republie nulle part. Seuls le titre, l'artiste, l'album et la durée sont
envoyés.

## Conseil pratique

Mets la licence et l'auteur de chaque son de référence dans son `.json`
(`{"auteur": "…", "licence": "CC BY 4.0", "source": "https://…"}`) : ces
métadonnées sont conservées dans la bibliothèque et renvoyées à la
reconnaissance, ce qui permet d'afficher l'attribution exigée par les licences
Creative Commons.
