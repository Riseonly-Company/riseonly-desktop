#!/usr/bin/env bash
set -euo pipefail

python3 scripts/check-boundaries.py
python3 scripts/gen-mod-tree.py --check
python3 scripts/test-gen-mod-tree.py
RISEONLY_WORKSPACE=/w bash ../riseonly-tools/scripts/sync-to-clients.sh --only desktop --check

if ! cargo fmt --all --check > /tmp/fmt.log 2>&1; then
    echo "::warning::cargo fmt finds $(grep -c '^Diff in' /tmp/fmt.log) unformatted spots (not a gate yet)"
fi

for check in check-design-tokens.py check-tree-parity.py; do
    if ! python3 "scripts/$check" > "/tmp/$check.log" 2>&1; then
        echo "::warning::$check reports findings (not a gate)"
        tail -n 40 "/tmp/$check.log"
    fi
done
