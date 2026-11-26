//! Repeatable scaling probe for native provider listing, path resolution, and range reads.

use std::time::Instant;

use super::*;

#[test]
#[ignore = "manual scaling probe; run with --ignored --nocapture"]
fn native_provider_list_and_range_read_probe() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    mark_daemon_live(dir.path());

    let src = tempfile::tempdir().unwrap();
    let photos = src.path().join("Photos");
    std::fs::create_dir(&photos).unwrap();
    for i in 0..1_000 {
        std::fs::write(
            photos.join(format!("IMG_{i:04}.jpg")),
            format!("photo-{i:04}"),
        )
        .unwrap();
    }
    for d in 0..10 {
        let sub = src.path().join(format!("Folder_{d:02}"));
        std::fs::create_dir(&sub).unwrap();
        for j in 0..10 {
            std::fs::write(sub.join(format!("doc_{j:02}.txt")), format!("doc-{d}-{j}")).unwrap();
        }
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let mut daemon = iris_drive_core::Daemon::open(dir.path()).unwrap();
        daemon.import_source_dir(src.path()).await.unwrap();
    });

    let data_dir = dir.path().display().to_string();
    let mut list_ms = Vec::new();
    for _ in 0..5 {
        let t0 = Instant::now();
        let list = super::super::native_provider_list_json(&data_dir);
        list_ms.push(t0.elapsed().as_millis());
        assert_eq!(list["file_count"].as_u64(), Some(1_100));
    }
    list_ms.sort_unstable();
    println!(
        "native_provider_list (1100 files, 1000 in Photos): median={} ms all={list_ms:?}",
        list_ms[list_ms.len() / 2]
    );

    let mut resolve_ms = Vec::new();
    for _ in 0..5 {
        let t0 = Instant::now();
        let res = super::super::native_provider_resolve_path_json(
            &data_dir,
            "Photos",
            "IMG_0000.jpg",
            "",
        );
        resolve_ms.push(t0.elapsed().as_millis());
        assert_eq!(res["path"].as_str(), Some("Photos/IMG_0000 (2).jpg"));
    }
    resolve_ms.sort_unstable();
    println!(
        "native_provider_resolve_path (Photos): median={} ms all={resolve_ms:?}",
        resolve_ms[resolve_ms.len() / 2]
    );

    measure_encrypted_range_read(dir.path(), &rt);
}

fn measure_encrypted_range_read(data_dir: &std::path::Path, rt: &tokio::runtime::Runtime) {
    let video_dir = tempfile::tempdir().unwrap();
    let video_path = video_dir.path().join("video.mov");
    let mut video = vec![0u8; 64 * 1024 * 1024];
    let mut prng: u32 = 23;
    for chunk in video.as_chunks_mut::<4>().0 {
        prng = prng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *chunk = prng.to_le_bytes();
    }
    std::fs::write(&video_path, &video).unwrap();
    rt.block_on(async {
        let daemon = iris_drive_core::Daemon::open(data_dir).unwrap();
        let (_, visible, _) = daemon.provider_projection().await.unwrap();
        let provider = HashTreeProviderFs::open(daemon.tree_handle(), visible.root_cid)
            .await
            .unwrap();
        iris_drive_core::provider::write_provider_file_from_path(
            &provider,
            "Videos/video.mov",
            &video_path,
        )
        .await
        .unwrap();

        let mut range_ms = Vec::new();
        let offset = 32 * 1024 * 1024usize;
        for _ in 0..5 {
            let t0 = Instant::now();
            let slice = provider
                .read(&"Videos/video.mov".to_owned(), offset as u64, 4096)
                .await
                .unwrap();
            range_ms.push(t0.elapsed().as_millis());
            assert_eq!(slice.as_slice(), &video[offset..offset + 4096]);
        }
        range_ms.sort_unstable();
        println!(
            "HashTreeProviderFs::read (4 KiB range in 64 MiB encrypted video): median={} ms all={range_ms:?}",
            range_ms[range_ms.len() / 2]
        );
    });
}
