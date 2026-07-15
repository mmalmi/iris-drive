## Project

`iris-drive` is a Drive-style sync app over local `htree` (`~/src/hashtree`): shared Rust core, native shells, UniFFI app-core, CAS + Nostr identity/peers/discovery/share invites. No storage protocol.

## Rules

- TDD non-trivial changes; prefer deterministic e2e with real htree or in-process equivalent. Avoid mocks and flaky time.
- Fix errors you hit. Keep files small.
- Nostr subscriptions, peer discovery, mutable roots: keep open; one quiet window is not absence.
- Distinguish explicit misses from timeouts. Bound per-peer work/memory; heartbeats without bytes do not keep requests alive.
- Mounts are virtual provider surfaces only: FileProvider, FUSE, WinFsp, SAF over htree/provider roots. No normal-folder fallback.
- No fallbacks: fix root cause or show the error.
- Perf experiments go in `docs/EXPERIMENTS.md`; omit private identifiers unless asked.
- Never pull/rebase from `htree://self/*`; it is publish-only.
- After relevant checks pass, commit and push to `htree://self/iris-drive`.
- Do not duplicate native UI logic; prefer Rust helpers.
- CLI is `idrive`; `htree` is the hashtree CLI.
