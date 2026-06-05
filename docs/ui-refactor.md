# Restructure Iris Drive views for a clean, consumer-grade UX

## Context

The desktop control panels (macOS/Windows/Linux) currently expose engine internals directly:
"My Drive" is a grid of technical tiles (Files / **Blocks** / raw Storage / Devices), Devices shows
npubs + CIDs + DCK generations, Backups asks for raw `fs:/`, `lmdb:`, `npub` strings, and there's a
whole **Network** tab of FIPS/relay/blossom diagnostics. A global top bar puts daemon
**Start / Stop / Restart** and snapshot-link tools on every page.

The goal is a Dropbox/Google-Drive/iCloud feel: a friendly status home, clean device & backup lists,
and **all the "nerd" controls tucked into Settings**. Note: the app is a *companion/status* app — actual
files are browsed in Finder/Explorer via the FileProvider — so "My Drive" should be a **status home**,
not a file browser.

Decisions:
- **Scope:** redesign the 3 existing desktop apps now. Android/iOS are empty stubs (`android/README.md`,
  `ios/README.md` = "Not started yet") — they get the **design spec** below to build to later, no code now.
- **My Drive** = friendly status home; daemon controls, block counts, and snapshot tools move to Settings.
- **Tech details:** per-item technical fields are revealed by **expanding a row in place**; account-level
  keys and all Network diagnostics move into **Settings**.

## New information architecture (all platforms)

Sidebar / nav goes from 5 items to **4**:

> **My Drive · Devices · Backups · Settings**   (Network is removed as a top-level item)

Mobile (spec): same 4 destinations as a bottom tab bar.

### My Drive — status home
- **Hero status** (large, friendly, color-coded), replacing the `Running/Stopped` pill and the tile grid:
  - "Up to date" (green) when running and no pending uploads
  - "Syncing N items…" with progress when uploads/backup sync are in flight
  - "Paused"/"Stopped" when the daemon is stopped
  - Drive name as the title.
- **One friendly summary line** below: e.g. "X files · Y used · N devices". Drop **Blocks** entirely
  from the user-facing view (it moves to Settings → Advanced). Keep at most 2–3 clean stats; no raw
  block-byte jargon.
- **Primary action:** "Open in Finder / Open folder".
- **Remove the global action bar.** Start/Stop/Restart and copy/view drive.iris.to link move to Settings.

### Devices — clean roster
- Row = online dot · device icon (this device / laptop / phone) · **name** · subtle secondary line
  (e.g. "This device" / "Online" / "Updated <date>"). No npub/CID on the face of the row.
- **Expand a row** to reveal technical details: full npub (with copy), root CID, key generation,
  published time, Public/Private. (Decision: expandable in place.)
- **"Add app install"** becomes a button -> sheet/popover with the AppKey + label fields + Approve
  (only when the current AppKey can admin the profile). No always-visible inline form.
- The **profile keys** block (Profile ID / Current AppKey / State) leaves this view -> Settings -> Account.

### Backups — clean list
- Row = kind icon · friendly **name** · status line ("Synced · up to date" / "Syncing 40%" / "Pending").
- **Expand a row** to reveal the raw target string + progress counts.
- **"Add backup"** becomes a button → sheet (raw `https://…/npub…/fs:/…/lmdb:/…` entry lives there with
  guidance, instead of on the main page). Keep a "Sync now" action.

### Settings — sectioned (the home for everything technical)
Grouped sections (native grouped `Form`/list per platform):
1. **General** — Menu bar on close (macOS) and any existing app toggles.
2. **Account** — IrisProfile ID, current AppKey (copy button), authorization state. *(moved from Devices)*
3. **Network** — the entire former Network tab: Relays editor, Blossom servers, FIPS diagnostics.
4. **Sync & Advanced** — Start / Stop / Restart daemon; copy/view drive.iris.to link; Blocks & raw storage.
5. **About** — version / drive name.

## Platform implementation

Each platform is native and changes the same way: drop the `network` nav entry, retarget its content into
a Settings section, restyle the four views per the spec, and add an expandable-row + settings-section
primitive.

**macOS — SwiftUI** (`macos/Sources/IrisDriveControlPanel.swift`)
- `IrisDrivePanelTab` (lines 4-42): remove `.network`; keep `.drive/.peers/.backups/.settings`.
- `selectedContent` switch (265-279) and `sidebar` (200-217): drop the Network case/row.
- Replace `overview` (281-291) with the status-home view (new `StatusHero` instead of the `StatTile` grid).
- Remove the global `actions` bar (235-263); fold its buttons into Settings.
- `peers` (293-354): drop `accountKeys`; make `PeerRow` (678-759) collapse to name+status with a
  disclosure (`DisclosureGroup` or expand state) for the technical metadata; turn `approveDeviceForm`
  (316-338) into a sheet behind an "Add device" button.
- `backups` (379-412): make `BackupTargetRow` (761-786) expandable; move the add form into a sheet.
- Rebuild `settings` (356-368) as a grouped `Form` with the 5 sections; move `network` content
  (370-377: `FipsDiagnostics`, `EndpointGroup`, `relayEditor`) and the account keys/daemon/snapshot
  controls into it. Reuse `relayEditor` (414-443) as-is inside the Network section.

**Windows — WPF/XAML** (`windows/MainWindow.xaml`, `windows/MainWindow.xaml.cs`, `windows/App.xaml`)
- Remove `NavNetworkButton` (sidebar ~129-162) and the `NetworkPage` panel (~336); update `SelectPage`
  (`MainWindow.xaml.cs` ~835-858) to drop the Network case.
- Rework `DrivePage` (~237) into the status home; remove the top action bar.
- Make device/backup rows expandable (Expander or a details toggle); move the add forms into dialogs.
- Move Network content + account keys + daemon/snapshot controls into `SettingsPage` (~355) as grouped
  `PanelBorder` sections. Reuse existing styles in `App.xaml` (`PanelBorder`, `NavButton`, `PrimaryButton`).

**Linux — GTK4/libadwaita** (`linux/src/ui.rs`, `linux/src/widgets.rs`)
- `nav_items` (257-263): remove the `"network"` entry; remove its stack page.
- Rebuild the `dashboard` page as the status home; drop the top action row.
- Use `adw::ExpanderRow` for device/backup detail; move add forms into dialogs.
- Fold `network_page` content + account keys + daemon/snapshot controls into `settings_page`, ideally as
  `adw::PreferencesGroup` sections. Reuse helpers in `widgets.rs` (`metric_tile`, `section_title`, etc.).

**Android / iOS — spec only.** No code. This document is the structure they build to: bottom tab bar with
the same 4 destinations, friendly status home, expandable rows, Settings holding Account + Network +
Advanced.

## New shared primitives to add per platform
- **StatusHero** — big icon + status text + drive name (My Drive).
- **ExpandableRow / DisclosureRow** — clean face + revealed technical detail (Devices, Backups).
- **SettingsSection** — grouped header + rows (native grouped Form / PanelBorder / PreferencesGroup).

## Verification
- **macOS:** `swift build` in `macos/`, launch the app, screenshot each of the 4 views + the new Settings
  sections (app-window screenshots, not fullscreen). Confirm: no Network tab; "Open in Finder" works;
  expanding a device/backup row reveals technical detail; Start/Stop/Restart and snapshot tools live in
  Settings and still function; relay add/edit/remove still works inside Settings → Network.
- **Windows:** build the WPF app on the configured Windows VM if needed, verify the same checklist with screenshots.
- **Linux:** `cargo build` in `linux/` (offload to the configured Linux VM if the mini is busy), run and screenshot.
- Commit to `master` and push to htree after each platform builds and looks right.
