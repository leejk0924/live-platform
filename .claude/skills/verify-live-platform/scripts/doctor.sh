#!/usr/bin/env bash
# Read-only: is this instance worth driving? Exit 0 only when every line says ok.
# Usage: bash .claude/skills/verify-live-platform/scripts/doctor.sh
set -uo pipefail
. "$(dirname "$0")/env.sh"
cd "$REPO"
bad=0
say() { echo "$1: $2"; [ "$1" = ok ] || bad=1; }

for svc in caddy api mediamtx postgres; do
    h="$(docker inspect -f '{{.State.Health.Status}}' "$(docker compose ps -q "$svc" 2>/dev/null)" 2>/dev/null)"
    [ "$h" = healthy ] && say ok "$svc healthy" || say FAIL "$svc is '${h:-not running}' (run scripts/launch.sh)"
done

body="$(curl -ksS --max-time 5 https://localhost/api/healthz 2>&1)"
[ "$body" = ok ] && say ok "https://localhost/api/healthz" || say FAIL "healthz answered '$body'"

# Port 443 must be our Caddy, not some other server.
server="$(curl -ksSI --max-time 5 https://localhost/ 2>/dev/null | tr -d '\r' | grep -i '^content-security-policy:' | head -1)"
[ -n "$server" ] && say ok "443 serves this app (CSP header present)" || say FAIL "443 does not look like this app's Caddy"

if [ -f "$STATE" ]; then
    . "$STATE"
    head="$(git rev-parse HEAD)"
    # The images are built from these paths; a commit that touches none of them (docs, this skill) needs no rebuild.
    build_paths=(crates web deploy docker compose.yml Cargo.toml Cargo.lock rust-toolchain.toml)
    if [ "$commit" = "$head" ]; then
        say ok "stack built from HEAD ${head:0:7}"
    elif git diff --quiet "$commit" HEAD -- "${build_paths[@]}" 2>/dev/null; then
        say ok "stack built from ${commit:0:7}; HEAD ${head:0:7} changes nothing it is built from"
    else
        say FAIL "stack built from ${commit:0:7} but HEAD ${head:0:7} changes $(git diff --name-only "$commit" HEAD -- "${build_paths[@]}" 2>/dev/null | wc -l) build files: run scripts/launch.sh"
    fi
    now_dirty=no; [ -n "$(git status --porcelain --untracked-files=no -- "${build_paths[@]}")" ] && now_dirty=yes
    if [ "$dirty" = yes ] || [ "$now_dirty" = yes ]; then
        echo "warn: uncommitted changes at launch=$dirty now=$now_dirty in build paths; relaunch if they matter"
    fi
else
    say FAIL "no launch record ($STATE): run scripts/launch.sh so the build matches HEAD"
fi

if [ -d "$LOCK" ]; then
    pid="$(cat "$LOCK/pid" 2>/dev/null)"
    say FAIL "another run holds $LOCK (pid ${pid:-?}); wait for it, or remove it only if that pid is gone"
else
    say ok "no other run holds the lock"
fi

[ -f "$VERIFY_HOME/node_modules/playwright-core/package.json" ] && say ok "playwright-core installed" \
    || say FAIL "playwright-core missing: run scripts/setup.sh"
chrome="${CHROME:-C:/Program Files/Google/Chrome/Application/chrome.exe}"
[ -f "$chrome" ] && say ok "chrome at $chrome" || say FAIL "chrome not found (set CHROME)"

exit $bad
