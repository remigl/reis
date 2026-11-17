Premier prototype de **Reis** : reconnaissance musicale 100 % hors ligne avec paroles synchronisées.

### Fichiers
| fichier | contenu |
|---|---|
| `reis-v0.1.0-android.apk` | app Android (arm64, armv7, x86_64), Android 7.0+ |
| `reis-v0.1.0-windows-x64.zip` | app Windows (dézipper, lancer `reis.exe`) |
| `reis-cli-*` | outil en ligne de commande : construire un catalogue, identifier, écouter au micro |
| `libreis_core-*.so` | moteur natif Linux (pour intégrer Reis ailleurs) |

### Avant de lancer l'app : il faut un catalogue
L'app ne contient aucune musique. Construis ton catalogue `.reis` à partir de ta bibliothèque (une fois, sur ordinateur) :

```bash
python3 index-builder/scan_library.py ~/Music build/
reis-cli build --meta build/meta.tsv --out catalog.reis
```
Puis, au premier lancement de l'app : **Importer un catalogue**.

### Limites connues
- APK signé avec la clé de debug (installation manuelle, « sources inconnues »).
- Précision mesurée sur un corpus synthétique ; pas encore validée sur de la vraie musique ni mesurée sur téléphone. Voir `docs/BENCHMARKS.md`.
