#!/usr/bin/env bash
# Launch gauntlet-rs. Builds on first run. Pass extra options through (e.g. --level levelB1).
set -euo pipefail
cd "$(dirname "$0")"
[ -x target/release/gauntlet ] || cargo build --release
DATA="${GDL_DATA:-}"
if [ -z "$DATA" ]; then
  for c in game-data "$HOME/Games/GameCube/Gauntlet - Dark Legacy (USA) (Rev 1).nkit.iso"; do
    [ -e "$c" ] && DATA="$c" && break
  done
fi
exec ./target/release/gauntlet ${DATA:+--data "$DATA"} "$@"
