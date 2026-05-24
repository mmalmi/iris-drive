# AGENTS.md

Repo notes for AI coding agents; pair with operator-local instructions.

## Project Shape

`iris-drive` is a Google-Drive-style file sync app on the local `htree` daemon (`~/src/hashtree`). Structure mirrors `~/src/nostr-vpn`: shared Rust core plus per-platform native shells over a UniFFI app-core crate. It does not run its own storage protocol; it consumes hashtree for content-addressed storage and Nostr for identity, peer discovery, and share invites.

## Shared Rules

- TDD for non-trivial changes: failing test first, then implementation.
- Prefer e2e over unit tests. Avoid mocks; talk to a real htree daemon or deterministic in-process equivalent.
- Keep tests deterministic; avoid flaky time assumptions.
- Fix errors you encounter, related or not.
- Keep files small; split modules before they sprawl.
- Nostr subscriptions, peer discovery, and mutable-root watches: prefer open subscriptions over one-shot timed fetches. No response inside one window is not evidence of absence.
- Treat explicit misses differently from timeouts when routing requests.
- Bound per-peer work/memory; heartbeats without bytes do not keep requests alive forever.
- Mounts must be virtual provider surfaces, not materialized normal folders. Use FileProvider, FUSE, WinFsp, SAF, WebDAV, or equivalent virtual adapters over hashtree/provider roots; do not add user-visible materialized-folder fallbacks.
- Record performance experiments in `docs/EXPERIMENTS.md`; omit identifying info (pubkeys, secrets, IPs, private hostnames, raw hashes) unless explicitly asked.
- Never `git pull`/`git rebase` from `htree://self/*`; it is publish-only storage, not an integration upstream.
- After relevant tests/build/lint pass, commit and push to htree (`htree://self/iris-drive`).

## Naming

CLI binary is `idrive`; hashtree CLI is `htree`; do not collide. Say "the iris-drive daemon" for this daemon, distinct from "the hashtree daemon" it wraps.
