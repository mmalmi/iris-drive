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

grep -Fq "daemon_tasks.abort_all().await;" \
  "$ROOT/crates/iris-drive-cli/src/daemon/runtime.rs" || {
    echo "daemon runtime must abort and await client-owning background tasks before exit" >&2
    exit 1
  }

grep -Fq "drop(notifications);" \
  "$ROOT/crates/iris-drive-cli/src/daemon/runtime.rs" || {
    echo "daemon runtime must drop Nostr notification receivers before client shutdown" >&2
    exit 1
  }

grep -Fq "Arc::try_unwrap(sync)" \
  "$ROOT/crates/iris-drive-cli/src/daemon/runtime.rs" || {
    echo "daemon runtime must own the FIPS sync before clean shutdown when possible" >&2
    exit 1
  }

grep -Fq "sync.shutdown().await" \
  "$ROOT/crates/iris-drive-cli/src/daemon/runtime.rs" || {
    echo "daemon runtime must explicitly shut down FIPS sync before runtime teardown" >&2
    exit 1
  }

grep -Fq "pub async fn shutdown(mut self)" \
  "$ROOT/crates/iris-drive-core/src/fips_sync.rs" || {
    echo "FipsBlockSync must expose an owned async shutdown path" >&2
    exit 1
  }

grep -Fq "pub(crate) struct DaemonTaskSet" \
  "$ROOT/crates/iris-drive-cli/src/daemon.rs" || {
    echo "daemon must keep join handles for shutdown-owned background tasks" >&2
    exit 1
  }

echo "NOSTR_CLIENT_SHUTDOWN_OK"
