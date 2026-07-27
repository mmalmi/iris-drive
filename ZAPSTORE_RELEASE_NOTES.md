# Iris Drive 0.1.30

- Show authorized devices online when connected through routed FIPS sessions.
- Keep an authenticated idle channel for accurate device presence.
- Stop redundant direct-path discovery while a routed device session is
  already healthy.
- Converge simultaneous edits to the same original and conflict-copy names on
  every device.
- Use each signed root's embedded causal timestamp so republished
  announcements cannot make devices choose different conflict winners.
- Prevent duplicate simultaneous dials between linked devices.
- Update to FIPS 0.4.44 and Hashtree FIPS transport 0.4.11.
