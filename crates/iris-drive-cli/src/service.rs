#[allow(clippy::wildcard_imports)]
use super::*;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::{Command as ProcessCommand, Stdio};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
const BINARY_VERSION_QUERY_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(target_os = "macos")]
const BINARY_VERSION_QUERY_POLL_INTERVAL: Duration = Duration::from_millis(20);

pub(crate) fn cmd_service(config_dir: &Path, command: ServiceCmd) -> Result<()> {
    let json_output = service_command_json(&command);
    let payload = match command {
        ServiceCmd::Install {
            launch, executable, ..
        } => install_service(config_dir, executable.as_deref(), launch)?,
        ServiceCmd::Start { .. } => start_service(config_dir)?,
        ServiceCmd::Stop { .. } => stop_service(config_dir)?,
        ServiceCmd::Uninstall { .. } => uninstall_service(config_dir)?,
        ServiceCmd::Status { .. } => service_status_payload(config_dir),
    };
    if json_output {
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        println!("{payload}");
    }
    Ok(())
}

fn service_command_json(command: &ServiceCmd) -> bool {
    match command {
        ServiceCmd::Install { json, .. }
        | ServiceCmd::Start { json }
        | ServiceCmd::Stop { json }
        | ServiceCmd::Uninstall { json }
        | ServiceCmd::Status { json } => *json,
    }
}

pub(crate) fn service_status_payload(config_dir: &Path) -> Value {
    #[cfg(target_os = "macos")]
    {
        match macos_service_status(config_dir) {
            Ok(status) => status,
            Err(error) => json!({
                "supported": true,
                "kind": "launchagent",
                "installed": false,
                "loaded": false,
                "running": false,
                "error": format!("{error:#}"),
            }),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = config_dir;
        json!({
            "supported": false,
            "kind": platform_service_kind(),
            "installed": false,
            "loaded": false,
            "running": false,
        })
    }
}

#[cfg(not(target_os = "macos"))]
fn platform_service_kind() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "systemd-user"
    }
    #[cfg(target_os = "windows")]
    {
        "windows-service"
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        "unsupported"
    }
}

fn install_service(config_dir: &Path, executable: Option<&Path>, launch: bool) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        macos_install_service(config_dir, executable, launch)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (config_dir, executable, launch);
        Err(anyhow::anyhow!(
            "Iris Drive service supervision is not implemented for this platform yet"
        ))
    }
}

fn start_service(config_dir: &Path) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        macos_start_service(config_dir)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = config_dir;
        Err(anyhow::anyhow!(
            "Iris Drive service supervision is not implemented for this platform yet"
        ))
    }
}

fn stop_service(config_dir: &Path) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        macos_stop_service(config_dir)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = config_dir;
        Err(anyhow::anyhow!(
            "Iris Drive service supervision is not implemented for this platform yet"
        ))
    }
}

fn uninstall_service(config_dir: &Path) -> Result<Value> {
    #[cfg(target_os = "macos")]
    {
        macos_uninstall_service(config_dir)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = config_dir;
        Err(anyhow::anyhow!(
            "Iris Drive service supervision is not implemented for this platform yet"
        ))
    }
}

#[allow(dead_code)]
pub(crate) fn macos_launch_agent_label(config_dir: &Path) -> String {
    format!(
        "to.iris.drive.daemon.{}",
        stable_path_hash_hex(config_dir, 16)
    )
}

#[allow(dead_code)]
pub(crate) fn macos_launch_agent_plist(config_dir: &Path, executable: &Path) -> String {
    let home = std::env::var("HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| macos_launch_agent_home_from_config_dir(config_dir));
    macos_launch_agent_plist_with_home(config_dir, executable, home.as_deref())
}

fn macos_launch_agent_home_from_config_dir(config_dir: &Path) -> Option<String> {
    let path = config_dir.to_string_lossy();
    let (home, _) = path.split_once("/Library/")?;
    (!home.is_empty()).then(|| home.to_owned())
}

fn macos_launch_agent_plist_with_home(
    config_dir: &Path,
    executable: &Path,
    home: Option<&str>,
) -> String {
    let label = macos_launch_agent_label(config_dir);
    let stdout_path = config_dir.join("logs").join("daemon.out.log");
    let stderr_path = config_dir.join("logs").join("daemon.err.log");
    let arguments = [
        executable.display().to_string(),
        "--config-dir".to_owned(),
        config_dir.display().to_string(),
        "daemon".to_owned(),
        "--service".to_owned(),
        "--watch-interval".to_owned(),
        "0".to_owned(),
        "--watch-debounce-ms".to_owned(),
        "100".to_owned(),
    ];
    let program_arguments = arguments
        .iter()
        .map(|argument| format!("        <string>{}</string>", xml_escape(argument)))
        .collect::<Vec<_>>()
        .join("\n");
    let environment_variables = home.map_or_else(String::new, |home| {
        format!(
            r#"    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>{}</string>
    </dict>
"#,
            xml_escape(home)
        )
    });

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
{}
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
{}    <key>ProcessType</key>
    <string>Background</string>
    <key>StandardOutPath</key>
    <string>{}</string>
    <key>StandardErrorPath</key>
    <string>{}</string>
</dict>
</plist>
"#,
        xml_escape(&label),
        program_arguments,
        environment_variables,
        xml_escape(&stdout_path.display().to_string()),
        xml_escape(&stderr_path.display().to_string())
    )
}

#[cfg(target_os = "macos")]
fn macos_install_service(
    config_dir: &Path,
    executable: Option<&Path>,
    launch: bool,
) -> Result<Value> {
    let executable = match executable {
        Some(path) => std::fs::canonicalize(path)
            .with_context(|| format!("canonicalizing service executable {}", path.display()))?,
        None => std::fs::canonicalize(
            std::env::current_exe().context("locating current idrive executable")?,
        )
        .context("canonicalizing current idrive executable")?,
    };
    let plist_path = macos_launch_agent_plist_path(config_dir)?;
    let plist = macos_launch_agent_plist(config_dir, &executable);
    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating LaunchAgents directory {}", parent.display()))?;
    }
    std::fs::create_dir_all(config_dir.join("logs")).with_context(|| {
        format!(
            "creating daemon log directory {}",
            config_dir.join("logs").display()
        )
    })?;
    std::fs::write(&plist_path, plist)
        .with_context(|| format!("writing LaunchAgent {}", plist_path.display()))?;
    let mut payload = macos_service_status(config_dir)?;
    if launch {
        payload = macos_start_service(config_dir)?;
    }
    if let Some(object) = payload.as_object_mut() {
        object.insert("installed".to_owned(), json!(true));
        object.insert("plist_written".to_owned(), json!(true));
        object.insert(
            "executable".to_owned(),
            json!(executable.display().to_string()),
        );
        object.insert(
            "plist_path".to_owned(),
            json!(plist_path.display().to_string()),
        );
    }
    Ok(payload)
}

#[cfg(target_os = "macos")]
fn macos_start_service(config_dir: &Path) -> Result<Value> {
    let plist_path = macos_launch_agent_plist_path(config_dir)?;
    if !plist_path.exists() {
        return Err(anyhow::anyhow!(
            "daemon service is not installed for {}",
            config_dir.display()
        ));
    }
    let label = macos_launch_agent_label(config_dir);
    let domain = macos_launchctl_domain()?;
    let commands = macos_start_launchctl_commands(&domain, &label, &plist_path);
    let bootout = command_refs(&commands[0]);
    let _ = launchctl(&bootout);
    let enable = command_refs(&commands[1]);
    launchctl_success(&enable).context("enabling Iris Drive LaunchAgent")?;
    let bootstrap = command_refs(&commands[2]);
    launchctl_success(&bootstrap).context("bootstrapping Iris Drive LaunchAgent")?;
    let kickstart = command_refs(&commands[3]);
    launchctl_success(&kickstart).context("starting Iris Drive LaunchAgent")?;
    macos_service_status(config_dir)
}

#[cfg(target_os = "macos")]
fn macos_stop_service(config_dir: &Path) -> Result<Value> {
    let label = macos_launch_agent_label(config_dir);
    let plist_path = macos_launch_agent_plist_path(config_dir)?;
    let domain = macos_launchctl_domain()?;
    let _ = launchctl(&["disable", &format!("{domain}/{label}")]);
    if plist_path.exists() {
        let _ = launchctl(&["bootout", &domain, &plist_path.display().to_string()]);
    }
    macos_service_status(config_dir)
}

#[cfg(target_os = "macos")]
fn macos_uninstall_service(config_dir: &Path) -> Result<Value> {
    let plist_path = macos_launch_agent_plist_path(config_dir)?;
    let _ = macos_stop_service(config_dir);
    match std::fs::remove_file(&plist_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("removing {}", plist_path.display()));
        }
    }
    macos_service_status(config_dir)
}

#[cfg(target_os = "macos")]
fn macos_service_status(config_dir: &Path) -> Result<Value> {
    let label = macos_launch_agent_label(config_dir);
    let plist_path = macos_launch_agent_plist_path(config_dir)?;
    let domain = macos_launchctl_domain()?;
    let print_target = format!("{domain}/{label}");
    let output = launchctl(&["print", &print_target]);
    let (loaded, running, pid, detail) = match output {
        Ok(output) => {
            let detail = String::from_utf8_lossy(&output.stdout).to_string();
            let running = output.status.success() && macos_launchctl_detail_running(&detail);
            let pid = parse_launchctl_pid(&detail);
            (output.status.success(), running, pid, detail)
        }
        Err(error) => (false, false, None, format!("{error:#}")),
    };
    let service_binary = macos_service_executable_path(&plist_path);
    let binary_version = service_binary
        .as_deref()
        .and_then(query_binary_version)
        .unwrap_or_default();
    Ok(json!({
        "supported": true,
        "kind": "launchagent",
        "label": label,
        "domain": domain,
        "plist_path": plist_path.display().to_string(),
        "installed": plist_path.exists(),
        "loaded": loaded,
        "running": running,
        "pid": pid,
        "binary_path": service_binary
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        "binary_version": binary_version,
        "detail": detail.lines().next().unwrap_or_default(),
    }))
}

#[cfg(target_os = "macos")]
fn macos_service_executable_path(plist_path: &Path) -> Option<PathBuf> {
    let plist = std::fs::read_to_string(plist_path).ok()?;
    macos_service_executable_path_from_plist_contents(&plist).map(PathBuf::from)
}

#[cfg(any(target_os = "macos", test))]
fn macos_service_executable_path_from_plist_contents(plist: &str) -> Option<String> {
    let program_arguments = plist.split_once("<key>ProgramArguments</key>")?.1;
    let value = program_arguments
        .split_once("<string>")?
        .1
        .split_once("</string>")?
        .0;
    let value = xml_unescape(value.trim());
    (!value.is_empty()).then_some(value)
}

#[cfg(target_os = "macos")]
fn query_binary_version(path: &Path) -> Option<String> {
    use std::io::Read as _;

    let mut child = ProcessCommand::new(path)
        .args(["version", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= BINARY_VERSION_QUERY_TIMEOUT => {
                let _ = child.kill();
                let _ = child.try_wait();
                return None;
            }
            Ok(None) => std::thread::sleep(BINARY_VERSION_QUERY_POLL_INTERVAL),
            Err(_) => {
                let _ = child.kill();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }

    let mut stdout = Vec::new();
    child.stdout.take()?.read_to_end(&mut stdout).ok()?;
    let value = serde_json::from_slice::<Value>(&stdout).ok()?;
    value
        .get("version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|version| !version.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(target_os = "macos")]
fn macos_launch_agent_plist_path(config_dir: &Path) -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("home directory unavailable"))?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{}.plist", macos_launch_agent_label(config_dir))))
}

#[cfg(target_os = "macos")]
fn macos_launchctl_domain() -> Result<String> {
    let output = ProcessCommand::new("/usr/bin/id")
        .arg("-u")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("running id -u")?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "id -u failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let uid = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok(format!("gui/{uid}"))
}

#[cfg(target_os = "macos")]
fn launchctl(args: &[&str]) -> std::io::Result<std::process::Output> {
    ProcessCommand::new("/bin/launchctl")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
}

#[cfg(target_os = "macos")]
fn launchctl_success(args: &[&str]) -> Result<()> {
    let output =
        launchctl(args).with_context(|| format!("running launchctl {}", args.join(" ")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "launchctl {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

#[cfg(any(target_os = "macos", test))]
fn macos_start_launchctl_commands(
    domain: &str,
    label: &str,
    plist_path: &Path,
) -> Vec<Vec<String>> {
    let plist_path = plist_path.display().to_string();
    let target = format!("{domain}/{label}");
    vec![
        vec!["bootout".to_owned(), domain.to_owned(), plist_path.clone()],
        vec!["enable".to_owned(), target.clone()],
        vec!["bootstrap".to_owned(), domain.to_owned(), plist_path],
        vec!["kickstart".to_owned(), "-k".to_owned(), target],
    ]
}

#[cfg(target_os = "macos")]
fn command_refs(command: &[String]) -> Vec<&str> {
    command.iter().map(String::as_str).collect()
}

#[cfg(any(target_os = "macos", test))]
fn parse_launchctl_pid(detail: &str) -> Option<u32> {
    detail.lines().find_map(|line| {
        line.trim()
            .strip_prefix("pid = ")
            .and_then(|pid| pid.parse::<u32>().ok())
    })
}

#[cfg(any(target_os = "macos", test))]
fn macos_launchctl_detail_running(detail: &str) -> bool {
    detail.contains("state = running") || parse_launchctl_pid(detail).is_some()
}

fn stable_path_hash_hex(path: &Path, width: usize) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in path.display().to_string().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
        .chars()
        .take(width)
        .collect::<String>()
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_launch_agent_plist_runs_service_supervised_daemon() {
        let config_dir = Path::new("/Users/example/Library/Application Support/Iris Drive/Config");
        let executable = Path::new("/Applications/Iris Drive.app/Contents/MacOS/idrive");

        let plist = macos_launch_agent_plist(config_dir, executable);

        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains("<string>--config-dir</string>"));
        assert!(plist.contains(
            "<string>/Users/example/Library/Application Support/Iris Drive/Config</string>"
        ));
        assert!(plist.contains("<string>daemon</string>"));
        assert!(plist.contains("<string>--service</string>"));
        assert!(plist.contains("<string>--watch-interval</string>"));
        assert!(plist.contains("<string>0</string>"));
        assert!(!plist.contains("IRIS_DRIVE_PARENT_PID"));
    }

    #[test]
    fn macos_launch_agent_plist_sets_home_for_launchd_environment() {
        let config_dir = Path::new("/Users/example/Library/Application Support/Iris Drive/Config");
        let executable = Path::new("/Applications/Iris Drive.app/Contents/MacOS/idrive");

        let plist =
            macos_launch_agent_plist_with_home(config_dir, executable, Some("/Users/example & co"));

        assert!(plist.contains("<key>EnvironmentVariables</key>"));
        assert!(plist.contains("<key>HOME</key>"));
        assert!(plist.contains("<string>/Users/example &amp; co</string>"));
    }

    #[test]
    fn macos_launch_agent_plist_parser_extracts_service_executable() {
        let config_dir = Path::new("/Users/example/Library/Application Support/Iris Drive/Config");
        let executable = Path::new("/Applications/Iris & Drive.app/Contents/MacOS/idrive");
        let plist = macos_launch_agent_plist(config_dir, executable);

        assert_eq!(
            macos_service_executable_path_from_plist_contents(&plist).as_deref(),
            Some("/Applications/Iris & Drive.app/Contents/MacOS/idrive")
        );
    }

    #[test]
    fn macos_launch_agent_label_is_stable_and_launchctl_safe() {
        let first = macos_launch_agent_label(Path::new("/tmp/iris drive/config"));
        let second = macos_launch_agent_label(Path::new("/tmp/iris drive/config"));

        assert_eq!(first, second);
        assert!(first.starts_with("to.iris.drive.daemon."));
        assert!(first.chars().all(|c| c.is_ascii_alphanumeric() || c == '.'));
    }

    #[test]
    fn macos_start_enables_disabled_launch_agent_before_bootstrap() {
        let commands = macos_start_launchctl_commands(
            "gui/501",
            "to.iris.drive.daemon.test",
            Path::new("/Users/example/Library/LaunchAgents/to.iris.drive.daemon.test.plist"),
        );
        let names = commands
            .iter()
            .map(|command| command[0].as_str())
            .collect::<Vec<_>>();

        assert_eq!(names, ["bootout", "enable", "bootstrap", "kickstart"]);
        assert_eq!(commands[1], ["enable", "gui/501/to.iris.drive.daemon.test"]);
    }

    #[test]
    fn macos_launchctl_pid_counts_as_running() {
        let detail = r"
gui/501/to.iris.drive.daemon.test = {
    state = spawn scheduled
    pid = 4242
}
";

        assert!(macos_launchctl_detail_running(detail));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_binary_version_query_reads_version_json() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let idrive = dir.path().join("idrive");
        std::fs::write(
            &idrive,
            format!(
                "#!/bin/sh\n[ \"$1\" = version ] || exit 2\n[ \"$2\" = --json ] || exit 2\necho '{{\"version\":\"{}\"}}'\n",
                env!("CARGO_PKG_VERSION")
            ),
        )
        .unwrap();
        std::fs::set_permissions(&idrive, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            query_binary_version(&idrive).as_deref(),
            Some(env!("CARGO_PKG_VERSION")),
        );
    }
}
