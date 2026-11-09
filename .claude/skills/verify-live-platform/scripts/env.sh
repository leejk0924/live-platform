# Sourced by the other scripts. Everything the skill writes lives under $VERIFY_HOME, outside the repo.
VERIFY_HOME="${VERIFY_HOME:-$HOME/.cache/verify-live-platform}"
# Git Bash on Windows: hand node a C:/... path, not /c/...
command -v cygpath >/dev/null 2>&1 && VERIFY_HOME="$(cygpath -m "$VERIFY_HOME")"
SKILL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO="$(git -C "$SKILL_DIR" rev-parse --show-toplevel)"
STATE="$VERIFY_HOME/state"        # launch bookkeeping: commit, dirty flag, whether we started the stack
LOCK="$VERIFY_HOME/lock"          # one driver at a time: the stack is shared
EVIDENCE="$VERIFY_HOME/evidence"  # proof survives cleanup
PLAYWRIGHT_VERSION=1.56.1
export VERIFY_HOME
mkdir -p "$VERIFY_HOME" "$EVIDENCE"
