#!/usr/bin/env bash
# One-time: install the browser driver into $VERIFY_HOME (not the repo) and check Chrome exists.
# Usage: bash .claude/skills/verify-live-platform/scripts/setup.sh
set -euo pipefail
. "$(dirname "$0")/env.sh"

chrome="${CHROME:-C:/Program Files/Google/Chrome/Application/chrome.exe}"
[ -f "$chrome" ] || { echo "Chrome not found at $chrome; set CHROME=<path to chrome>" >&2; exit 1; }

if [ ! -f "$VERIFY_HOME/node_modules/playwright-core/package.json" ] ||
   ! grep -q "\"version\": \"$PLAYWRIGHT_VERSION\"" "$VERIFY_HOME/node_modules/playwright-core/package.json"; then
    [ -f "$VERIFY_HOME/package.json" ] || echo '{"name":"verify-live-platform","private":true}' > "$VERIFY_HOME/package.json"
    npm install --prefix "$VERIFY_HOME" --no-audit --no-fund --save-exact "playwright-core@$PLAYWRIGHT_VERSION" >/dev/null
fi
echo "ok: playwright-core $PLAYWRIGHT_VERSION in $VERIFY_HOME, chrome $chrome"
