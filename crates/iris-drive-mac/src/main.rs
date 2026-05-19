//! Menu-bar wrapper around the `idrive daemon`.
//!
//! Owns the macOS `NSApplication` main thread, runs a tao event loop,
//! and shows a tray icon plus a small menu. The daemon itself runs as
//! a child process — `iris-drive-mac` parses its JSON event log and
//! reflects state into the menu. Subprocess separation means: no
//! shared mutexes, the CLI binary stays pure, and the daemon's
//! existing supervisor/restart story still works as-is.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result};
use serde_json::Value;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::TrayIconBuilder;

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
                    && let Some(dir) = &cfg_dir_for_loop {
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
                        let fetched = value.get("fetched").and_then(serde_json::Value::as_u64).unwrap_or(0);
                        let () = status_item.set_text(format!("Fetched {fetched} blocks"));
                    }
                    "shutdown" => {
                        let () = status_item.set_text("Daemon stopped");
                    }
                    _ => {}
                }
            }
            Event::UserEvent(UserEvent::DaemonExited(code)) => {
                let () = status_item
                    .set_text(format!("Daemon exited ({code:?}) — restart Iris Drive"));
            }
            _ => {}
        }
    });
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
/// would ship per-state icons (synced / syncing / error); a flat
/// colour blob lets us ship the wrapper today without bundling
/// assets.
fn make_icon() -> tray_icon::Icon {
    const SIZE: u32 = 22;
    const HALF: u32 = SIZE / 2;
    let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    let radius_sq = (HALF - 1) * (HALF - 1);
    for y in 0..SIZE {
        for x in 0..SIZE {
            // Filled circle: transparent at corners, blue inside.
            let dx = x.abs_diff(HALF);
            let dy = y.abs_diff(HALF);
            let r2 = dx * dx + dy * dy;
            if r2 < radius_sq {
                data.extend_from_slice(&[60, 120, 220, 255]);
            } else {
                data.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    tray_icon::Icon::from_rgba(data, SIZE, SIZE).expect("valid icon")
}
