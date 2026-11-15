//! Large files reach the Drive whole through the paths the phones use: the
//! share sheet's import and the Files/SAF/photo-backup write.

use super::*;

fn video_bytes() -> Vec<u8> {
    let mut state: u32 = 11;
    (0..6 * 1024 * 1024 + 7)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 16).to_le_bytes()[0]
        })
        .collect()
}

fn read_back(data_dir: &str, path: &str, out: &Path) -> Vec<u8> {
    let read = super::super::native_provider_read_json(data_dir, path, &out.display().to_string());
    assert!(
        read["error"].as_str().unwrap_or_default().is_empty(),
        "read {path}: {read:#}"
    );
    std::fs::read(out).unwrap()
}

#[test]
fn a_large_shared_video_is_saved_whole_by_both_mobile_paths() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    mark_daemon_live(dir.path());
    let data_dir = dir.path().display().to_string();
    let bytes = video_bytes();
    let source = dir.path().join("staged.mov");
    std::fs::write(&source, &bytes).unwrap();
    let source = source.display().to_string();

    let shared =
        super::super::native_provider_import_shared_file_json(&data_dir, "Trip.mov", &source);
    assert!(
        shared["error"].as_str().unwrap_or_default().is_empty(),
        "{shared:#}"
    );
    let written = super::super::native_provider_write_json(&data_dir, "Videos/Trip.mov", &source);
    assert!(
        written["error"].as_str().unwrap_or_default().is_empty(),
        "{written:#}"
    );
    let again =
        super::super::native_provider_import_shared_file_json(&data_dir, "Trip.mov", &source);
    assert!(
        again["error"].as_str().unwrap_or_default().is_empty(),
        "{again:#}"
    );

    let list = super::super::native_provider_list_json(&data_dir);
    let sizes = list["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == "file")
        .map(|entry| {
            (
                entry["path"].as_str().unwrap().to_owned(),
                entry["size"].as_u64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let len = bytes.len() as u64;
    assert_eq!(
        sizes,
        vec![
            ("Trip (2).mov".to_owned(), len),
            ("Trip.mov".to_owned(), len),
            ("Videos/Trip.mov".to_owned(), len),
        ]
    );
    for path in ["Trip.mov", "Trip (2).mov", "Videos/Trip.mov"] {
        assert!(
            read_back(&data_dir, path, &dir.path().join("out")) == bytes,
            "{path} differs"
        );
    }
}
