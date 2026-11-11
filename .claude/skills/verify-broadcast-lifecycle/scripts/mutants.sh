#!/usr/bin/env bash
# Shows that each property can still fail: runs every spec/mutants/<name>.cfg
# (one broken variant of the code from spec/Mutants.tla, one property) and
# expects TLC to report that property violated.
#   bash .claude/skills/verify-broadcast-lifecycle/scripts/mutants.sh            # all
#   bash .claude/skills/verify-broadcast-lifecycle/scripts/mutants.sh <name> ... # some
set -uo pipefail

skill_dir=$(cd "$(dirname "$0")/.." && pwd)
TLA_HOME=${TLA_HOME:-$HOME/.cache/tlaplus}
cd "$skill_dir/spec/mutants"
names=("$@")
[ ${#names[@]} -eq 0 ] && names=($(ls *.cfg | sed 's/\.cfg$//'))

failed=()
for name in "${names[@]}"; do
  property=$(sed -n 's/^PROPERTY[[:space:]]*\([A-Za-z]*\).*/\1/p' "$name.cfg")
  bash "$skill_dir/scripts/check.sh" "$skill_dir/spec" "mutants/$name.cfg" Mutants >/dev/null 2>&1
  log=$TLA_HOME/last-run.txt
  if grep -q "Error: Action property $property is violated\|Error: Invariant $property is violated" "$log" \
     || grep -q "Error: Temporal properties were violated" "$log"; then
    echo "caught   $name ($property)"
  else
    echo "MISSED   $name ($property): see $log"
    failed+=("$name")
  fi
done

if [ ${#failed[@]} -eq 0 ]; then echo "MUTANTS OK (${#names[@]} caught)"; exit 0; fi
echo "MUTANTS FAILED: ${failed[*]}" >&2
exit 1
