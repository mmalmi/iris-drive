//! Menu-bar wrapper around the `idrive daemon`.
//!
//! Owns the macOS `NSApplication` main thread, runs a tao event loop,
//! and shows a tray icon plus a small menu. The daemon itself runs as
//! a child process — `iris-drive-mac` parses its JSON event log and
//! reflects state into the menu. Subprocess separation means: no
//! shared mutexes, the CLI binary stays pure, and the daemon's
//! existing supervisor/restart story still works as-is.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result};
use iris_drive_core::account::Account;
use iris_drive_core::config::AppConfig;
use iris_drive_core::paths::{config_path_in, key_path_in};
use iris_drive_core::{Drive, PRIMARY_DRIVE_ID};
use serde_json::Value;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::TrayIconBuilder;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};

#[derive(Debug)]
enum UserEvent {
    Menu(MenuEvent),
    DaemonLog(Value),
    DaemonExited(Option<i32>),
}

#[allow(clippy::too_many_lines)]
fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = proxy.send_event(UserEvent::Menu(e));
    }));

    let menu = Menu::new();
    let status_item = MenuItem::new("Iris Drive — starting…", false, None);
    let workingdir_item = MenuItem::new("Working dir: (unknown)", false, None);
    let open_drive = MenuItem::new("Open My Drive", true, None);
    let open_config = MenuItem::new("Show Config Folder", true, None);
    let quit = MenuItem::new("Quit Iris Drive", true, None);
    menu.append_items(&[
        &status_item,
        &workingdir_item,
        &PredefinedMenuItem::separator(),
        &open_drive,
        &open_config,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .context("building menu")?;

    let config_dir = iris_drive_core::paths::default_config_dir();
    let mut working_dir: Option<PathBuf> = None;

    // First-launch bootstrap. The CLI exposes `idrive init` /
    // `blossom-servers add` / `import` for power users, but the tray
    // app is the zero-config path: if no keys or config exist yet,
    // generate them, point the primary drive at `~/Iris Drive`, and
    // create that folder. After this returns the daemon child can
    // start with everything it needs already on disk.
    if let Some(dir) = config_dir.as_ref() {
        if let Err(e) = bootstrap_first_launch(dir) {
            eprintln!("first-launch bootstrap failed: {e:#}");
        } else if let Some(wd) = read_primary_working_dir(dir) {
            let () = workingdir_item.set_text(format!("Working dir: {}", wd.display()));
            working_dir = Some(wd);
        }
    }

    // Spawn `idrive daemon` and pump its stdout into the event loop.
    let proxy = event_loop.create_proxy();
    let mut child = spawn_daemon().context("spawning idrive daemon")?;
    let stdout = child.stdout.take().expect("daemon stdout");
    let stderr = child.stderr.take();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            // Try JSON first; non-JSON lines (tracing logs) get ignored.
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                let _ = proxy.send_event(UserEvent::DaemonLog(value));
            }
        }
        let exit = child.wait().ok().and_then(|s| s.code());
        let _ = proxy.send_event(UserEvent::DaemonExited(exit));
    });
    // Drain stderr to keep it from filling its pipe buffer.
    if let Some(stderr) = stderr {
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for _ in reader.lines().map_while(Result::ok) {}
        });
    }

    let mut tray_icon = None;
    let cfg_dir_for_loop = config_dir.clone();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                tray_icon = match TrayIconBuilder::new()
                    .with_menu(Box::new(menu.clone()))
                    .with_tooltip("Iris Drive")
                    .with_icon(make_icon())
                    .build()
                {
                    Ok(t) => Some(t),
                    Err(e) => {
                        eprintln!("failed to build tray icon: {e}");
                        *control_flow = ControlFlow::Exit;
                        return;
                    }
                };

                // Note: tao's tray-icon example does a CFRunLoopWakeUp
                // here to nudge the icon into appearing immediately.
                // Workspace forbids `unsafe_code` so we skip it; the
                // icon still appears on next event-loop iteration. If
                // we ever see startup latency, the right fix is an
                // objc2 wrapper or downgrading the forbid to deny.
            }
            Event::UserEvent(UserEvent::Menu(e)) => {
                if e.id == quit.id() {
                    tray_icon.take();
                    *control_flow = ControlFlow::Exit;
                } else if e.id == open_drive.id() {
                    if let Some(dir) = &working_dir {
                        let _ = Command::new("open").arg(dir).spawn();
                    }
                } else if e.id == open_config.id()
                    && let Some(dir) = &cfg_dir_for_loop
                {
                    let _ = Command::new("open").arg(dir).spawn();
                }
            }
            Event::UserEvent(UserEvent::DaemonLog(value)) => {
                let kind = value
                    .get("event")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                match kind {
                    "subscribed" => {
                        let () = status_item.set_text("Subscribed");
                        if let Some(p) = value.get("working_dir").and_then(|v| v.as_str()) {
                            working_dir = Some(PathBuf::from(p));
                            let () = workingdir_item.set_text(format!("Working dir: {p}"));
                        }
                    }
                    "auto_published" => {
                        if let Some(cid) = value.get("root_cid").and_then(|v| v.as_str()) {
                            let short = &cid[..cid.len().min(12)];
                            let () = status_item.set_text(format!("Published rev {short}…"));
                        } else {
                            let () = status_item.set_text("Published new revision");
                        }
                    }
                    "app_keys" => {
                        let () = status_item.set_text("Received roster update");
                    }
                    "drive_root" => {
                        let outcome = value
                            .get("outcome")
                            .and_then(|v| v.as_str())
                            .unwrap_or("applied");
                        let () = status_item.set_text(format!("Drive root: {outcome}"));
                    }
                    "blossom_downloaded" => {
                        let fetched = value
                            .get("fetched")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0);
                        let () = status_item.set_text(format!("Fetched {fetched} blocks"));
                    }
                    "shutdown" => {
                        let () = status_item.set_text("Daemon stopped");
                    }
                    _ => {}
                }
            }
            Event::UserEvent(UserEvent::DaemonExited(code)) => {
                let () =
                    status_item.set_text(format!("Daemon exited ({code:?}) — restart Iris Drive"));
            }
            _ => {}
        }
    });
}

/// On first launch, generate an account, configure `~/Iris Drive` as
/// the primary drive's working dir, and create that folder. No-op if
/// any of those pieces already exist (so re-runs are safe). The CLI's
/// `idrive init` is the manual equivalent; this is its tray twin.
fn bootstrap_first_launch(config_dir: &Path) -> Result<()> {
    bootstrap_first_launch_with(config_dir, &default_working_dir())
}

/// Inner form that takes an explicit working dir so tests can avoid
/// poking the real `$HOME`.
fn bootstrap_first_launch_with(config_dir: &Path, working_dir: &Path) -> Result<()> {
    let key_path = key_path_in(config_dir);
    let config_path = config_path_in(config_dir);

    let mut config = AppConfig::load_or_default(&config_path).context("loading config")?;

    if !key_path.exists() {
        std::fs::create_dir_all(config_dir).context("creating config dir")?;
        let account =
            Account::create(config_dir, Some("Mac".into())).context("generating account keys")?;
        config.account = Some(account.state.clone());
        if config.drive(PRIMARY_DRIVE_ID).is_none() {
            config.upsert_drive(Drive::primary(&account.state.owner_pubkey));
        }
    }

    let mut dir_to_create = working_dir.to_path_buf();
    if let Some(drive) = config
        .drives
        .iter_mut()
        .find(|d| d.drive_id == PRIMARY_DRIVE_ID)
    {
        if drive.working_dir.is_none() {
            drive.working_dir = Some(working_dir.to_path_buf());
        }
        if let Some(existing) = drive.working_dir.as_ref() {
            dir_to_create.clone_from(existing);
        }
    }

    config.save(&config_path).context("saving config")?;
    std::fs::create_dir_all(&dir_to_create).context("creating working dir")?;
    Ok(())
}

fn read_primary_working_dir(config_dir: &Path) -> Option<PathBuf> {
    let config = AppConfig::load_or_default(config_path_in(config_dir)).ok()?;
    config
        .drive(PRIMARY_DRIVE_ID)
        .and_then(|d| d.working_dir.clone())
}

/// `$HOME/Iris Drive` — matches the visible folder name users expect
/// from a Drive/Dropbox-style app. Falls back to CWD if `$HOME` is
/// unset (basically only headless test rigs).
fn default_working_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map_or_else(
            || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            PathBuf::from,
        )
        .join("Iris Drive")
}

/// Spawn `idrive daemon` from the same directory as this binary if
/// present, otherwise fall back to `idrive` on PATH.
fn spawn_daemon() -> std::io::Result<Child> {
    let same_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("idrive")))
        .filter(|p| p.exists());
    let cmd = same_dir.unwrap_or_else(|| PathBuf::from("idrive"));
    Command::new(cmd)
        .arg("daemon")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

/// Build a simple 22×22 placeholder menu-bar icon. The real product
/// would ship per-state icons (synced / syncing / error); a plain
/// white glyph keeps the dev wrapper visible on the macOS menu bar.
fn make_icon() -> tray_icon::Icon {
    const SIZE: u32 = 22;
    const HALF: u32 = SIZE / 2;
    let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    let radius_sq = (HALF - 1) * (HALF - 1);
    for y in 0..SIZE {
        for x in 0..SIZE {
            // Filled circle: transparent at corners, white inside.
            let dx = x.abs_diff(HALF);
            let dy = y.abs_diff(HALF);
            let r2 = dx * dx + dy * dy;
            if r2 < radius_sq {
                data.extend_from_slice(&[255, 255, 255, 255]);
            } else {
                data.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    tray_icon::Icon::from_rgba(data, SIZE, SIZE).expect("valid icon")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn bootstrap_creates_account_and_working_dir() {
        let cfg = tempdir().unwrap();
        let work_parent = tempdir().unwrap();
        let work = work_parent.path().join("Iris Drive");

        bootstrap_first_launch_with(cfg.path(), &work).unwrap();

        assert!(key_path_in(cfg.path()).exists(), "device key written");
        assert!(work.is_dir(), "working dir created");

        let config = AppConfig::load_or_default(config_path_in(cfg.path())).unwrap();
        assert!(config.account.is_some(), "account stamped");
        let drive = config
            .drive(PRIMARY_DRIVE_ID)
            .expect("primary drive present");
        assert_eq!(drive.working_dir.as_deref(), Some(work.as_path()));
        assert!(
            !config.blossom_servers.is_empty(),
            "default blossom server seeded"
        );
    }

    #[test]
    fn bootstrap_is_idempotent() {
        let cfg = tempdir().unwrap();
        let work_parent = tempdir().unwrap();
        let work = work_parent.path().join("Iris Drive");

        bootstrap_first_launch_with(cfg.path(), &work).unwrap();
        let first = AppConfig::load_or_default(config_path_in(cfg.path())).unwrap();

        bootstrap_first_launch_with(cfg.path(), &work).unwrap();
        let second = AppConfig::load_or_default(config_path_in(cfg.path())).unwrap();

        // Keys + account survive untouched; same npub, same drive id.
        assert_eq!(
            first.account.as_ref().map(|a| &a.device_pubkey),
            second.account.as_ref().map(|a| &a.device_pubkey),
        );
        assert_eq!(first.drives, second.drives);
    }

    #[test]
    fn bootstrap_preserves_existing_working_dir() {
        let cfg = tempdir().unwrap();
        let work_parent = tempdir().unwrap();
        let new_dir = work_parent.path().join("Iris Drive");

        bootstrap_first_launch_with(cfg.path(), &new_dir).unwrap();

        // User picks a different folder (simulated by editing config).
        let custom = work_parent.path().join("Somewhere Else");
        {
            let mut config = AppConfig::load_or_default(config_path_in(cfg.path())).unwrap();
            let drive = config
                .drives
                .iter_mut()
                .find(|d| d.drive_id == PRIMARY_DRIVE_ID)
                .unwrap();
            drive.working_dir = Some(custom.clone());
            config.save(config_path_in(cfg.path())).unwrap();
        }

        // Re-bootstrap shouldn't overwrite the user's choice.
        bootstrap_first_launch_with(cfg.path(), &new_dir).unwrap();
        let config = AppConfig::load_or_default(config_path_in(cfg.path())).unwrap();
        let drive = config.drive(PRIMARY_DRIVE_ID).unwrap();
        assert_eq!(drive.working_dir.as_deref(), Some(custom.as_path()));
        assert!(custom.is_dir());
    }
}
