#!/usr/bin/env bash
# One search variant against three house bots, 4 players, same seeds for each.
# Usage: tuning/run_search_experiments.sh GAMES VARIANT...
cd "$(dirname "$0")/.."
games=$1; shift
for v in "$@"; do
  out="tuning/exp_${v}_${games}.txt"
  ./target/release/tournament --bots "$v",house,house,house --sizes 4 --games "$games" --seed 2024 > "$out" 2>&1
  printf '%-34s ' "$v"; grep -E "^$v " "$out" | awk '{print "win " $3 "  place " $5 "  score " $6 "  ms/move " $8}'
done
