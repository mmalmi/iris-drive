# Iris Drive 0.1.34

- Show files that existed before device linking on the newly approved device.
- Sync files created on Web back to native and mobile provider views through
  the durable relay and Blossom path.
- Preserve native and browser device names in the Devices list.
- Complete approval only after the current Drive root and all of its blocks
  are available to the joining device.
- Accept Web-created key epochs while keeping existing native profiles
  compatible.
- Preserve every file and nested directory when a path changes between file
  and folder kinds, with deterministic conflict copies instead of data loss.
- Keep those replacements and conflict copies stable when another linked
  device later saves an unrelated file.
