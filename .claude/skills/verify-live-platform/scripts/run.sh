#!/usr/bin/env bash
# Run one journey under the lock and keep its evidence.
# Usage: bash .claude/skills/verify-live-platform/scripts/run.sh <journey>   (journeys/<journey>.mjs)
# Evidence: $VERIFY_HOME/evidence/<UTC time>-<journey>/ with log.txt, checks.tsv, *.png, meta.txt
set -uo pipefail
. "$(dirname "$0")/env.sh"
journey="${1:?usage: run.sh <journey>}"
script="$SKILL_DIR/scripts/journeys/$journey.mjs"
[ -f "$script" ] || { echo "no journey $script" >&2; exit 2; }

if ! mkdir "$LOCK" 2>/dev/null; then
    echo "another run holds $LOCK (pid $(cat "$LOCK/pid" 2>/dev/null)); not driving a shared stack twice" >&2
    exit 3
fi
echo $$ > "$LOCK/pid"
trap 'rm -rf "$LOCK"' EXIT

run_dir="$EVIDENCE/$(date -u +%Y%m%dT%H%M%SZ)-$journey"
mkdir -p "$run_dir"
{
    echo "journey=$journey"
    echo "commit=$(git -C "$REPO" rev-parse HEAD)"
    [ -f "$STATE" ] && sed 's/^/launch_/' "$STATE"
} > "$run_dir/meta.txt"

VERIFY_RUN_DIR="$run_dir" node "$script" 2>&1 | tee "$run_dir/log.txt"
code=${PIPESTATUS[0]}
echo "exit=$code" >> "$run_dir/meta.txt"
echo "evidence: $run_dir"
exit "$code"
