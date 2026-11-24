use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use super::{Adopted, adopt};

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn previous_install(root: &Path) -> PathBuf {
    let from = root.join("J8PPJKD7TA.to.iris.drive").join("Iris Drive");
    write(&from.join("Config/config.toml"), "profile = \"existing\"\n");
    write(&from.join("Config/keys/app_key"), "secret");
    write(&from.join("Config/blocks/data.mdb"), "blocks");
    write(&from.join("FileProvider/state.json"), "{}");
    std::os::unix::fs::symlink("Config/config.toml", from.join("config-link")).unwrap();
    from
}

#[test]
fn copies_previous_install_once_after_retiring_its_service() {
    let root = tempfile::tempdir().unwrap();
    let from = previous_install(root.path());
    let to = root
        .path()
        .join("8G9P8AN75D.to.iris.drive")
        .join("Iris Drive");
    let retired = RefCell::new(Vec::new());

    let outcome = adopt(&from, &to, |config| {
        assert!(
            !to.join("Config/config.toml").exists(),
            "service retired before copying"
        );
        retired.borrow_mut().push(config.to_path_buf());
        Ok(())
    })
    .unwrap();

    assert_eq!(outcome, Adopted::Copied { files: 4 });
    assert_eq!(retired.into_inner(), vec![from.join("Config")]);
    assert_eq!(
        fs::read_to_string(to.join("Config/keys/app_key")).unwrap(),
        "secret"
    );
    assert_eq!(
        fs::read_to_string(to.join("FileProvider/state.json")).unwrap(),
        "{}"
    );
    assert_eq!(
        fs::read_link(to.join("config-link")).unwrap(),
        Path::new("Config/config.toml")
    );
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(to.join("adopted-from.json")).unwrap()).unwrap();
    assert_eq!(record["from"], from.display().to_string());
    assert_eq!(
        fs::read_to_string(from.join("Config/keys/app_key")).unwrap(),
        "secret"
    );
    let leftovers: Vec<_> = fs::read_dir(to.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name())
        .filter(|name| name != "Iris Drive")
        .collect();
    assert!(
        leftovers.is_empty(),
        "no staging left behind: {leftovers:?}"
    );

    let again = adopt(&from, &to, |_| {
        panic!("an adopted install keeps its service")
    })
    .unwrap();
    assert_eq!(again, Adopted::Current);
}

#[test]
fn points_copied_settings_at_the_new_folder() {
    let root = tempfile::tempdir().unwrap();
    let from = previous_install(root.path());
    let to = root
        .path()
        .join("8G9P8AN75D.to.iris.drive")
        .join("Iris Drive");
    let old = from.display().to_string();
    let escaped = old.replace('/', "\\/");
    write(
        &from.join("fileprovider-runtime.json"),
        &format!(
            "{{\"configDirectory\":\"{escaped}\\/Config\",\"idriveExecutable\":\"\\/Applications\\/Iris Drive.app\\/Contents\\/MacOS\\/idrive\"}}"
        ),
    );
    write(
        &from.join("Config/extra.toml"),
        &format!(
            "cache = \"{old}/Config/cache\"\nroot = \"{old}\"\nsibling = \"{old} Dev/Config\"\n"
        ),
    );
    fs::write(from.join("Config/blocks/data.mdb"), old.as_bytes()).unwrap();

    adopt(&from, &to, |_| Ok(())).unwrap();

    let new = to.display().to_string();
    assert_eq!(
        fs::read_to_string(to.join("fileprovider-runtime.json")).unwrap(),
        format!(
            "{{\"configDirectory\":\"{}\\/Config\",\"idriveExecutable\":\"\\/Applications\\/Iris Drive.app\\/Contents\\/MacOS\\/idrive\"}}",
            new.replace('/', "\\/")
        ),
    );
    assert_eq!(
        fs::read_to_string(to.join("Config/extra.toml")).unwrap(),
        format!(
            "cache = \"{new}/Config/cache\"\nroot = \"{new}\"\nsibling = \"{old} Dev/Config\"\n"
        ),
        "only whole path components of the previous folder change"
    );
    assert_eq!(
        fs::read(to.join("Config/blocks/data.mdb")).unwrap(),
        old.as_bytes(),
        "binary data untouched"
    );
}

#[test]
fn leaves_an_existing_profile_alone() {
    let root = tempfile::tempdir().unwrap();
    let from = previous_install(root.path());
    let to = root.path().join("new").join("Iris Drive");
    write(&to.join("Config/config.toml"), "profile = \"new\"\n");

    assert_eq!(
        adopt(&from, &to, |_| panic!("not retired")).unwrap(),
        Adopted::Current
    );
    assert_eq!(
        fs::read_to_string(to.join("Config/config.toml")).unwrap(),
        "profile = \"new\"\n"
    );
}

#[test]
fn reports_when_there_is_nothing_to_adopt() {
    let root = tempfile::tempdir().unwrap();
    let to = root.path().join("new").join("Iris Drive");
    let outcome = adopt(&root.path().join("missing").join("Iris Drive"), &to, |_| {
        panic!("not retired")
    });
    assert_eq!(outcome.unwrap(), Adopted::NothingToAdopt);
    assert!(!to.exists());
}

#[test]
fn unreadable_previous_data_is_an_error_not_a_fresh_start() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let from = previous_install(root.path());
    fs::set_permissions(from.join("Config"), fs::Permissions::from_mode(0o000)).unwrap();
    let to = root.path().join("new").join("Iris Drive");

    let error = adopt(&from, &to, |_| Ok(())).unwrap_err();
    fs::set_permissions(from.join("Config"), fs::Permissions::from_mode(0o755)).unwrap();

    assert!(
        format!("{error:#}").contains("previous Drive data"),
        "{error:#}"
    );
    assert!(!to.join("Config/config.toml").exists());
}

#[test]
fn keeps_files_created_before_adoption_beside_the_adopted_data() {
    let root = tempfile::tempdir().unwrap();
    let from = previous_install(root.path());
    let to = root.path().join("new").join("Iris Drive");
    write(&to.join("Config/logs/app.log"), "started");

    assert_eq!(
        adopt(&from, &to, |_| Ok(())).unwrap(),
        Adopted::Copied { files: 4 }
    );
    assert_eq!(
        fs::read_to_string(to.join("Config/config.toml")).unwrap(),
        "profile = \"existing\"\n"
    );
    let aside: Vec<_> = fs::read_dir(to.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("Iris Drive.before-adopt-")
        })
        .collect();
    assert_eq!(aside.len(), 1);
    assert_eq!(
        fs::read_to_string(aside[0].join("Config/logs/app.log")).unwrap(),
        "started"
    );
}
