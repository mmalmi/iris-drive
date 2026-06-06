#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Debug, Parser)]
#[command(name = "idrive", version, about = "Iris Drive CLI / daemon")]
pub(crate) struct Cli {
    /// Override the config dir (default: OS config dir / iris-drive).
    #[arg(long, env = "IRIS_DRIVE_CONFIG_DIR", global = true)]
    pub(crate) config_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Print the idrive CLI version.
    Version {
        /// Print version metadata as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Install this idrive binary to a user-selectable path.
    #[command(name = "install-cli")]
    InstallCli {
        /// Destination path for the idrive executable.
        #[arg(long)]
        path: Option<PathBuf>,
        /// Replace an existing file.
        #[arg(long)]
        force: bool,
    },
    /// Remove an idrive binary previously installed with `install-cli`.
    #[command(name = "uninstall-cli")]
    UninstallCli {
        /// Installed executable path to remove.
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Check, download, or install a verified hashtree-published idrive update.
    Update(UpdateArgs),
    /// **Create** flow: generate a fresh `IrisProfile`, recovery phrase, and
    /// per-install `AppKey` on this machine. Single-install default; this
    /// `AppKey` starts with profile admin authority.
    Init {
        /// Don't error if config already exists; print the existing state.
        #[arg(long)]
        force: bool,
        /// Human-readable app install label (e.g. "Mac mini").
        #[arg(long)]
        label: Option<String>,
        /// Optional username/display name for the owner profile.
        #[arg(long)]
        username: Option<String>,
        /// Optional local profile photo path.
        #[arg(long)]
        profile_photo: Option<String>,
    },
    /// **Restore** flow: use a 12-word recovery phrase or recovery `nsec` to
    /// recover an `IrisProfile` onto this install. A fresh local `AppKey` is
    /// generated; the recovery secret is never cloned as the `AppKey`.
    Restore {
        /// 12-word `IrisProfile` recovery phrase, nsec1..., or 64-char hex secret.
        recovery_secret: String,
        /// Replace an existing local setup.
        #[arg(long)]
        force: bool,
        /// Human-readable app install label.
        #[arg(long)]
        label: Option<String>,
    },
    /// Use a recovery phrase to admit this install's fresh `AppKey` into
    /// the synced `IrisProfile` roster. If no phrase is provided, the
    /// saved local recovery phrase is used.
    #[command(name = "recover-app-key")]
    RecoverAppKey {
        /// Optional 12-word recovery phrase. Omit to use the saved phrase.
        recovery_phrase: Option<String>,
        /// Human-readable `AppKey` label.
        #[arg(long)]
        label: Option<String>,
    },
    /// **Link** flow: turn this install into a secondary `AppKey` under an
    /// existing `IrisProfile` using an admin invite URL. Generates a fresh
    /// local `AppKey`; approval arrives through signed `IrisProfile` roster
    /// ops.
    Link {
        /// Invite URL from an admin `AppKey`.
        invite: String,
        /// Replace an existing local setup.
        #[arg(long)]
        force: bool,
        /// Human-readable app install label.
        #[arg(long)]
        label: Option<String>,
    },
    /// Log out this local install and remove local profile key material.
    Logout,
    /// Approve a pending install by adding its `AppKey` to the `IrisProfile`
    /// roster. Only usable by profile admins.
    Approve {
        /// `AppKey` pubkey to authorize (npub1... or 64-char hex).
        app_key: String,
        /// Optional app install label to record alongside.
        #[arg(long)]
        label: Option<String>,
    },
    /// Revoke an authorized `AppKey` and rotate the drive content key.
    Revoke {
        /// `AppKey` pubkey to revoke (npub1... or 64-char hex).
        app_key: String,
    },
    /// Print the current `IrisProfile`/`AppKeys` roster projection as JSON.
    Roster,
    /// Rotate the drive content key (DCK) without changing the roster.
    /// Useful for periodic key freshness. Admin-only.
    RotateDck,
    /// Print daemon and sync status as JSON.
    Status,
    /// Print compact GUI summary stats as JSON.
    Stats,
    /// List shared folders and add shortcuts into My Drive.
    Shares {
        #[command(subcommand)]
        command: Option<SharesCmd>,
    },
    /// Classify `AppKey`-link, invite, and `AppKey` input using app-core parsing.
    #[command(name = "link-input")]
    LinkInput {
        #[command(subcommand)]
        command: LinkInputCmd,
    },
    /// Manage linked `AppKeys` and `AppKey`-link requests.
    #[command(name = "app-keys", alias = "apps", alias = "installs", subcommand)]
    AppKeys(AppKeysCmd),
    /// View or toggle the local `nhash.iris.localhost` resolver service.
    NhashResolver {
        #[command(subcommand)]
        command: Option<NhashResolverCmd>,
    },
    /// Inspect or resolve durable conflict ledger records.
    #[command(subcommand)]
    Conflicts(ConflictsCmd),
    /// List configured drives.
    Drives,
    /// Show the local `IrisProfile` and current `AppKey`.
    Whoami,
    /// Index a local directory into an in-memory hashtree and print the
    /// root CID + summary. Useful for hands-on sanity checks against the
    /// indexer.
    Index {
        /// Directory to index.
        dir: PathBuf,
    },
    /// Index a local directory into the persistent on-disk store and
    /// stamp the resulting root CID onto the primary drive. Survives
    /// across daemon restarts (blocks live under <config-dir>/blocks/).
    Import {
        /// Source directory to import once.
        dir: PathBuf,
    },
    /// List the merged view of My Drive — files across every authorized
    /// `AppKey`'s tree with LWW resolution applied. On a single-`AppKey` install
    /// this is just that `AppKey`'s tree.
    List {
        /// Walk back N revisions on this `AppKey`'s tree before merging
        /// (0 = current = default, 1 = previous, ...). History comes
        /// from the `.hashtree/prev` chain stored in each directory's `TreeNode`.
        #[arg(long, default_value_t = 0)]
        at: usize,
    },
    /// Hidden native-provider bridge used by FileProvider/FUSE adapters.
    #[command(hide = true, subcommand)]
    Provider(ProviderCmd),
    /// Walk this `AppKey`'s `.hashtree/prev` revision chain and print each root
    /// CID + top-level entry count, newest-first. Blocks GC'd from
    /// the local store terminate the walk silently.
    History {
        /// Maximum number of revisions to walk back. Defaults to 50.
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Build + print Nostr events ready to broadcast to relays.
    #[command(subcommand)]
    Event(EventCmd),
    /// List or modify configured Nostr relays.
    Relays {
        #[command(subcommand)]
        command: Option<RelaysCmd>,
    },
    /// List or modify configured Blossom HTTP blob servers used for
    /// block replication.
    #[command(subcommand)]
    BlossomServers(BlossomServersCmd),
    /// List, add, remove, or sync encrypted backup targets.
    #[command(subcommand)]
    Backups(BackupsCmd),
    /// Publish signed `IrisProfile`/share roster ops and this install's drive
    /// roots to all configured relays.
    Publish {
        /// Override config relays with these URLs.
        #[arg(long)]
        relay: Vec<String>,
        /// Per-relay connect timeout (seconds).
        #[arg(long, default_value_t = 10)]
        timeout: u64,
    },
    /// Pull signed `IrisProfile`/share roster ops and drive-root events from
    /// relays, then apply them locally.
    Sync {
        /// Override config relays with these URLs.
        #[arg(long)]
        relay: Vec<String>,
        /// Seconds to wait for relay responses.
        #[arg(long, default_value_t = 10)]
        timeout: u64,
    },
    /// Run a long-running subscriber + publisher. Maintains open
    /// subscriptions for `IrisProfile` roster ops and drive-root events, applies
    /// each event in real time, serves the local gateway, and keeps any active
    /// virtual mount/provider refreshed. Stops on Ctrl+C.
    Daemon {
        /// Override config relays with these URLs.
        #[arg(long)]
        relay: Vec<String>,
        /// Deprecated compatibility flag. Provider changes are event-driven.
        #[arg(long, default_value_t = 60)]
        watch_interval: u64,
        /// Reserved for virtual-provider write coalescing.
        #[arg(long, default_value_t = 500)]
        watch_debounce_ms: u64,
        /// Start the loopback browser gateway on this port.
        #[arg(long, default_value_t = DEFAULT_GATEWAY_PORT)]
        gateway_port: u16,
        /// Disable the loopback browser gateway.
        #[arg(long)]
        no_gateway: bool,
        /// Mount My Drive with hashtree FUSE instead of watching a normal folder.
        /// Currently supported on Linux.
        #[arg(long)]
        mount: bool,
        /// Mountpoint for --mount. Defaults to the configured/default drive path.
        #[arg(long)]
        mountpoint: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum SharesCmd {
    /// Create a cryptographic share root for a folder.
    Create {
        /// Folder path inside My Drive to share.
        source_path: String,
        /// Display name for recipients. Defaults to the last path segment.
        #[arg(long)]
        name: Option<String>,
    },
    /// List shared folders visible to this app install.
    List,
    /// List members for one share.
    Members {
        /// Share UUID.
        share_id: String,
    },
    /// Invite an `IrisProfile` member using profile evidence or one direct `AppKey`.
    Invite {
        /// Share UUID.
        share_id: String,
        /// Recipient `IrisProfile` UUID for direct `AppKey` invite.
        #[arg(long)]
        profile: Option<String>,
        /// Recipient `AppKey` pubkey (npub1... or 64-char hex) for direct invite.
        #[arg(long = "app-key")]
        app_key: Option<String>,
        /// JSON file with signed `IrisProfile` roster ops and facet acceptances.
        #[arg(long = "recipient-evidence")]
        recipient_evidence: Option<PathBuf>,
        /// Recipient role: reader, editor, or admin.
        #[arg(long, default_value = "reader")]
        role: String,
        /// Display/contact npub hint.
        #[arg(long)]
        npub: Option<String>,
        /// Display name for this member.
        #[arg(long)]
        display_name: Option<String>,
        /// Label for the recipient app install.
        #[arg(long)]
        label: Option<String>,
    },
    /// Accept/import a share invite bundle.
    Accept {
        /// Invite URL or base64url bundle.
        invite: String,
    },
    /// Revoke one `IrisProfile` member from a share and rotate the share key.
    Revoke {
        /// Share UUID.
        share_id: String,
        /// `IrisProfile` UUID to revoke.
        profile_id: String,
        /// Optional reason recorded in `AppKey` tombstones.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Change one `IrisProfile` member's share role.
    Role {
        /// Share UUID.
        share_id: String,
        /// `IrisProfile` UUID to update.
        profile_id: String,
        /// New role: reader, editor, or admin.
        role: String,
    },
    /// Add a shortcut for a share into My Drive.
    Shortcut {
        /// Share UUID.
        share_id: String,
        /// Shortcut path in My Drive. Defaults to a unique root-level name.
        #[arg(long)]
        path: Option<String>,
        /// Parent directory for the default shortcut path.
        #[arg(long)]
        parent: Option<String>,
        /// Path inside the share to open. Empty means the share root.
        #[arg(long, default_value = "")]
        target_path: String,
    },
    /// Repair missing key wraps for a share's current key epoch.
    #[command(name = "repair-wraps")]
    RepairWraps {
        /// Share UUID.
        share_id: String,
    },
}

#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct UpdateArgs {
    /// Only check whether an update is available.
    #[arg(long)]
    pub(crate) check: bool,
    /// Download the selected artifact without installing it.
    #[arg(long)]
    pub(crate) download_only: bool,
    /// Directory for --download-only artifacts.
    #[arg(long)]
    pub(crate) download_dir: Option<PathBuf>,
    /// Emit machine-readable JSON.
    #[arg(long)]
    pub(crate) json: bool,
    /// Destination binary to update (defaults to the currently running executable).
    #[arg(long)]
    pub(crate) path: Option<PathBuf>,
    /// Install even when the latest release is not newer than this binary.
    #[arg(long)]
    pub(crate) force: bool,
    /// Override the hashtree release reference.
    #[arg(long, env = "IRIS_DRIVE_UPDATE_HTREE_REF")]
    pub(crate) reference: Option<String>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AppKeysCmd {
    /// Print an invite URL for this admin `AppKey`.
    Invite,
    /// Reset this admin `AppKey`'s invite URL by rotating its invite secret.
    #[command(name = "reset-invite")]
    ResetInvite,
    /// Request linking this `AppKey` using an admin invite URL or manual profile target.
    #[command(alias = "ask", alias = "connect", alias = "link")]
    Request {
        /// Invite URL from an admin `AppKey`, or `IrisProfile` UUID for manual pairing.
        invite_or_profile: String,
        /// Admin `AppKey` pubkey for manual pairing when no invite URL is available.
        #[arg(long = "admin-app-key", alias = "admin-device", alias = "admin")]
        admin_app_key: Option<String>,
        /// Human-readable app install label.
        #[arg(long)]
        label: Option<String>,
    },
    /// Print inbound and outbound pending `AppKey`-link requests.
    Requests,
    /// Approve a pending `AppKey`-link request.
    Approve {
        /// `AppKey` pubkey or `AppKey`-link approval URL.
        request: String,
        /// Optional app install label to record alongside.
        #[arg(long)]
        label: Option<String>,
    },
    /// Reject a pending `AppKey`-link request without authorizing it.
    Reject {
        /// `AppKey` pubkey or `AppKey`-link approval URL.
        request: String,
    },
    /// Print the current authorized `AppKey` roster.
    List,
    /// Repair missing key wraps for the current key epoch.
    #[command(name = "repair-wraps")]
    RepairWraps,
    /// Revoke an authorized `AppKey` and rotate the drive content key.
    Revoke {
        /// `AppKey` pubkey to revoke (npub1... or 64-char hex).
        app_key: String,
    },
    /// Promote an authorized `AppKey` to admin.
    #[command(name = "appoint-admin", alias = "promote-admin")]
    AppointAdmin {
        /// `AppKey` pubkey to promote (npub1... or 64-char hex).
        app_key: String,
    },
    /// Demote an admin `AppKey` to a normal member.
    #[command(name = "demote-admin")]
    DemoteAdmin {
        /// `AppKey` pubkey to demote (npub1... or 64-char hex).
        app_key: String,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum LinkInputCmd {
    /// Print the app-core link input classification as JSON.
    Classify {
        /// `AppKey`-link, invite, `AppKey` npub, or `AppKey` hex input to classify.
        input: String,
    },
    /// Print the app-core link input validation as JSON.
    Validate {
        /// `AppKey`-link, invite, `AppKey` npub, or `AppKey` hex input to validate.
        input: String,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConflictsCmd {
    /// Mark a conflict record resolved after the files have been handled.
    Resolve {
        /// Conflict id from `idrive status`.
        conflict_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum NhashResolverCmd {
    /// Print resolver settings as JSON.
    Status,
    /// Enable the local resolver service.
    Enable,
    /// Disable the local resolver service.
    Disable,
}

#[derive(Debug, Subcommand)]
pub(crate) enum BlossomServersCmd {
    /// Print current Blossom servers as JSON.
    List,
    /// Append a server URL to the configured list.
    Add { url: String },
    /// Remove a server URL from the configured list.
    Remove { url: String },
}

#[derive(Debug, Subcommand)]
pub(crate) enum BackupsCmd {
    /// Print configured encrypted backup targets as JSON.
    List,
    /// Add or update a Blossom URL, FIPS npub, filesystem, or LMDB backup target.
    Add {
        target: String,
        #[arg(long)]
        label: Option<String>,
    },
    /// Remove a backup target.
    Remove { target: String },
    /// Push the current encrypted root to usable backup targets.
    Sync {
        /// Restrict sync to one configured target.
        #[arg(long)]
        target: Option<String>,
    },
    /// Sample backup targets and record storage, latency, and bandwidth status.
    Check {
        /// Restrict check to one configured target.
        #[arg(long)]
        target: Option<String>,
        /// Number of live blocks to sample per target.
        #[arg(long, default_value_t = 16)]
        sample_size: usize,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum RelaysCmd {
    /// Print current relay URLs as JSON.
    List,
    /// Append a relay URL to the configured list.
    Add { url: String },
    /// Replace an existing relay URL.
    Update { old_url: String, new_url: String },
    /// Remove a relay URL from the configured list.
    Remove { url: String },
    /// Restore the default relay list.
    Reset,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ProviderCmd {
    /// Print the virtual merged drive tree as JSON.
    List,
    /// Export one virtual file into a provider-owned temporary file.
    Read {
        /// Virtual path inside the drive.
        path: String,
        /// Output file path.
        output: PathBuf,
    },
    /// Export the current virtual root into a provider-owned private cache.
    #[command(name = "hydrate-cache", hide = true)]
    HydrateCache {
        /// Cache directory to update.
        dir: PathBuf,
    },
    /// Create or replace one virtual file from a provider-owned temporary file.
    Write {
        /// Virtual path inside the drive.
        path: String,
        /// Source file path.
        source: PathBuf,
    },
    /// Create a virtual directory.
    Mkdir {
        /// Virtual path inside the drive.
        path: String,
    },
    /// Delete a virtual file or directory.
    Delete {
        /// Virtual path inside the drive.
        path: String,
    },
    /// Rename or move a virtual item.
    Rename {
        /// Existing virtual path inside the drive.
        old_path: String,
        /// New virtual path inside the drive.
        new_path: String,
    },
    /// Resolve a native provider parent/name into a normalized collision-free virtual path.
    #[command(name = "resolve-path", hide = true)]
    ResolvePath {
        /// Parent virtual path, empty for the root.
        parent_path: String,
        /// Native display name to normalize.
        display_name: String,
        /// Existing virtual path to ignore when resolving rename collisions.
        excluding_path: Option<String>,
    },
    /// Validate a native provider document id path and return its canonical metadata.
    #[command(name = "normalize-path", hide = true)]
    NormalizePath {
        /// Virtual path carried inside a native provider document id.
        path: String,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum EventCmd {
    /// AppKey-signed drive-root event (kind 30078) for the primary
    /// drive. Requires a previous `idrive import` so there's a CID
    /// to publish.
    DriveRoot,
}
