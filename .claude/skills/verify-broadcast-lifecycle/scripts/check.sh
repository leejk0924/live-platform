#!/usr/bin/env bash
# Model-checks the broadcast lifecycle spec with TLC in a pinned JRE container.
#   bash .claude/skills/verify-broadcast-lifecycle/scripts/check.sh
#   bash .claude/skills/verify-broadcast-lifecycle/scripts/check.sh <spec dir> <cfg> [module]
# <cfg> is relative to <spec dir>; [module] defaults to BroadcastLifecycle.
# Exit 0 only when TLC finishes with no error. The full TLC output, with any
# counterexample, is in $TLA_HOME/last-run.txt.
set -euo pipefail

TLA_VERSION=1.7.4
TLA_SHA256=936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
JRE_IMAGE=eclipse-temurin@sha256:0b99b3376fbb50dbf9c0af8535fdd5b8eb0d3fdfc8b1e4967f383b6919f6901a # 21-jre
TLA_HOME=${TLA_HOME:-$HOME/.cache/tlaplus}
TLC_HEAP=${TLC_HEAP:-3g}

skill_dir=$(cd "$(dirname "$0")/.." && pwd)
spec_dir=$(cd "${1:-$skill_dir/spec}" && pwd)
cfg=${2:-BroadcastLifecycle.cfg}
module=${3:-BroadcastLifecycle}

jar=$TLA_HOME/tla2tools-$TLA_VERSION.jar
mkdir -p "$TLA_HOME"
if [ ! -f "$jar" ]; then
  curl -fsSL -o "$jar.part" \
    "https://github.com/tlaplus/tlaplus/releases/download/v$TLA_VERSION/tla2tools.jar"
  mv "$jar.part" "$jar"
fi
echo "$TLA_SHA256  $jar" | sha256sum -c --quiet - || { echo "tla2tools checksum mismatch: $jar" >&2; exit 2; }

# Docker on Windows needs Windows paths; elsewhere these are plain paths.
host() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else echo "$1"; fi; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp "$spec_dir"/*.tla "$work/"
cp "$spec_dir/$cfg" "$work/run.cfg"

started=$(date +%s)
set +e
MSYS_NO_PATHCONV=1 docker run --rm \
  -v "$(host "$work"):/spec" -v "$(host "$jar"):/tla2tools.jar:ro" -w /spec \
  "$JRE_IMAGE" \
  java -Xmx"$TLC_HEAP" -XX:+UseParallelGC -cp /tla2tools.jar tlc2.TLC \
    -workers auto -cleanup -metadir /tmp/tlc -config run.cfg "$module.tla" \
  > "$TLA_HOME/last-run.txt" 2>&1
tlc=$?
set -e
took=$(( $(date +%s) - started ))

grep -E "^Error|violated|states generated|^State [0-9]+:|Back to state" "$TLA_HOME/last-run.txt" | head -60 || true
if [ "$tlc" -eq 0 ] && grep -q "Model checking completed. No error has been found." "$TLA_HOME/last-run.txt"; then
  echo "MODEL OK ($cfg, ${took}s)"
  exit 0
fi
echo "MODEL FAILED ($cfg, tlc exit $tlc, ${took}s). Full output: $TLA_HOME/last-run.txt" >&2
exit 1
