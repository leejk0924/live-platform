#!/usr/bin/env bash
# Inner hexagonal crates may depend only on what is allowlisted here, so an
# adapter crate cannot slip in under a name deny.toml does not know.
set -euo pipefail

declare -A allowed=(
    [domain]="thiserror"
    [application]="domain thiserror async-trait"
)

failed=0
for crate in domain application; do
    deps=$(cargo tree -p "$crate" -e normal,build --depth 1 --prefix none --format '{p}' \
        --target all --all-features --locked \
        | tail -n +2 | awk '{print $1}' | sort -u)
    crate_ok=1
    for dep in $deps; do
        if [[ " ${allowed[$crate]} " != *" $dep "* ]]; then
            echo "$crate depends on $dep, which is not in its allowlist"
            crate_ok=0
            failed=1
        fi
    done
    if [[ $crate_ok -eq 1 ]]; then
        echo "ok: $crate"
    fi
done

# deny.toml lets api wrap sqlx for its database tests; production code must
# still reach Postgres through infrastructure.
if cargo tree -p api -e normal --depth 1 --prefix none --format '{p}' --locked     | tail -n +2 | awk '{print $1}' | grep -qx sqlx; then
    echo "api depends on sqlx outside dev-dependencies"
    failed=1
else
    echo "ok: api"
fi

exit "$failed"
