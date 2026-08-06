# Iris Drive 0.1.33

- Require an explicit confirmation before approval links add a device on iOS,
  Android, Linux, macOS, or Windows.
- Wake mobile device linking immediately after approval.
- Backfill the complete device roster as soon as a joining device becomes
  authorized.
- Persist and acknowledge relay-based approvals across app restarts, then
  remove the exact pending receipt from the owner's device.
- Preserve concurrent approvals and prevent background refreshes from
  overwriting a newly linked device.
- Keep every newly authorized device on the current FIPS peer allowlist when
  several approvals overlap.
- Show files that existed before linking without waiting for an app restart or
  a direct FIPS connection.
- Preserve provider writes that overlap an incoming remote root.
- Ignore malformed or unrelated relay events during the linking handshake.
