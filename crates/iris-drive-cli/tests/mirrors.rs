//! Exercise the same persistent mirror registration and runner used by the Mac app.
use assert_cmd::Command;
use serde_json::Value;
use std::{fs, path::Path};
use tempfile::tempdir;

fn cli(config: &Path, args: &[&str]) -> Value {
    let output = Command::cargo_bin("idrive")
        .unwrap()
        .env("IRIS_DRIVE_CONFIG_DIR", config)
        .args(args)
        .output()
        .unwrap();
    if !output.status.success() {
        assert!(
            args.starts_with(&["mirror", "run"])
                && String::from_utf8_lossy(&output.stderr).contains("need attention"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn scoped_mirror_updates_retries_and_keeps_deleted_files() {
    let tmp = tempdir().unwrap();
    let config = tmp.path().join("config");
    let source = tmp.path().join("source");
    let seed = tmp.path().join("seed");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir(&seed).unwrap();
    fs::write(seed.join("unrelated"), b"mine").unwrap();
    fs::write(source.join("nested/document"), b"first").unwrap();
    fs::write(source.join("keep"), b"keep").unwrap();
    fs::write(source.join(".later.icloud"), b"placeholder").unwrap();
    cli(&config, &["init"]);
    cli(&config, &["import", seed.to_str().unwrap()]);
    cli(
        &config,
        &[
            "mirror",
            "add",
            source.to_str().unwrap(),
            "--to",
            "Archive/iCloud",
            "--json",
        ],
    );
    let run = || cli(&config, &["mirror", "run", "Archive/iCloud", "--json"]);
    let report = run();
    assert_eq!(report["copied"], 2);
    assert_eq!(report["pending"], 1);
    fs::remove_file(source.join(".later.icloud")).unwrap();
    fs::write(source.join("later"), b"downloaded").unwrap();
    fs::remove_file(source.join("keep")).unwrap();
    fs::write(source.join("nested/document"), vec![42; 1_500_000]).unwrap();
    assert_eq!(run()["copied"], 2);
    let report = run();
    assert_eq!(report["copied"], 0);
    assert_eq!(report["unchanged"], 2);
    assert_eq!(report["pending"], 0);
    cli(&config, &["mirror", "remove", "Archive/iCloud", "--json"]);
    assert_eq!(
        cli(&config, &["mirror", "list", "--json"]),
        serde_json::json!([])
    );
    for (path, expected) in [
        ("unrelated", b"mine".to_vec()),
        ("Archive/iCloud/keep", b"keep".to_vec()),
        ("Archive/iCloud/nested/document", vec![42; 1_500_000]),
        ("Archive/iCloud/later", b"downloaded".to_vec()),
    ] {
        let output = tmp.path().join("read");
        cli(
            &config,
            &["provider", "read", path, output.to_str().unwrap()],
        );
        assert_eq!(fs::read(output).unwrap(), expected);
    }
}

#[test]
fn mirror_destination_conflicts_stay_intact_and_registration_is_scoped() {
    let tmp = tempdir().unwrap();
    let config = tmp.path().join("config");
    let source = tmp.path().join("source");
    let seed = tmp.path().join("seed");
    fs::create_dir_all(seed.join("Archive")).unwrap();
    fs::create_dir(&source).unwrap();
    fs::write(seed.join("Archive/existing"), b"original in Iris").unwrap();
    fs::write(source.join("existing"), b"unrelated").unwrap();
    fs::write(source.join("new"), b"new").unwrap();
    fs::create_dir(source.join("empty")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(seed.join("Archive/existing"), source.join("link")).unwrap();
    cli(&config, &["init"]);
    cli(&config, &["import", seed.to_str().unwrap()]);
    cli(
        &config,
        &[
            "mirror",
            "add",
            source.to_str().unwrap(),
            "--to",
            "Archive",
            "--json",
        ],
    );
    for dest in ["Archive/other", "archive", "../outside", ".hashtree"] {
        Command::cargo_bin("idrive")
            .unwrap()
            .env("IRIS_DRIVE_CONFIG_DIR", &config)
            .args(["mirror", "add", source.to_str().unwrap(), "--to", dest])
            .assert()
            .failure();
    }
    let r = cli(&config, &["mirror", "run", "Archive", "--json"]);
    assert_eq!(r["copied"], 1);
    assert_eq!(r["pending"], if cfg!(unix) { 2 } else { 1 });
    let listing = cli(&config, &["provider", "list"]);
    let entries = listing["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["path"] == "Archive/empty" && e["kind"] == "directory")
    );
    assert!(!entries.iter().any(|e| e["path"] == "Archive/link"));
    let out = tmp.path().join("restored");
    cli(
        &config,
        &[
            "provider",
            "read",
            "Archive/existing",
            out.to_str().unwrap(),
        ],
    );
    assert_eq!(fs::read(out).unwrap(), b"original in Iris");
    fs::rename(&source, tmp.path().join("unplugged")).unwrap();
    let last = cli(&config, &["mirror", "run", "Archive", "--json"]);
    assert_eq!(last["pending"], 1);
    let status = cli(&config, &["mirror", "status", "--json"]);
    assert_eq!(status[0]["last_run"], last);
    assert_eq!(status[0]["running"], false);
}
