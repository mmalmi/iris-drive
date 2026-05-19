# AGENTS.md

Notes for AI coding agents working in this repo. Pair with any local
operator instructions outside the repository.

## Project shape

`iris-drive` is a Google-Drive-style file sync app built on top of the local
`htree` daemon (see `~/src/hashtree`). Structure mirrors `~/src/nostr-vpn`:
shared Rust core + per-platform native shells over a UniFFI app-core crate.

Iris Drive does not run its own storage protocol — it consumes hashtree for
content-addressed storage and uses Nostr for identity, peer discovery, and
share invites.

## Shared rules

- TDD when changes are non-trivial: failing test first, then implementation.
- Prefer e2e tests over unit tests. Avoid mocks; talk to a real htree daemon
  or a deterministic in-process equivalent.
- Keep tests deterministic. No flaky time-based assumptions.
- Fix errors you encounter, related or not.
- Keep files small. Split modules before they sprawl.
- For Nostr subscriptions, peer discovery, and mutable-root watches, prefer
  open subscriptions over one-shot timed fetches. A missing response inside
  one window is not evidence the data does not exist.
- Treat explicit misses differently from timeouts when routing requests.
- Bounded per-peer work and memory — heartbeats without bytes do not keep
  requests alive forever.
- Record performance experiments in `docs/EXPERIMENTS.md`, omitting
  identifying information (pubkeys, secrets, IPs, private hostnames, raw
  hashes) unless the user explicitly asks otherwise.
- Never `git pull` or `git rebase` from `htree://self/*` — it is publish-only
  storage, not an integration upstream.
- Commit after relevant tests/build/lint pass, then push to the htree remote
  (`htree://self/iris-drive`).

## Naming

CLI binary is `idrive`. The hashtree CLI is `htree`; do not collide. The
iris-drive daemon is "the iris-drive daemon," distinct from "the hashtree
daemon" which is what we wrap.
