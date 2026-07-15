//! End-to-end coverage for daemon-owned browser gateway startup.

use assert_cmd::Command;
use std::process::{Output, Stdio};
use tempfile::tempdir;

#[allow(dead_code)]
mod support;

use support::LocalNostrRelay;

fn idrive(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("idrive").unwrap();
    cmd.env("IRIS_DRIVE_CONFIG_DIR", dir);
    cmd
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstatus: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_json(dir: &std::path::Path, args: &[&str]) -> serde_json::Value {
    let output = idrive(dir).args(args).output().unwrap();
    assert_success(&output);
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "invalid json: {err}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_keeps_sync_running_when_gateway_port_is_busy() {
    let dir = tempdir().unwrap();
    let relay = LocalNostrRelay::spawn().await;
    run_json(dir.path(), &["init", "--label", "owner"]);
    let occupied_port = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let gateway_port = occupied_port.local_addr().unwrap().port().to_string();
    let status_path = dir.path().join("daemon-status.json");
    let stdout_path = dir.path().join("daemon.stdout.log");
    let stderr_path = dir.path().join("daemon.stderr.log");
    let stdout = std::fs::File::create(&stdout_path).unwrap();
    let stderr = std::fs::File::create(&stderr_path).unwrap();

    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("idrive"))
        .env("IRIS_DRIVE_CONFIG_DIR", dir.path())
        .env("IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP", "false")
        .env("IRIS_DRIVE_FIPS_ENABLE_WEBRTC", "false")
        .env("IRIS_DRIVE_FIPS_UDP_BIND_ADDR", "127.0.0.1:0")
        .env("IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR", "")
        .env("IRIS_DRIVE_FIPS_UDP_PUBLIC", "false")
        .args([
            "daemon",
            "--relay",
            &relay.url,
            "--watch-interval",
            "0",
            "--gateway-port",
            &gateway_port,
        ])
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .unwrap();

    let mut running_status = None;
    for _ in 0..600 {
        if let Some(exit) = child.try_wait().unwrap() {
            panic!("daemon exited before running status: {exit}");
        }
        if let Ok(data) = std::fs::read(&status_path)
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data)
            && value["running"] == true
            && value["browser_gateway"]["running"] == false
            && value["browser_gateway"]["disabled_by"] == "bind_error"
        {
            running_status = Some(value);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let status = running_status.unwrap_or_else(|| {
        let _ = child.kill();
        let status = std::fs::read_to_string(&status_path).unwrap_or_default();
        let stdout = std::fs::read_to_string(&stdout_path).unwrap_or_default();
        let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
        panic!(
            "daemon did not write running bind-error status with a busy gateway port\nstatus: {status}\nstdout: {stdout}\nstderr: {stderr}"
        );
    });

    assert_eq!(status["browser_gateway"]["requested"], true);
    assert_eq!(status["browser_gateway"]["enabled"], true);
    assert_eq!(status["browser_gateway"]["running"], false);
    assert_eq!(status["browser_gateway"]["disabled_by"], "bind_error");
    assert!(status["browser_gateway"]["error"].as_str().is_some());
    assert!(child.try_wait().unwrap().is_none());
    child.kill().unwrap();
    let _ = child.wait();
}
