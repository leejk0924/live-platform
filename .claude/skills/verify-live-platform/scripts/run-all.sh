#!/usr/bin/env bash
# Run every journey in turn, then the shell hardening check. Exit 0 only when all pass.
# Usage: bash .claude/skills/verify-live-platform/scripts/run-all.sh [journey ...]   (default: all of journeys/)
# donation needs internet access to the Toss sandbox; pass the others by name to skip it.
set -uo pipefail
. "$(dirname "$0")/env.sh"
cd "$REPO"
journeys=("$@")
[ ${#journeys[@]} -eq 0 ] && journeys=(watch publish comments recording donation)
failed=()
for j in "${journeys[@]}"; do
    echo "=== $j"
    out="$(bash "$SKILL_DIR/scripts/run.sh" "$j" 2>&1)"; code=$?
    grep -E 'FAIL|VERIFY|evidence:|another run' <<<"$out"
    [ "$code" -eq 0 ] || failed+=("$j")
done
if [ $# -eq 0 ]; then
    echo "=== hardening"
    out="$(bash deploy/check-hardening.sh local 2>&1)"; code=$?
    tail -3 <<<"$out"
    [ "$code" -eq 0 ] || failed+=(hardening)
fi
if [ ${#failed[@]} -eq 0 ]; then echo "ALL OK"; else echo "FAILED: ${failed[*]}"; exit 1; fi
