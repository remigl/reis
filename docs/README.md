# Documentation de Reis

Par où commencer :

1. **Utiliser Reis** → [GUIDE.md](GUIDE.md)
2. **Comprendre** comment un son est reconnu → [CONCEPTS.md](CONCEPTS.md)
3. **Intégrer** Reis dans un programme ou une app → [API.md](API.md)
4. **Modifier / étendre** le moteur → [ARCHITECTURE.md](ARCHITECTURE.md), [FORMATS.md](FORMATS.md), [MODELS.md](MODELS.md), [../CONTRIBUTING.md](../CONTRIBUTING.md)
5. **Vérifier** les performances annoncées → [BENCHMARKS.md](BENCHMARKS.md)
6. **Droits** sur les sons, paroles, images, modèles → [DATA_LICENSES.md](DATA_LICENSES.md)

Glossaire :

| terme | sens |
|---|---|
| référence | un fichier son de ta bibliothèque |
| label / catégorie | le dossier d'une référence (`animaux/chien`) |
| bibliothèque | le fichier `.reis` produit par `reis index` |
| empreinte exacte | constellation de pics, identifie un enregistrement précis |
| embedding | vecteur produit par le réseau, décrit « ce que c'est » |
| fenêtre | morceau de 3 s analysé par le réseau |
| seuil | score sémantique minimal pour accepter une catégorie |
| faux positif | un son absent de la bibliothèque reconnu à tort |
| ensemble fermé / ouvert | sans / avec la possibilité de répondre « inconnu » |
