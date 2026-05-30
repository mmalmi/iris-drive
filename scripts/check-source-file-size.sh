#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ALLOWLIST="${SOURCE_FILE_SIZE_ALLOWLIST:-$ROOT/scripts/source-file-size-allowlist.txt}"
DEFAULT_LINE_LIMIT="${SOURCE_FILE_LINE_LIMIT:-1000}"
failed=0

allowlisted_paths=()
allowlisted_limits=()
allowlisted_seen=()

if [[ -f "$ALLOWLIST" ]]; then
  while read -r path limit _reason; do
    [[ -z "${path:-}" || "${path:0:1}" == "#" ]] && continue
    if [[ -z "${limit:-}" || ! "$limit" =~ ^[0-9]+$ ]]; then
      printf '%s: invalid allowlist entry: %s %s\n' "$ALLOWLIST" "$path" "${limit:-}" >&2
      failed=1
      continue
    fi
    allowlisted_paths+=("$path")
    allowlisted_limits+=("$limit")
    allowlisted_seen+=(0)
  done < "$ALLOWLIST"
fi

line_limit_for() {
  local file="$1"

  case "$file" in
    *.rs) printf '%s\n' "${RUST_FILE_LINE_LIMIT:-$DEFAULT_LINE_LIMIT}" ;;
    *.swift) printf '%s\n' "${SWIFT_FILE_LINE_LIMIT:-$DEFAULT_LINE_LIMIT}" ;;
    *.kt|*.kts) printf '%s\n' "${KOTLIN_FILE_LINE_LIMIT:-$DEFAULT_LINE_LIMIT}" ;;
    *.java)
      printf '%s\n' "${JAVA_FILE_LINE_LIMIT:-${KOTLIN_FILE_LINE_LIMIT:-$DEFAULT_LINE_LIMIT}}"
      ;;
    *.cs) printf '%s\n' "${CS_FILE_LINE_LIMIT:-$DEFAULT_LINE_LIMIT}" ;;
    *.sh) printf '%s\n' "${SHELL_FILE_LINE_LIMIT:-$DEFAULT_LINE_LIMIT}" ;;
    *.js|*.mjs|*.ts|*.tsx) printf '%s\n' "${JS_FILE_LINE_LIMIT:-$DEFAULT_LINE_LIMIT}" ;;
    *.ps1) printf '%s\n' "${POWERSHELL_FILE_LINE_LIMIT:-$DEFAULT_LINE_LIMIT}" ;;
    *.xaml|*.xml) printf '%s\n' "${XML_FILE_LINE_LIMIT:-$DEFAULT_LINE_LIMIT}" ;;
    *.yml|*.yaml|*.toml|*.gradle)
      printf '%s\n' "${CONFIG_FILE_LINE_LIMIT:-$DEFAULT_LINE_LIMIT}"
      ;;
    *) printf '%s\n' "$DEFAULT_LINE_LIMIT" ;;
  esac
}

tracked_source_files() {
  git -C "$ROOT" ls-files -z -- \
    '*.rs' '*.swift' '*.kt' '*.kts' '*.java' '*.cs' \
    '*.sh' '*.js' '*.mjs' '*.ts' '*.tsx' '*.ps1' \
    '*.xaml' '*.xml' '*.yml' '*.yaml' '*.toml' '*.gradle'
}

while IFS= read -r -d '' file; do
  full_path="$ROOT/$file"
  [[ -f "$full_path" ]] || continue

  limit="$(line_limit_for "$file")"
  base_limit="$limit"
  allowlist_index=-1
  for ((index = 0; index < ${#allowlisted_paths[@]}; index++)); do
    if [[ "${allowlisted_paths[$index]}" == "$file" ]]; then
      allowlist_index="$index"
      break
    fi
  done
  if (( allowlist_index >= 0 )); then
    limit="${allowlisted_limits[$allowlist_index]}"
    allowlisted_seen[$allowlist_index]=1
  fi

  lines="$(wc -l < "$full_path" | tr -d '[:space:]')"
  if (( lines > limit )); then
    if [[ "$limit" != "$base_limit" ]]; then
      printf '%s has %s lines; allowlisted limit is %s (default %s)\n' \
        "$file" "$lines" "$limit" "$base_limit" >&2
    else
      printf '%s has %s lines; limit is %s\n' "$file" "$lines" "$limit" >&2
    fi
    failed=1
  fi
done < <(tracked_source_files)

for ((index = 0; index < ${#allowlisted_paths[@]}; index++)); do
  path="${allowlisted_paths[$index]}"
  if [[ "${allowlisted_seen[$index]}" != "1" ]]; then
    printf '%s: stale allowlist entry for missing or untracked file: %s\n' \
      "$ALLOWLIST" "$path" >&2
    failed=1
  fi
done

if (( failed )); then
  printf '\nSource file size guard failed. Split large files, or add a narrow, explained entry to %s.\n' \
    "$ALLOWLIST" >&2
  exit 1
fi
