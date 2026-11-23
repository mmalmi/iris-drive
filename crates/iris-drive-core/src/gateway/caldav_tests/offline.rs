use hashtree_core::{Cid, Store};
use nostr_sdk::Keys;

use super::{account_caldav_identity, http_request, init_account_config};
use crate::Daemon;
use crate::calendar::{CalendarData, event_from_ics, local_replica_tree, save_calendar_data};
use crate::config::AppConfig;
use crate::gateway::{GatewayBind, GatewayServer};
use crate::identity::AppKey;
use crate::nostr_events::build_private_hashtree_root_event;
use crate::paths::{config_path_in, key_path_in};
use crate::relay_sync::{FilesRootApply, apply_remote_files_root_event};
use std::path::Path;
use tempfile::tempdir;

fn ical(id: &str, title: &str, location: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:{id}\r\nSUMMARY:{title}\r\nLOCATION:{location}\r\nDTSTART:20260622T100000Z\r\nDTEND:20260622T103000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

async fn start(dir: &Path) -> GatewayServer {
    let daemon = Daemon::open(dir).unwrap();
    GatewayServer::bind_with_tree(dir, daemon.tree_handle(), GatewayBind::loopback_v4(0))
        .await
        .unwrap()
}

async fn import_signed_calendar_root(from: &Path, to: &Path, tree: &str, signer: &Keys) {
    let source = Daemon::open(from).unwrap();
    let target = Daemon::open(to).unwrap();
    let reference =
        &source.config().drive(tree).unwrap().app_key_roots[&signer.public_key().to_hex()];
    let event = build_private_hashtree_root_event(signer, tree, reference).unwrap();
    let root = Cid::parse(&reference.root_cid).unwrap();
    assert!(root.key.is_some());
    let blocks = crate::block_sync::collect_live_sync_hashes(source.tree(), &root, 4)
        .await
        .unwrap();
    for hash in blocks {
        let bytes = source.tree().get_store().get(&hash).await.unwrap().unwrap();
        target.tree().get_store().put(hash, bytes).await.unwrap();
    }
    let mut config = target.config().clone();
    assert_eq!(
        apply_remote_files_root_event(&mut config, &event, Some(signer)).unwrap(),
        FilesRootApply::Applied
    );
    config.save(config_path_in(to)).unwrap();
}

#[tokio::test]
async fn calendar_offline_caldav_writers_merge_after_reopen_and_report_deletes() {
    let original = tempdir().unwrap();
    let other = tempdir().unwrap();
    let reader = tempdir().unwrap();
    init_account_config(original.path());
    let identity = account_caldav_identity(original.path());
    let mut seed = Daemon::open(original.path()).unwrap();
    let signer = AppKey::load(key_path_in(original.path())).unwrap();
    let config = AppConfig::load_or_default(config_path_in(original.path())).unwrap();
    for target in [other.path(), reader.path()] {
        std::fs::copy(key_path_in(original.path()), key_path_in(target)).unwrap();
        config.save(config_path_in(target)).unwrap();
    }
    let tree = crate::calendar::CALENDAR_TREE_NAME;
    let mut data = CalendarData::new(&identity, 1);
    for id in ["shared", "gone"] {
        data.events
            .push(event_from_ics(&ical(id, "Seed", "Room A"), id, 1).unwrap());
    }
    save_calendar_data(&mut seed, &data).await.unwrap();
    drop(seed);
    import_signed_calendar_root(original.path(), other.path(), tree, signer.keys()).await;

    let first = start(original.path()).await;
    let second = start(other.path()).await;
    let href = format!("/caldav/calendars/{identity}/calendar/");
    let shared = format!("{href}shared.ics");
    let gone = format!("{href}gone.ics");
    let old = http_request(first.local_addr(), "GET", "localhost", &shared, &[], b"").await;
    let old_etag = old
        .lines()
        .find_map(|line| line.strip_prefix("etag: "))
        .unwrap();
    let left = ical("shared", "Renamed", "Room A");
    let right = ical("shared", "Seed", "Room B");
    let headers = [("If-Match", old_etag)];
    let (a, b) = tokio::join!(
        http_request(
            first.local_addr(),
            "PUT",
            "localhost",
            &shared,
            &headers,
            left.as_bytes()
        ),
        http_request(
            second.local_addr(),
            "PUT",
            "localhost",
            &shared,
            &headers,
            right.as_bytes()
        ),
    );
    for response in [a, b] {
        assert!(response.starts_with("HTTP/1.1 204"), "{response}");
    }
    for (server, id) in [(&first, "created-a"), (&second, "created-b")] {
        let path = format!("{href}{id}.ics");
        let body = ical(id, id, "Room A");
        let created = http_request(
            server.local_addr(),
            "PUT",
            "localhost",
            &path,
            &[("If-None-Match", "*")],
            body.as_bytes(),
        )
        .await;
        assert!(created.starts_with("HTTP/1.1 201"), "{created}");
        let duplicate = http_request(
            server.local_addr(),
            "PUT",
            "localhost",
            &path,
            &[("If-None-Match", "*")],
            body.as_bytes(),
        )
        .await;
        assert!(duplicate.starts_with("HTTP/1.1 412"), "{duplicate}");
    }
    let late = ical("gone", "Late offline edit", "Room A");
    let (deleted, changed) = tokio::join!(
        http_request(first.local_addr(), "DELETE", "localhost", &gone, &[], b""),
        http_request(
            second.local_addr(),
            "PUT",
            "localhost",
            &gone,
            &[],
            late.as_bytes()
        ),
    );
    assert!(deleted.starts_with("HTTP/1.1 204"), "{deleted}");
    assert!(changed.starts_with("HTTP/1.1 204"), "{changed}");
    first.shutdown().await.unwrap();
    second.shutdown().await.unwrap();

    let state = config.profile.as_ref().unwrap();
    assert!(local_replica_tree(reader.path(), state).unwrap().is_none());
    import_signed_calendar_root(original.path(), reader.path(), tree, signer.keys()).await;
    for from in [other.path(), original.path()] {
        let replica = local_replica_tree(from, state).unwrap().unwrap();
        import_signed_calendar_root(from, reader.path(), &replica, signer.keys()).await;
    }
    let fresh = start(reader.path()).await;
    let report = http_request(
        fresh.local_addr(),
        "GET",
        "localhost",
        "/caldav/calendar.ics",
        &[],
        b"",
    )
    .await;
    for text in [
        "SUMMARY:Renamed",
        "LOCATION:Room B",
        "SUMMARY:created-a",
        "SUMMARY:created-b",
    ] {
        assert!(report.contains(text), "{text} missing from feed");
    }
    assert!(!report.contains("Late offline edit"));
    let deletion_with_old_etag = http_request(
        fresh.local_addr(),
        "DELETE",
        "localhost",
        &shared,
        &[("If-Match", old_etag)],
        b"",
    )
    .await;
    assert!(
        deletion_with_old_etag.starts_with("HTTP/1.1 412"),
        "{deletion_with_old_etag}"
    );
    let incremental = http_request(fresh.local_addr(), "REPORT", "localhost", &href, &[],
        br#"<D:sync-collection xmlns:D="DAV:"><D:sync-token>prior</D:sync-token></D:sync-collection>"#).await;
    assert!(incremental.contains(&format!("<D:href>{gone}</D:href><D:status>HTTP/1.1 404")));
    fresh.shutdown().await.unwrap();
}
