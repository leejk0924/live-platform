#!/usr/bin/env bash
# Checks every commit in a range against the commit rules in CLAUDE.md.
# Usage: scripts/check-commit-msg.sh [<range>]   (default: origin/main..HEAD)
set -euo pipefail

range="${1:-origin/main..HEAD}"
types='feat|fix|refactor|perf|test|docs|build|ci|chore'
failed=0

for sha in $(git rev-list --no-merges "$range"); do
    subject=$(git log -1 --format=%s "$sha")
    body=$(git log -1 --format=%B "$sha")
    last=$(printf '%s\n' "$body" | sed '/^[[:space:]]*$/d' | tail -n 1)
    problems=()

    [[ "$subject" =~ ^($types):\ .+ ]] || problems+=("subject must be '<type>: <summary>' with type in {$types}")
    [[ "$subject" != *. ]] || problems+=("subject must not end with a period")
    [[ "$last" =~ ^Refs\ \#[0-9]+$ ]] || problems+=("last line must be 'Refs #<issue>'")
    if printf '%s\n' "$body" | grep -qi '^co-authored-by:'; then
        problems+=("no Co-Authored-By lines")
    fi

    if ((${#problems[@]})); then
        failed=1
        echo "${sha:0:7} $subject"
        printf '  - %s\n' "${problems[@]}"
    fi
done

if ((failed)); then
    exit 1
fi
echo "ok: $(git rev-list --no-merges --count "$range") commits in $range"
