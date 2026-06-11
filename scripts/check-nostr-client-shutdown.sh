#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if grep -R --line-number -E '(^|[^[:alnum:]_])(client|relay_client)\\.disconnect\\(\\)\\.await' \
  "$ROOT/crates" >/tmp/iris-drive-nostr-disconnects.$$; then
  cat /tmp/iris-drive-nostr-disconnects.$$ >&2
  rm -f /tmp/iris-drive-nostr-disconnects.$$
  echo "use iris_drive_core::relay_sync::shutdown_client instead of plain nostr client disconnect" >&2
  exit 1
fi
rm -f /tmp/iris-drive-nostr-disconnects.$$

grep -Fq "shutdown_client_for_process_exit(client).await;" \
  "$ROOT/crates/iris-drive-cli/src/daemon/runtime.rs" || {
    echo "daemon runtime must use shutdown_client_for_process_exit on exit" >&2
    exit 1
  }

echo "NOSTR_CLIENT_SHUTDOWN_OK"
