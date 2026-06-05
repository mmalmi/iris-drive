//! Menu-bar wrapper around the `idrive daemon`.
//!
//! Owns the macOS `NSApplication` main thread, runs a tao event loop,
//! and shows a tray icon plus a small menu. The daemon itself runs as
//! a child process — `iris-drive-mac` parses its JSON event log and
//! reflects state into the menu. Subprocess separation means: no
//! shared mutexes, the CLI binary stays pure, and the daemon's
//! existing supervisor/restart story still works as-is.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result};
use iris_drive_core::config::AppConfig;
use iris_drive_core::paths::{config_path_in, key_path_in};
use iris_drive_core::profile::Profile;
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
    let mount_item = MenuItem::new("Drive mount: native provider", false, None);
    let open_drive = MenuItem::new("Open My Drive", false, None);
    let open_config = MenuItem::new("Show Config Folder", true, None);
    let quit = MenuItem::new("Quit Iris Drive", true, None);
    menu.append_items(&[
        &status_item,
        &mount_item,
        &PredefinedMenuItem::separator(),
        &open_drive,
        &open_config,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .context("building menu")?;

    let config_dir = iris_drive_core::paths::default_config_dir();
    // First-launch bootstrap creates account/config only. The visible drive
    // surface is provided by native virtual providers, not by a backing folder.
    if let Some(dir) = config_dir.as_ref()
        && let Err(e) = bootstrap_first_launch(dir)
    {
        eprintln!("first-launch bootstrap failed: {e:#}");
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

/// On first launch, generate an account and primary drive config. No-op if
/// those pieces already exist.
fn bootstrap_first_launch(config_dir: &Path) -> Result<()> {
    bootstrap_first_launch_with(config_dir)
}

fn bootstrap_first_launch_with(config_dir: &Path) -> Result<()> {
    let key_path = key_path_in(config_dir);
    let config_path = config_path_in(config_dir);

    let mut config = AppConfig::load_or_default(&config_path).context("loading config")?;

    if !key_path.exists() {
        std::fs::create_dir_all(config_dir).context("creating config dir")?;
        let account =
            Profile::create(config_dir, Some("Mac".into())).context("generating profile keys")?;
        config.profile = Some(account.state.clone());
        if config.drive(PRIMARY_DRIVE_ID).is_none() {
            config.upsert_drive(Drive::primary(account.state.root_scope_id()));
        }
    }

    config.save(&config_path).context("saving config")?;
    Ok(())
}

/// Spawn `idrive daemon` from the same directory as this binary if
/// present, otherwise use `idrive` on PATH.
fn spawn_daemon() -> std::io::Result<Child> {
    let same_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("idrive")))
        .filter(|p| p.exists());
    let cmd = same_dir.unwrap_or_else(|| "idrive".into());
    Command::new(cmd)
        .arg("daemon")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

/// Build a 22×22 template version of the Iris Drive platter/reader glyph.
fn make_icon() -> tray_icon::Icon {
    const SIZE: u16 = 22;
    const CENTER: f32 = 11.0;
    const RING_RADIUS: f32 = 6.4;
    const STROKE_WIDTH: f32 = 2.4;
    const PUPIL_RADIUS: f32 = 1.6;
    const READER_START: (f32, f32) = (4.1, 17.9);
    const READER_END: (f32, f32) = (8.8, 13.2);

    let mut data = Vec::with_capacity(usize::from(SIZE) * usize::from(SIZE) * 4);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let px = f32::from(x) + 0.5;
            let py = f32::from(y) + 0.5;
            let dx = px - CENTER;
            let dy = py - CENTER;
            let distance_from_center = (dx * dx + dy * dy).sqrt();
            let ring_alpha = stroke_alpha((distance_from_center - RING_RADIUS).abs(), STROKE_WIDTH);
            let pupil_alpha = fill_alpha(distance_from_center, PUPIL_RADIUS);
            let reader_alpha = stroke_alpha(
                distance_to_segment((px, py), READER_START, READER_END),
                STROKE_WIDTH,
            );
            let alpha = ring_alpha.max(pupil_alpha).max(reader_alpha);
            data.extend_from_slice(&[255, 255, 255, alpha]);
        }
    }
    tray_icon::Icon::from_rgba(data, u32::from(SIZE), u32::from(SIZE)).expect("valid icon")
}

fn stroke_alpha(distance: f32, width: f32) -> u8 {
    fill_alpha(distance, width / 2.0)
}

fn fill_alpha(distance: f32, radius: f32) -> u8 {
    let edge = distance - radius;
    if edge <= -0.5 {
        255
    } else if edge >= 0.5 {
        0
    } else {
        let alpha = ((0.5 - edge) * 255.0).round().clamp(0.0, 255.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "alpha is rounded and clamped into the u8 channel range"
        )]
        {
            alpha as u8
        }
    }
}

fn distance_to_segment(point: (f32, f32), start: (f32, f32), end: (f32, f32)) -> f32 {
    let vx = end.0 - start.0;
    let vy = end.1 - start.1;
    let wx = point.0 - start.0;
    let wy = point.1 - start.1;
    let segment_len_sq = vx * vx + vy * vy;
    let t = ((wx * vx + wy * vy) / segment_len_sq).clamp(0.0, 1.0);
    let closest_x = start.0 + t * vx;
    let closest_y = start.1 + t * vy;
    let dx = point.0 - closest_x;
    let dy = point.1 - closest_y;
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn bootstrap_creates_account() {
        let cfg = tempdir().unwrap();

        bootstrap_first_launch_with(cfg.path()).unwrap();

        assert!(key_path_in(cfg.path()).exists(), "device key written");

        let config = AppConfig::load_or_default(config_path_in(cfg.path())).unwrap();
        assert!(config.profile.is_some(), "account stamped");
        assert!(
            config.drive(PRIMARY_DRIVE_ID).is_some(),
            "primary drive present"
        );
        assert!(
            !config.blossom_servers.is_empty(),
            "default blossom server seeded"
        );
    }

    #[test]
    fn bootstrap_is_idempotent() {
        let cfg = tempdir().unwrap();

        bootstrap_first_launch_with(cfg.path()).unwrap();
        let first = AppConfig::load_or_default(config_path_in(cfg.path())).unwrap();

        bootstrap_first_launch_with(cfg.path()).unwrap();
        let second = AppConfig::load_or_default(config_path_in(cfg.path())).unwrap();

        // AppKey + profile survive untouched; same npub, same drive id.
        assert_eq!(
            first.profile.as_ref().map(|a| &a.app_key_pubkey),
            second.profile.as_ref().map(|a| &a.app_key_pubkey),
        );
        assert_eq!(first.drives, second.drives);
    }
}
