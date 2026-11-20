#!/usr/bin/env bash
# ESC-50 protocol for the semantic + exact engines (≈ 40 min on 2 cores).
#   benchmarks/esc50.sh ESC50_DIR MODELS_DIR [WORK_DIR]
# ESC-50: git clone https://github.com/karolpiczak/ESC-50 (CC BY-NC 3.0, not redistributed here)
# Models: mn04_as.rnm / mn10_as.rnm from the models-v1 release.
set -euo pipefail
ESC="$1"; MODELS="$2"; ROOT="$(cd "$(dirname "$0")/.." && pwd)"; W="${3:-$ROOT/bench-work/esc50}"; R="$ROOT/target/release/reis"
cargo build --release --manifest-path "$ROOT/Cargo.toml"
mkdir -p "$W" && cd "$W"
[ -d esc ] || python3 "$ROOT/benchmarks/prepare_esc50.py" "$ESC" esc
CONDS="clean speaker noise10 speaker_noise5"
run() { # name model [index options...]
  local name=$1 model=$2; shift 2
  [ -f "$name.reis" ] || "$R" index esc/ref -o "$name.reis" --model "$MODELS/$model" "$@" 2>&1 | grep -E "wrote|semantic:"
  thr=$("$R" info "$name.reis" | sed -n 's/.*threshold \([0-9.]*\).*/\1/p')
  args=()
  for c in $CONDS; do "$R" eval "$name.reis" "esc/q_$c" --engines semantic --threshold -1 --csv "$name.$c.csv" > "$name.$c.txt" 2>/dev/null; args+=("$c:$name.$c.csv"); done
  echo; echo "### $name (threshold $thr)"; python3 "$ROOT/benchmarks/esc50_report.py" esc/held_out.txt "$thr" "${args[@]}"
  grep -h "time per query" "$name.clean.txt"
}
run mn10 mn10_as.rnm --engines exact,semantic
run mn10_augment mn10_as.rnm --engines semantic --augment speaker
run mn04 mn04_as.rnm --engines semantic
echo; echo "### exact engine, same recording (3 s excerpts of references)"
for c in $CONDS; do echo -n "$c: "; "$R" eval mn10.reis "esc/same_$c" --engines exact 2>/dev/null | grep "top-1"; done
echo -n "other recordings of the same category (clean): "; "$R" eval mn10.reis esc/q_clean --engines exact 2>/dev/null | grep "top-1"
