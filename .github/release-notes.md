**Reis v0.3.0** — l'app crée elle-même ta bibliothèque d'empreintes, **sur le téléphone**, à partir de tes musiques et de tes sons. Plus besoin d'ordinateur.

### Dans l'app
1. Premier lancement → **Créer ma bibliothèque** (ou, plus tard, bouton 🎵 « Bibliothèque »).
2. Choisis ce que Reis doit reconnaître :
   - **Musiques** : le morceau exact, sa position et ses paroles synchronisées (si un `.lrc` est à côté du fichier ou ajouté avec lui) ;
   - **Types de sons** : le même genre de son, même un autre enregistrement ; chaque dossier devient une catégorie ;
   - **Les deux**.
3. Coche les dossiers de ton téléphone (ou ajoute des fichiers), puis **Créer les empreintes**. Tout se fait hors ligne.

Tous les formats lisibles par Android sont acceptés (MP3, AAC/M4A, FLAC, OGG, Opus, WAV…).

### Fichiers
| fichier | contenu |
|---|---|
| `reis-v0.3.0-android.apk` | app Android 7.0+ (modèle neuronal inclus) |
| `reis-v0.3.0-windows-x64.zip` | app Windows |
| `reis-cli-v0.3.0-…` | outil `reis` + modèles, pour construire de grosses bibliothèques sur ordinateur |
| `mn10_as.rnm`, `mn04_as.rnm` | modèles EfficientAT (MIT) |

### Limites
- APK signé avec la clé de debug (installation manuelle).
- Fonction jamais testée sur un vrai téléphone : merci de remonter les problèmes.
- Paroles : seulement les `.lrc` lisibles par l'app (choisis-les avec les sons dans « Ajouter des fichiers » si elles ne sont pas trouvées automatiquement).
- La bibliothèque est reconstruite entièrement à chaque fois (pas encore d'ajout incrémental).
