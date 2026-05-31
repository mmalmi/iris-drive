# AGENTS.md

## Project

`iris-drive` is a Drive-style sync app over local `htree` (`~/src/hashtree`). Shape mirrors `~/src/nostr-vpn`: shared Rust core, native shells, UniFFI app-core. It uses htree CAS plus Nostr identity, peers, discovery, and share invites; it has no storage protocol.

## Rules

- TDD non-trivial changes: failing test first.
- Prefer deterministic e2e with real htree or deterministic in-process equivalent; avoid mocks and flaky time.
- Fix errors you hit, related or not.
- Keep files small; split modules before they sprawl.
- Keep Nostr subscriptions, peer discovery, and mutable-root watches open; one quiet window is not absence.
- Treat explicit misses differently from timeouts.
- Bound per-peer work/memory; heartbeats without bytes do not keep requests alive.
- Mounts are virtual provider surfaces only: FileProvider, FUSE, WinFsp, SAF, etc. over htree/provider roots. No user-visible normal-folder fallbacks.
- Record perf experiments in `docs/EXPERIMENTS.md`; omit pubkeys, secrets, IPs, private hosts, raw hashes unless asked.
- Never `git pull`/`git rebase` from `htree://self/*`; it is publish-only.
- After relevant checks pass, commit and push to `htree://self/iris-drive`.
- No need to await Nostr publishes.
- No fallbacks: fix root causes; if impossible, show the error.

## Naming

`idrive` is the CLI; `htree` is the hashtree CLI. Say "iris-drive daemon" for this daemon and "hashtree daemon" for htree.
