**Reis v0.2.0** — reconnaissance de sons 100 % hors ligne à partir de ta propre bibliothèque de références : musiques, cris d'animaux, bruits divers.

Tu fournis un dossier de sons de référence, Reis en fait une bibliothèque d'empreintes, puis reconnaît ces sons dans un fichier ou au micro.

| fichier | contenu |
|---|---|
| `reis-cli-v0.2.0-windows-x64.zip` / `-linux-x86_64.tar.gz` | outil `reis` + modèles neuronaux, prêt à l'emploi |
| `reis-v0.2.0-android.apk` | app Android (importe une bibliothèque `.reis`) |
| `reis-v0.2.0-windows-x64.zip` | app Windows |
| `mn10_as.rnm`, `mn04_as.rnm` | modèles EfficientAT (MIT, AudioSet) convertis pour Reis |

Démarrage rapide :
```bash
reis index mes-sons/ -o sons.reis      # un sous-dossier = une catégorie
reis recognize sons.reis enregistrement.wav
```
Documentation : `docs/README.md`. Mesures : `docs/BENCHMARKS.md`.

Limites : APK signé avec la clé de debug ; app jamais testée sur un vrai téléphone.
