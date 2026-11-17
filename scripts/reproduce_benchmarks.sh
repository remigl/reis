#!/usr/bin/env bash
# Full, deterministic benchmark pipeline (≈15 min on 2 cores).
# Needs: Rust, Python 3 + numpy + scipy, ffmpeg built with libchromaprint/libmp3lame/libopus.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; W="${1:-$ROOT/bench-work}"; R="$ROOT/target/release/reis"
cargo build --release --manifest-path "$ROOT/Cargo.toml"
mkdir -p "$W" && cd "$W"
[ -d db ]  || python3 "$ROOT/benchmarks/gen_corpus.py" db 300 60 1234
[ -d neg ] || python3 "$ROOT/benchmarks/gen_corpus.py" neg 60 60 999
mkdir -p db/art; for i in $(seq 0 29); do [ -f "db/art/Album $i.webp" ] || ffmpeg -v error -y -f lavfi \
  -i "gradients=s=240x240:seed=$i,format=yuv420p" -frames:v 1 -c:v libwebp -quality 70 "db/art/Album $i.webp"; done
[ -d qs ]  || python3 "$ROOT/benchmarks/make_queries.py" db neg qs 60 20 7
$R build --meta db/meta.tsv --lyrics-dir db/lyrics --art-dir db/art --out catalog.reis
$R info catalog.reis | tee info.txt
$R bench catalog.reis qs/queries.tsv --csv landmark.csv | tee landmark.md
python3 "$ROOT/benchmarks/calibrate.py" landmark.csv 0.0003
python3 -I "$ROOT/benchmarks/bench_chromaprint.py" db qs/queries.tsv chromaprint.csv | tee chromaprint.md
python3 "$ROOT/benchmarks/sim_engine.py" "$R" catalog.reis db neg sim 20 | tee engine.md
$R scale catalog.reis --copies 100 --queries qs/queries.tsv | tee scale.md
