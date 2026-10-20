#!/usr/bin/env bash

setup_host_to_file() {
  local label="$1" destination="$2" idx
  idx="$(label_index "$label")"
  setup_host "$label"
  {
    printf 'base=%s\n' "${BASES[$idx]}"
    printf 'config=%s\n' "${CONFIGS[$idx]}"
    printf 'work=%s\n' "${WORKS[$idx]}"
    printf 'idrive=%s\n' "${IDRIVES[$idx]}"
    printf 'log=%s\n' "${LOGS[$idx]}"
    printf 'err=%s\n' "${ERRS[$idx]}"
    printf 'pid=%s\n' "${PIDS[$idx]}"
  } >"$destination"
}

apply_host_setup_file() {
  local label="$1" source="$2" key value
  while IFS='=' read -r key value; do
    case "$key" in
      base|config|work|idrive|log|err|pid) set_host_value "$label" "$key" "$value" ;;
    esac
  done <"$source"
}

setup_host_group_to_files() {
  local metadata_dir="$1" labels="$2" label
  while IFS= read -r label; do
    [[ -n "$label" ]] || continue
    echo "setting up $label ($(host_value "$label" ssh))"
    setup_host_to_file "$label" "$metadata_dir/$label"
  done <<<"$labels"
}

setup_hosts_parallel() {
  local meta_dir label host_key group_index index status=0
  local -a group_hosts=() group_labels=()
  meta_dir="$(mktemp -d -t iris-drive-e2e-setup.XXXXXX)"
  for label in "${LABELS[@]}"; do
    host_key="$(host_value "$label" ssh)"
    group_index=-1
    for index in "${!group_hosts[@]}"; do
      if [[ "${group_hosts[$index]}" == "$host_key" ]]; then
        group_index="$index"
        break
      fi
    done
    if [[ "$group_index" -lt 0 ]]; then
      group_hosts+=("$host_key")
      group_labels+=("$label")
    else
      group_labels[$group_index]+=$'\n'"$label"
    fi
  done

  parallel_group_begin e2e-host-setup
  for index in "${!group_hosts[@]}"; do
    parallel_group_start "host-$index" setup_host_group_to_files \
      "$meta_dir" "${group_labels[$index]}"
  done
  parallel_group_wait || status=$?
  if [[ "$status" -eq 0 ]]; then
    for label in "${LABELS[@]}"; do
      apply_host_setup_file "$label" "$meta_dir/$label"
    done
  fi
  rm -rf "$meta_dir"
  return "$status"
}
