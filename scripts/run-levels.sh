#!/usr/bin/env bash
# Run the level runner (gauntlet --run-level) over many levels in parallel:
# one process per level, a few at a time, at low priority; then merge the
# per-level results into <out>/summary.md (and .tsv / .json).
#
#   scripts/run-levels.sh [-j N] [-o OUT] [-p WAR,VAL] [-t SECS] [-b BINARY] [levels...]
#
# Levels default to every level folder (game-data/files/Gauntlet/LEVELS/level*).
set -euo pipefail
cd "$(dirname "$0")/.."
jobs=4
out=runner-out
party=WAR
timeout=1800
bin=target/release/gauntlet
while getopts "j:o:p:t:b:" opt; do
  case $opt in
    j) jobs=$OPTARG ;;
    o) out=$OPTARG ;;
    p) party=$OPTARG ;;
    t) timeout=$OPTARG ;;
    b) bin=$OPTARG ;;
    *) exit 2 ;;
  esac
done
shift $((OPTIND - 1))
[ -x "$bin" ] || cargo build --release
levels=("$@")
if [ ${#levels[@]} -eq 0 ]; then
  for d in game-data/files/Gauntlet/LEVELS/level*/; do
    l=$(basename "$d")
    # levelT*: unused test levels.
    case $l in levelT*) continue ;; esac
    levels+=("$l")
  done
fi
mkdir -p "$out"
export GDL_MUTE=1 RUST_LOG=${RUST_LOG:-warn}
tag=${party//,/+}
run_one() {
  local l=$1
  nice -n 15 "$bin" --run-level "$l" --runner-party "$party" \
    --runner-timeout "$timeout" --runner-out "$out" >"$out/$l-$tag.log" 2>&1 || true
  grep -E "^$l:" "$out/$l-$tag.log" | tail -1 || echo "$l: no result (see $out/$l-$tag.log)"
}
export -f run_one
export bin party timeout out tag
printf '%s\n' "${levels[@]}" | xargs -P "$jobs" -I{} bash -c 'run_one "$@"' _ {}
"$bin" --runner-merge "$out"
