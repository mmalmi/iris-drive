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
    /// **Create** flow: generate a fresh owner key + a fresh device key
    /// on this machine. Single-device default; this install has owner
    /// signing authority and the `AppKeys` roster lists this one device.
    Init {
        /// Don't error if config already exists; print the existing state.
        #[arg(long)]
        force: bool,
        /// Human-readable device label (e.g. "Mac mini").
        #[arg(long)]
        label: Option<String>,
        /// Optional username/display name for the owner profile.
        #[arg(long)]
        username: Option<String>,
        /// Optional local profile photo path.
        #[arg(long)]
        profile_photo: Option<String>,
    },
    /// **Restore** flow: import an existing owner `nsec` onto this
    /// device. A fresh device key is generated; this install has owner
    /// signing authority.
    Restore {
        /// Owner secret key as nsec1... or 64-char hex.
        nsec: String,
        /// Replace an existing local setup.
        #[arg(long)]
        force: bool,
        /// Human-readable device label.
        #[arg(long)]
        label: Option<String>,
    },
    /// **Link** flow: turn this install into a secondary device under an
    /// existing owner. Generates a fresh device key; does NOT receive
    /// the owner key. The device waits in `awaiting_approval` until the
    /// owner approves it from an owner-capable device.
    Link {
        /// Owner pubkey as npub1... or 64-char hex.
        owner: String,
        /// Replace an existing local setup.
        #[arg(long)]
        force: bool,
        /// Human-readable device label.
        #[arg(long)]
        label: Option<String>,
    },
    /// Log out this local install and remove local account key material.
    Logout,
    /// Approve a pending device by adding it to the `AppKeys` roster.
    /// Only usable on admin devices.
    Approve {
        /// Device pubkey to authorize (npub1... or 64-char hex).
        device: String,
        /// Optional device label to record alongside.
        #[arg(long)]
        label: Option<String>,
    },
    /// Revoke an authorized device and rotate the drive content key.
    Revoke {
        /// Device pubkey to revoke (npub1... or 64-char hex).
        device: String,
    },
    /// Print the current `AppKeys` roster as JSON.
    Roster,
    /// Rotate the drive content key (DCK) without changing the roster.
    /// Useful for periodic key freshness. Admin-only.
    RotateDck,
    /// Print daemon and sync status as JSON.
    Status,
    /// Print compact GUI summary stats as JSON.
    Stats,
    /// Manage linked devices and device-link requests.
    #[command(subcommand)]
    Devices(DevicesCmd),
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
    /// Show the local identity (owner + device pubkeys + auth state).
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
    /// List the merged view of the primary drive — files across every
    /// authorized device's tree with LWW resolution applied. On a
    /// single-device install this is just that device's tree.
    List {
        /// Walk back N revisions on this device's tree before merging
        /// (0 = current = default, 1 = previous, ...). History comes
        /// from the `.hashtree/prev` chain stored in each directory's `TreeNode`.
        #[arg(long, default_value_t = 0)]
        at: usize,
    },
    /// Hidden native-provider bridge used by FileProvider/FUSE adapters.
    #[command(hide = true, subcommand)]
    Provider(ProviderCmd),
    /// Walk this device's `.hashtree/prev` revision chain and print each root
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
    /// Publish current state (`AppKeys` + this device's drive root) to
    /// all configured relays. Skips `AppKeys` on linked devices that
    /// lack owner-signing authority.
    Publish {
        /// Override config relays with these URLs.
        #[arg(long)]
        relay: Vec<String>,
        /// Per-relay connect timeout (seconds).
        #[arg(long, default_value_t = 10)]
        timeout: u64,
    },
    /// Pull latest `AppKeys` + drive-root events from relays and apply
    /// them locally. After this, `idrive list` reflects every
    /// authorized device's contribution.
    Sync {
        /// Override config relays with these URLs.
        #[arg(long)]
        relay: Vec<String>,
        /// Seconds to wait for relay responses.
        #[arg(long, default_value_t = 10)]
        timeout: u64,
    },
    /// Run a long-running subscriber + publisher. Maintains open
    /// subscriptions for `AppKeys` + drive-root events, applies each
    /// event in real time, serves the local gateway, and keeps any active
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
pub(crate) enum DevicesCmd {
    /// Print an invite URL for this owner-capable device.
    Invite,
    /// Reset this admin device's invite URL by rotating its invite secret.
    #[command(name = "reset-invite")]
    ResetInvite,
    /// Request linking this device using an owner/admin invite URL or owner pubkey.
    #[command(alias = "ask", alias = "connect", alias = "link")]
    Request {
        /// Invite URL from an owner-capable device, or an owner npub/hex.
        owner_or_invite: String,
        /// Admin device pubkey for manual pairing when no invite URL is available.
        #[arg(long, alias = "admin")]
        admin_device: Option<String>,
        /// Human-readable device label.
        #[arg(long)]
        label: Option<String>,
    },
    /// Print inbound and outbound pending device-link requests.
    Requests,
    /// Approve a pending device link request.
    Approve {
        /// Device pubkey or device-link approval URL.
        request: String,
        /// Optional device label to record alongside.
        #[arg(long)]
        label: Option<String>,
    },
    /// Print the current authorized-device roster.
    List,
    /// Revoke an authorized device and rotate the drive content key.
    Revoke {
        /// Device pubkey to revoke (npub1... or 64-char hex).
        device: String,
    },
    /// Promote an authorized device to admin.
    #[command(name = "appoint-admin", alias = "promote-admin")]
    AppointAdmin {
        /// Device pubkey to promote (npub1... or 64-char hex).
        device: String,
    },
    /// Demote an admin device to a normal member.
    #[command(name = "demote-admin")]
    DemoteAdmin {
        /// Device pubkey to demote (npub1... or 64-char hex).
        device: String,
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
}

#[derive(Debug, Subcommand)]
pub(crate) enum EventCmd {
    /// Owner-signed `AppKeys` roster event (kind 30078).
    /// Requires owner-signing authority on this install.
    AppKeys,
    /// Device-signed drive-root event (kind 30078) for the primary
    /// drive. Requires a previous `idrive import` so there's a CID
    /// to publish.
    DriveRoot,
}
