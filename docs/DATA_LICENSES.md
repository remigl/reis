# Données : sources, licences, ce qui peut être redistribué

Règle du projet : **le dépôt ne contient aucune donnée tierce**. Il contient du
code et des outils qui construisent un catalogue `.reis` localement. Qui
publie un catalogue est responsable de ce qu'il y met.

| Source | Ce qu'on en tire | Licence | Redistribuable dans un catalogue public ? |
|---|---|---|---|
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

`index-builder/lrclib_extract.py` lit un dump SQLite LRCLIB **local** et
apparie artiste/titre/durée (± 2 s). Le catalogue produit est destiné à votre
appareil. Pour un catalogue public, n'incluez que des paroles dont vous avez les
droits (paroles libres, domaine public, accord d'éditeur).
