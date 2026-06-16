# Platform GUI parity

macOS is the current behavior reference for the native control panel. Linux,
Windows, iOS, and Android should expose the same user-visible sync controls
where the platform allows it, even when their OS file-provider backends differ.
`UI scaffold` means the shell exposes the control and state shape, but still
needs the full mobile app-core/idrive backend hookup before it is production
equivalent to desktop.

| Capability | Linux GTK | macOS SwiftUI | Windows WPF | iOS SwiftUI | Android Compose |
| --- | --- | --- | --- | --- | --- |
| First-run create profile | Yes | Yes | Yes | Local create flow | App-core create flow |
| Restore IrisProfile | Yes | Yes | Yes | Local restore flow | App-core restore flow |
| Link this app install flow | Sign in -> Link this app install | Sign in -> Link this app install | Sign in -> Link this app install | Sign in -> Link this app install + deep link | Sign in -> Link this app install + deep link |
| Log out local profile | Yes | Yes | Yes | App-core logout flow | App-core logout flow |
| Copy IrisProfile/AppKey info | Yes | Yes | Yes | Yes | App-core UI flow |
| Add another app install | Add app install dialog | Add app install sheet | Add app install dialog | Add app install sheet + deep link | Add app install dialog + deep link |
| Start/stop/restart sync daemon | Yes | Yes | Yes | Foreground sync control scaffold | Foreground service start/stop/restart |
| Auto-scan local drive folder | No; mount publishes writes | No | No | No | No |
| Open drive folder | Yes, mounted | FileProvider domain | Cloud Files placeholders | Files app FileProvider domain + open action | SAF DocumentsProvider + open action |
| Open share dialog for selected folder | `iris-drive://share` + `drive.iris.to/share` | `iris-drive://share` + `drive.iris.to/share` | `iris-drive://share` via installer protocol | `iris-drive://share` + `drive.iris.to/share` | SAF folder action -> `iris-drive://share` |
| Copy/view drive.iris.to link | Yes | Yes | Yes | Yes | App-core UI flow |
| Open/save Drive content link | Passed `drive.iris.to/#/nhash...` or `#/npub.../tree/path` URL -> local resolver or app-core import | Universal link -> local resolver or app-core import | Launch URL -> local resolver or app-core import | Universal link -> local resolver or app-core import attempt | App link -> local resolver or app-core import attempt |
| App installs list and auth state | Yes | Yes | Yes | Local UI flow | App-core UI flow |
| App install online/sync status | Yes | Planned | Yes | Local-only scaffold | Local scaffold |
| AppKey revoke control | Yes | Planned | Yes | Local UI flow | App-core UI flow |
| Relay add/reset controls | Yes | Yes | Yes | Local UI flow | App-core UI flow |
| Direct FIPS block sync | Yes | Yes | Yes | Harness daemon peer; app pending | Harness daemon peer; app pending |
| File server list | Yes | Yes | Yes | Read-only list | Read-only list |
| Hashtree config/block/root paths | Yes | Yes | Yes | App-group runtime/config/block paths | App files path only |
| Tray/menu-bar control | Yes | Yes | Yes | N/A | N/A |
| Close to tray/menu-bar | Yes | Yes | Yes | N/A | N/A |
| Native OS file-provider surface | FUSE mount | FileProvider domain | Cloud Files read hydration | FileProvider extension/domain | DocumentsProvider read/write surface |
| Multi-app e2e label | `ubuntu` | `macos` | `windows` | `ios` provider-command peer + simulator smoke | `android` provider-command peer + adb smoke |

## Desktop test target

The local "e2e everything between 3 VMs" command is:

```bash
just e2e-3vms
```

It runs the Rust workspace tests, updates/builds/starts the configured macOS,
Ubuntu, and Windows dev VMs, then runs the native 3-VM sync battery against the
real FileProvider, FUSE, and Cloud Files surfaces. Linux GTK and Windows WPF GUI smokes
also run as first-class desktop UI phases: Linux must expose a visible GTK
window on the VM display or a disposable Xvfb display, and Windows must expose
and navigate the WPF window through UI Automation. VM hostnames stay in
`~/.config/iris-drive/dev-lab.env` or local git remotes, not in tracked files.
The native smoke writes per-hop timing JSONL to `target/e2e-3vms-*-timings.jsonl`.

The minimum parity smoke for native desktop shells is:

1. Create an owner profile on one VM.
2. Link the other VM as a secondary app install from the GUI and copy its AppKey ID.
3. Open Add AppKey in the owner GUI, paste the AppKey ID, and approve it.
4. Confirm both AppKeys tabs show the authorized peer and its FIPS online/sync state.
5. Create, rename, edit, and delete files inside the mounted drive.
6. Confirm authorized peers receive the new roots without using a normal folder scan.
7. Confirm native directory viewers/watchers wake after remote creates and deletes without reopening the folder.
8. Confirm the three native visible directories have matching path/content manifests and no unintended conflict copies.

Block replication now tries direct hashtree-over-FIPS transfer between
authorized Iris Drive instances first. File servers remain configured as a
remote cache path, not the primary direct sync transport.

OS share/context integrations should only route into the app. The stable route
is `iris-drive://share?path=<folder>&name=<optional-name>` or
`https://drive.iris.to/share?...`; native shells select the **Shares** tab and
prefill the create-share form, while app-core remains responsible for the actual
share creation. Android exposes this through the SAF folder settings action for
non-root Iris Drive folders. Optional `recipient_npub`, `recipient_name`, and
`recipient_profile` query fields prefill invite/contact fields only; signed
recipient evidence and roster ops remain the authority for the member's
IrisProfile UUID and AppKeys.
Linux accepts the route through its desktop `x-scheme-handler/iris-drive`
registration and Windows accepts it through the per-user installer protocol
registration plus a running-instance handoff; both native shells classify the
URL with app-core before touching UI fields.

When `drive.iris.to` is served through the local Iris Drive gateway/native
runtime, the share dialog may POST `create_share` and later share-management
actions to `/api/iris-drive/share-action`. That route dispatches Rust core share
actions and returns core `SharedFolderView` projections. Regular HTTPS browser
pages keep using the app handoff URL instead of implementing authority logic in
web code.

If contact search supplies signed recipient evidence, `drive.iris.to` may pass
that opaque evidence JSON to `invite_share_member_from_evidence`; Rust core
resolves the representative npub to an IrisProfile member and AppKeys before
granting access. CLI users can produce the same opaque evidence with
`idrive shares recipient-evidence --display-name <name>`, and native shells can
dispatch `export_share_recipient_evidence` and read
`last_share_recipient_evidence` from app-core state. The exported JSON is not
authority until Rust core validates it during invite.
If contact search only supplies a representative npub/display hint, the web or
native shell may dispatch `record_pending_share_invite`. Rust core records that
as pending share metadata without adding a member, key facet, wrap, or authority
until signed recipient evidence resolves to an IrisProfile UUID.

Share invite pages may render a static preview from the signed invite bundle.
When the local gateway/native endpoint is available, accepting an invite and
adding a shortcut dispatch `accept_share_invite` and `add_share_shortcut`
through Rust core instead of writing browser-local authority state.

CLI, UniFFI app-core, and the local gateway all route share mutations through
`iris_drive_core::dispatch_share_action`. Surface-specific code may parse UI
strings and render JSON/records, but create, invite, accept, role, revoke,
shortcut, and repair state transitions stay in Rust core.
Read-only share state uses the same core projection path through
`iris_drive_core::share_action_state` / `SharedFolderView`, including source
path, entity members, role/write authorization, repair-needed/key-unavailable
state, missing-wrap detail, pending representative-npub invite hints, and
shortcuts.
Native shells render app-core share records rather than recalculating authority:
macOS, iOS, Android, and Windows show source path, entity members, pending
representative-npub invites, roles, repair state, and shortcuts, and route
share create/invite/accept/revoke/role/shortcut/repair controls through Rust
app-core actions.

App-core and CLI status surfaces should expose profile roster actors as
`app_actors`, `authorized_app_key_count`, `online_app_key_count`,
`app_key_npub`, and `is_current_app_key`. FIPS transport diagnostics may keep
their lower-level `*_device*` field names because they describe network
endpoints rather than IrisProfile roster authority.

## Mobile test target

iOS has a buildable SwiftUI shell plus a FileProvider extension registered by
the containing app. The local iOS simulator smoke is:

```bash
just ios-smoke
just ios-gui-smoke
```

Android has a buildable Jetpack Compose shell plus a SAF `DocumentsProvider`
registered as `to.iris.drive.documents`. The local Android adb smoke is:

```bash
just android-smoke
just android-gui-smoke
```

For the full desktop + mobile lab, use:

```bash
just e2e-5devices
```

That runs the iOS simulator and GUI linking smokes, runs the Android GUI
linking and provider smokes on the configured Android host, runs Linux GTK and
Windows WPF GUI smokes on the desktop hosts, then includes both mobile host
labels as daemon peers in the shared multi-app sync harness.
