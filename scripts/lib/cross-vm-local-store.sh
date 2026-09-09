# shellcheck shell=bash

E2E_LOCAL_SHARED_DATA_DIR=""

prepare_local_shared_store() {
  local label last_local="" base
  for label in "${LABELS[@]}"; do
    if [[ "$(host_value "$label" kind)" == posix && "$(host_value "$label" ssh)" == local ]]; then
      last_local="$label"
    fi
  done
  [[ -n "$last_local" ]] || return 0
  base="$(host_value "$last_local" base)"
  [[ -d "$base" && ! -L "$base" && "${base##*/}" == "iris-drive-e2e-$RUN_ID-"* ]] || {
    echo "local shared store requires this run's owned temporary base" >&2
    return 1
  }
  # The last local role is stopped last, so its normal base cleanup removes
  # the shared pool only after every local daemon has stopped using it.
  mkdir -m 700 "$base/shared-hashtree" || return 1
  E2E_LOCAL_SHARED_DATA_DIR="$base/shared-hashtree"
}

run_local_e2e_script() {
  local script="$1"
  if [[ -n "$E2E_LOCAL_SHARED_DATA_DIR" ]]; then
    printf "%s\n" "$script" | env HTREE_DATA_DIR="$E2E_LOCAL_SHARED_DATA_DIR" bash -se
  else
    printf "%s\n" "$script" | bash -se
  fi
}

assert_local_shared_store_removed() {
  [[ -z "$E2E_LOCAL_SHARED_DATA_DIR" ]] || {
    [[ ! -e "$E2E_LOCAL_SHARED_DATA_DIR" && ! -L "$E2E_LOCAL_SHARED_DATA_DIR" ]] || {
      echo "owned local shared fixture storage remains after daemon cleanup" >&2
      return 1
    }
  }
}

# Setup precedes discovery of the remote base. Later commands, including the
# foreground Windows daemon SSH session, must use the same run-owned pool.
remote_shared_store_script() {
  local label="$1" script="$2" base kind
  base="$(host_value "$label" base)"
  kind="$(host_value "$label" kind)"
  if [[ -n "$base" && "$(host_value "$label" ssh)" != local ]]; then
    if [[ "$kind" == windows ]]; then
      printf '$env:HTREE_DATA_DIR = %s\n' "$(ps_quote "$base\shared-hashtree")"
    else
      printf 'export HTREE_DATA_DIR=%s\n' "$(sh_quote "$base/shared-hashtree")"
    fi
  fi
  printf '%s\n' "$script"
}
