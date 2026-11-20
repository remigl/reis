# Contribuer à Reis

## Compiler et tester

```bash
cargo build --release            # core + CLI ; aucune crate externe
cargo test --release             # unitaires + bout en bout
cargo clippy --release
cargo build --release --examples -p reis-core
```

Vérifier le runtime neuronal (modèles de la release `models-v1` dans `models/`) :

```bash
target/release/reis model-check models/mn10_as.rnm models/mn10_as.ref.bin
```

App : `scripts/setup_flutter.sh`, puis `scripts/build_android.sh` ou
`scripts/build_desktop.sh`, puis `flutter run`. La CI (`.github/workflows/release.yml`)
compile l'APK et l'app Windows à chaque push sur `main` et publie la release.

## Règles

- **Pas de dépendance** dans `core/` : tout est en bibliothèque standard. Si une
  dépendance devient vraiment nécessaire, la justifier par une mesure.
- **Toute décision de qualité se mesure** : un changement d'empreinte, de seuil
  ou de modèle s'accompagne d'un benchmark avant/après
  (`scripts/reproduce_benchmarks.sh`, `benchmarks/esc50.sh`) et de la mise à
  jour de `docs/BENCHMARKS.md`.
- **Compatibilité des fichiers** : une modification du format `.reis` ou `.rnm`
  incrémente sa version (`library::VERSION`, en-tête `.rnm`) et met à jour
  `docs/FORMATS.md`. Les lecteurs ignorent ce qu'ils ne connaissent pas : préférer
  ajouter une section ou une clé JSON.
- **Pas de données tierces dans le dépôt** (sons, paroles, images, poids).

## Recettes

| je veux… | où |
|---|---|
| ajouter un modèle neuronal | `tools/models/` + [docs/MODELS.md](docs/MODELS.md) |
| une nouvelle opération de réseau | `enum Op` et `Model::forward_mel` dans `core/src/nn.rs` + un test |
| un nouveau moteur d'empreintes | [docs/ARCHITECTURE.md#points-dextension](docs/ARCHITECTURE.md#points-dextension) |
| changer la règle de décision | `Recognizer::decide`, `RecognizerConfig` (`core/src/recognize.rs`) |
| une nouvelle commande | `cli/src/cmds.rs` + `USAGE` dans `cli/src/main.rs` + `docs/GUIDE.md` |
| une autre interface | ABI C (`core/src/ffi.rs`) ; l'app Flutter en est un exemple |
