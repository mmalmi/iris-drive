# Snapshot-first sync implementation plan

This plan refines the multi-device sync design after comparing Iris Drive with
Syncthing and Perkeep. The direction is snapshot-first, not log-first:

- A signed drive root is the canonical truth.
- Every signed root is a complete, verifiable snapshot of current drive state.
- Whole-file hashes remain first-class file identity.
- Logs and indexes may accelerate sync or explain history, but current state
  must never require replaying a delta log from genesis.
- FIPS and hashtree transport move and verify content; they do not decide truth.

## Current gaps

The existing code already has the right rough pieces, but they are still v1:

- `DeviceRootRef` carries `root_cid`, `published_at`, and `dck_generation`.
- `merge_drives` resolves per-path winners with latest `published_at`.
- `sync.rs` enumerates both sides and treats different hashes as conflicts.
- `conflict.rs` has a better base/local/remote resolver, but sync does not
  yet persist or feed it base snapshots.
- `.hashtree/prev` and `.hashtree/tombstones` exist as snapshot metadata.

The main issue to fix is not "missing operation logs"; it is missing causal
metadata and durable base state.

## Invariants

1. A signed root snapshot is sufficient to read the drive.
   Losing local indexes, optional operation records, or conflict UI metadata
   must not make the current root unreadable.

2. Whole-file identity is preserved.
   Each file entry must expose size and whole-file hash or an equivalent
   stable content CID. Chunk trees, range reads, and dedupe are implementation
   details below that whole-file identity.

3. Causality is attached to root snapshots.
   Devices should compare root ancestry and per-device sequence observations,
   not wall-clock time, to decide whether one root descends from another or is
   concurrent with it.

4. Wall-clock timestamps are UI metadata and deterministic tie-breakers only.
   They are not conflict-resolution truth.

5. `.hashtree` data is reserved for structural metadata.
   It may contain snapshot metadata, history links, tombstones, conflict
   records, and optional operation hints. User-visible paths must be derived
   from the snapshot, not from a required op replay.

6. Missing content is distinct from unavailable content.
   A FIPS timeout, relay miss, or silent peer means "unknown from this source",
   not "the content does not exist".

## Target root metadata

Add root-level metadata to each device snapshot. This can live either in the
signed drive-root event or inside the root under `.hashtree/root.json`. The
preferred implementation is to put snapshot metadata in `.hashtree/root.json`
so it travels with the root bytes, then publish/sign the resulting root CID.

The metadata must not include the root CID itself if it is embedded in the
root. The signed publish event already binds the root CID to the publisher.

```json
{
  "schema": 1,
  "drive_id": "main",
  "device_id": "<device pubkey>",
  "device_seq": 42,
  "dck_generation": 3,
  "parents": [
    {
      "device_id": "<device pubkey>",
      "device_seq": 41,
      "root_cid": "<previous root cid>"
    }
  ],
  "observed": {
    "<device pubkey>": {
      "device_seq": 18,
      "root_cid": "<latest observed root cid>"
    }
  },
  "created_at": 1779360000
}
```

Field rules:

- `device_seq` is monotonic per device and drive.
- `parents` are the roots this root directly replaces or incorporates.
- `observed` is the compact vector-clock-style view of other authorized
  devices at publish time.
- `created_at` is useful for display, ordering lists, and fallback sorting.
- The owner/device signature is still the authority for authenticity.

For compatibility, old roots without this metadata are treated as legacy roots
with unknown causality and fall back to current timestamp behavior until all
active devices have republished.

## File entry model

Do not make sync depend on delta reconstruction. A file entry should remain
complete enough to compare, fetch, and materialize the current file directly.

Required logical fields per file:

```text
path
size
whole_file_hash
content_cid or chunk_tree_cid
link_type
metadata_version
mtime optional
```

`whole_file_hash` is the conflict and convergence key. `content_cid` or
`chunk_tree_cid` is the retrieval key. They may be the same value if hashtree
represents the file that way, but the sync engine should name the concepts
separately so it never accidentally compares only a chunk root where a
whole-file identity is required.

## `.hashtree` layout

Keep `.hashtree` as one reserved top-level namespace:

```text
.hashtree/root.json
.hashtree/prev
.hashtree/tombstones/<original path>
.hashtree/conflicts/<conflict id>.json
.hashtree/ops/<optional event id>.json
```

Semantics:

- `root.json` describes the snapshot and its causal observations.
- `prev` links to the previous root for history browsing and repair.
- `tombstones` represent deletes in the current snapshot.
- `conflicts` records conflict provenance and resolution state.
- `ops` is optional. It can explain renames or power history UI, but the drive
  must be valid if `ops` is absent.

## Merge algorithm

Replace timestamp LWW with causal comparison.

For each path, collect all candidate writes and tombstones from authorized
device snapshots.

1. Discard snapshots from unauthorized devices or stale DCK generations.
2. Group candidates by path.
3. If candidates have identical whole-file hash and size, converge without a
   conflict even if they came from concurrent roots.
4. If one candidate causally descends from another, keep the descendant.
5. If a tombstone causally descends from a write, delete the path.
6. If a write causally descends from a tombstone, restore the path.
7. If candidates are concurrent and differ, create or update a conflict record.
8. Use wall-clock time and device id only as deterministic fallback ordering
   for display and stable file naming.

Concurrent write/delete is a conflict. The resolver should preserve the file
content as a conflict copy and record that the other side deleted the original
path. Do not silently let delete win by timestamp.

## Local sync state

Add a durable local sync database, but treat it as a rebuildable cache.

Suggested tables:

```text
roots(device_id, device_seq, root_cid, dck_generation, observed_json, seen_at)
path_state(path, root_cid, whole_file_hash, content_cid, size, metadata_json)
base_state(path, base_root_cid, whole_file_hash, content_cid, size)
needs(hash_or_cid, source_hint, priority, first_seen_at, last_attempt_at)
source_availability(hash_or_cid, source_id, state, updated_at)
conflicts(conflict_id, path, local_json, remote_json, state, created_at)
```

Rebuild rule:

- If the DB is missing or corrupt, enumerate the latest signed roots, read
  `.hashtree/root.json`, walk each snapshot, and rebuild current `path_state`.
- Historical base quality may be reduced after rebuild, but current state must
  remain correct.

## Sync application

Move `sync.rs` from two-way whole enumeration toward snapshot application:

1. Learn a remote signed root.
2. Fetch missing root metadata and directory blocks.
3. Compute path-level diff between the old applied root and the new root.
4. For each changed path, build base/local/remote `FileSnapshot` values.
5. Feed them through the existing conflict resolver.
6. Apply non-conflicting changes to the provider.
7. Write conflict files and `.hashtree/conflicts` records for conflicts.
8. Publish a new signed local root only after local provider state has been
   indexed into a complete snapshot.

The current full-enumeration path can remain as a fallback and test harness,
but the production engine should use root anchors and path diffs.

## Hashtree work

Contribute these primitives upstream to `~/src/hashtree` where appropriate:

- Path/item diff: `diff_items(old_root, new_root) -> Vec<ItemChange>`.
- Whole-file identity surfaced consistently in provider entries.
- Range reads verified against file/chunk metadata.
- Snapshot metadata helpers for reading and writing `.hashtree/root.json`.
- Conflict metadata helpers if they are broadly useful beyond Iris Drive.

Existing hash-level tree diff remains useful for replication and repair, but
Iris needs path-level changes for safe user-facing sync.

## FIPS and network fetch

Use hashtree-over-FIPS as a content retrieval layer:

- Request by hash or CID.
- Verify every response against the requested hash.
- Treat silence and timeout as unknown, not as a negative content miss.
- Hedge requests across devices, local cache, Blossom mirrors, and future
  relays.
- Track source quality in `source_availability`.
- Bound in-flight work per peer and globally.

Sync semantics must not depend on which source returned the bytes. The signed
root and verified content hash decide truth.

## Conflict UX model

Conflict files are real files in the snapshot. Conflict records explain them.

Example conflict record:

```json
{
  "schema": 1,
  "path": "report.pdf",
  "visible_conflict_path": "report (conflict from phone).pdf",
  "local": {
    "device_id": "laptop",
    "device_seq": 42,
    "root_cid": "<cid>",
    "whole_file_hash": "<hash>"
  },
  "remote": {
    "device_id": "phone",
    "device_seq": 18,
    "root_cid": "<cid>",
    "whole_file_hash": "<hash>"
  },
  "state": "unresolved",
  "created_at": 1779360000
}
```

Rules:

- Avoid conflict-copy loops by recognizing existing conflict records and
  generated conflict filenames.
- Cap repeated conflicts per original path and surface overflow in status UI.
- Resolving a conflict writes a normal new root snapshot. The conflict record
  may remain as history or be marked resolved.

## Test plan

Prefer e2e-style tests around real provider and hashtree behavior.

Required cases:

- Two devices edit different files while offline, then converge.
- Two devices edit the same file differently, conflict preserved.
- Two devices edit same file to identical bytes, no conflict.
- One device edits while another deletes, conflict preserved.
- Device clock skew does not change conflict outcome.
- Nostr event delivery order does not change conflict outcome.
- Legacy `published_at` roots can still merge during migration.
- Case-only path conflicts are detected on case-insensitive filesystems.
- Unicode normalization conflicts are detected across platforms.
- Symlinks and reserved `.hashtree` paths cannot escape the drive root.
- FIPS silent peer produces unknown, not negative miss.
- Poisoned FIPS response is ignored after hash verification.
- Local sync DB deletion rebuilds current state from signed roots.

## Implementation phases

### Phase A: root metadata

- Add `DriveRootMeta` and root metadata read/write helpers.
- Attach `.hashtree/root.json` during indexing.
- Extend `DeviceRootRef` with `device_seq`, `parents`, and observed roots.
- Keep `published_at` for display and migration fallback.
- Add migration tests for old roots.

### Phase B: causal merge

- Implement root causality comparison.
- Replace timestamp LWW in `merge_drives`.
- Add concurrent write, concurrent delete, and write/delete tests.
- Preserve same-content convergence across concurrent roots.

### Phase C: durable sync cache

- Add the local sync DB.
- Persist base snapshots after successful application.
- Rebuild cache from signed roots when missing.
- Feed base/local/remote snapshots into `conflict.rs`.

### Phase D: conflict ledger

- Add `.hashtree/conflicts` records.
- Add generated conflict filename detection.
- Add status data for unresolved conflicts.
- Add conflict resolution flow that writes a normal new root.

### Phase E: path diff and provider integration

- Add or consume hashtree item diff.
- Replace production full-enumeration sync with root-anchor diff sync.
- Keep full enumeration only as fallback/debug.
- Add provider tests that apply diffs without whole-file materialization unless
  bytes are actually needed.

### Phase F: FIPS-backed retrieval

- Route missing content reads through hashtree network/FIPS sources.
- Add source availability and retry policy.
- Ensure timeout and miss semantics match hashtree-on-FIPS.
- Add poisoned response and silent peer tests.

## Non-goals

- No CRDT file-content merge in v1.
- No required operation-log replay for current state.
- No trust in wall-clock timestamps for correctness.
- No transport-specific sync semantics.
- No hidden user-visible files outside the single `.hashtree` namespace.
