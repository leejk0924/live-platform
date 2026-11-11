#!/usr/bin/env bash
# Lists changes to the code the model covers since the sync commit recorded in
# SKILL.md, committed or not. Read-only.
#   bash .claude/skills/verify-broadcast-lifecycle/scripts/drift.sh          # file list
#   bash .claude/skills/verify-broadcast-lifecycle/scripts/drift.sh --diff   # full diff
# Exit 0 when nothing covered changed, 1 when something did (re-read those
# changes against the model before trusting a pass).
set -euo pipefail

skill_md=$(cd "$(dirname "$0")/.." && pwd)/SKILL.md
sync=$(sed -n 's/^Sync commit: `\([0-9a-f]\{40\}\)`.*/\1/p' "$skill_md")
mapfile -t covered < <(sed -n '/<!-- covered:start -->/,/<!-- covered:end -->/p' "$skill_md" \
  | sed -n 's/^- `\([^`]*\)`.*/\1/p')
[ -n "$sync" ] && [ ${#covered[@]} -gt 0 ] || { echo "no sync commit or covered list in $skill_md" >&2; exit 2; }

cd "$(git rev-parse --show-toplevel)"
git cat-file -e "$sync^{commit}" 2>/dev/null || { echo "sync commit $sync is not in this clone (git fetch?)" >&2; exit 2; }

if [ "${1:-}" = "--diff" ]; then
  git diff "$sync" -- "${covered[@]}"
fi
changed=$(git diff --name-only "$sync" -- "${covered[@]}")
if [ -z "$changed" ]; then
  echo "no drift: covered code unchanged since ${sync:0:7}"
  exit 0
fi
echo "covered code changed since ${sync:0:7}:"
echo "$changed" | sed 's/^/  /'
exit 1
