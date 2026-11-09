#!/usr/bin/env bash
# Tear down only what this skill started. Never touches $VERIFY_HOME/evidence.
# Usage: bash .claude/skills/verify-live-platform/scripts/cleanup.sh
set -uo pipefail
. "$(dirname "$0")/env.sh"
cd "$REPO"

# A lock whose pid is gone belongs to a crashed run: drop it. A live pid is someone else's run: leave it.
if [ -d "$LOCK" ]; then
    pid="$(cat "$LOCK/pid" 2>/dev/null)"
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
        echo "run $pid still holds the lock; not cleaning up under it" >&2
        exit 1
    fi
    rm -rf "$LOCK"
    echo "removed stale lock"
fi

# Playwright launches Chrome with its own temp profile and closes it when the journey exits.
# Kill only Chrome processes whose command line carries a Playwright profile, i.e. ones a crashed journey left.
if command -v powershell.exe >/dev/null 2>&1; then
    powershell.exe -NoProfile -Command \
        "Get-CimInstance Win32_Process -Filter \"Name='chrome.exe'\" | Where-Object { \$_.CommandLine -like '*playwright_chromiumdev_profile*' } | ForEach-Object { Stop-Process -Id \$_.ProcessId -Force; 'stopped orphan chrome ' + \$_.ProcessId }"
else
    pkill -f 'playwright_chromiumdev_profile' && echo "stopped orphan chrome" || true
fi

# Stop the stack only if launch.sh found it down and started it.
if [ -f "$STATE" ] && grep -q '^started_by_us=yes' "$STATE"; then
    docker compose stop
    echo "stopped the compose stack this skill started (volumes kept)"
else
    echo "left the compose stack running: it was already up before launch"
fi
rm -f "$STATE"
echo "evidence kept in $EVIDENCE"
