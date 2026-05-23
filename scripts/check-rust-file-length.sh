#!/usr/bin/env bash
set -euo pipefail

limit="${RUST_FILE_LINE_LIMIT:-1000}"
failed=0

while IFS= read -r file; do
    lines="$(wc -l < "$file" | tr -d ' ')"
    if (( lines > limit )); then
        printf '%s has %s lines; limit is %s\n' "$file" "$lines" "$limit" >&2
        failed=1
    fi
done < <(rg --files -g '*.rs' -g '!target/**')

if (( failed )); then
    printf 'Split large Rust files, or rerun with RUST_FILE_LINE_LIMIT=<n> when there is a deliberate exception.\n' >&2
    exit 1
fi
