**Reis v0.4.0** — paroles depuis Internet, dans l'app.

### Nouveau
- **Chercher les paroles** pendant l'écoute : Reis déduit le titre et l'artiste (tags, ou nom du fichier comme `03 - Daft Punk - One More Time (Official Video).mp3`), interroge [LRCLIB](https://lrclib.net) et propose une liste. Pour chaque résultat, tu vois s'il est synchronisé ou texte seul et l'écart de durée. Touche un résultat pour un aperçu, puis choisis. Le choix est gardé sur le téléphone.
- **À la création de la bibliothèque** : option *Télécharger les paroles manquantes* pour récupérer automatiquement les paroles synchronisées de même durée.
- Les paroles non synchronisées s'affichent en texte simple.

C'est la seule fonction en ligne, facultative. Seuls le titre, l'artiste, l'album et la durée sont envoyés, jamais l'audio.

### Toujours là
Création de la bibliothèque sur le téléphone (Musiques / Types de sons / Les deux), reconnaissance hors ligne, paroles synchronisées.

### Fichiers
| fichier | contenu |
|---|---|
| `reis-v0.4.0-android.apk` | app Android 7.0+ |
| `reis-v0.4.0-windows-x64.zip` | app Windows |
| `reis-cli-v0.4.0-…` | outil `reis` + modèles |
| `mn10_as.rnm`, `mn04_as.rnm` | modèles EfficientAT (MIT) |

Limites : APK signé avec la clé de debug ; jamais testé sur un vrai téléphone.
