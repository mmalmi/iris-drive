use super::*;
use crate::app_key_link_invite::encode_app_key_link_invite;
use crate::app_key_link_transport::create_app_key_approval_bootstrap;
use nostr_sdk::Keys;
use nostr_sdk::nips::nip19::ToBech32;

#[test]
fn classify_link_input_is_shared_for_invites_app_keys_and_approval_links() {
    let profile_id = NostrIdentityId::new_v4();
    let admin = Keys::generate().public_key();
    let invite_key = Keys::generate().public_key();
    let invite = encode_app_key_link_invite(profile_id, &admin.to_hex(), &invite_key.to_hex())
        .expect("invite");

    let invite_classification = classify_link_input(&invite);
    assert_eq!(invite_classification.kind, "invite");
    assert!(invite_classification.is_complete);
    assert!(invite_classification.is_valid);
    assert_eq!(
        invite_classification.admin_app_key_pubkey,
        admin.to_bech32().expect("npub")
    );
    assert!(invite_classification.has_invite_pubkey);

    let app_key_npub = admin.to_bech32().expect("npub");
    let app_key = classify_link_input(&app_key_npub);
    assert_eq!(app_key.kind, "app_key_pubkey");
    assert!(app_key.is_complete);
    assert!(app_key.is_valid);
    assert_eq!(app_key.normalized_input, app_key_npub);
    assert_eq!(app_key.admin_app_key_pubkey, app_key_npub);

    let short_app_key = classify_link_input("npub1short");
    assert_eq!(short_app_key.kind, "app_key_pubkey");
    assert!(!short_app_key.is_complete);
    assert!(!short_app_key.is_valid);

    let request_device = Keys::generate();
    let request_device_npub = request_device.public_key().to_bech32().expect("npub");
    let request =
        create_app_key_approval_bootstrap(&request_device, None).expect("approval bootstrap");
    let approval = classify_link_input(&request.url);
    assert_eq!(approval.kind, "app_key_approval");
    assert!(approval.is_complete);
    assert!(approval.is_valid);
    assert_eq!(approval.app_key_pubkey, request_device_npub);
    assert!(!approval.has_invite_pubkey);
}

#[test]
fn classify_invite_routes_distinguishes_partial_and_nearby_links() {
    let short = classify_link_input("https://drive.iris.to/invite/demo");
    assert_eq!(short.kind, "invite");
    assert!(!short.is_complete);
    assert!(!short.is_valid);

    let unrelated = classify_link_input("https://drive.iris.to/app-key-linker?owner=npub1x");
    assert_eq!(unrelated.kind, "iris_web");
    assert!(unrelated.is_valid);

    let custom_scheme_invite = classify_link_input("iris-drive://invite/demo");
    assert_eq!(custom_scheme_invite.kind, "unknown");
}

#[test]
fn classify_device_invites_accepts_shared_scheme_and_nostr_wrappers() {
    use base64::Engine;

    let profile_id = NostrIdentityId::from_uuid(uuid::Uuid::from_u128(1));
    let admin = Keys::parse(&"01".repeat(32))
        .expect("admin key")
        .public_key();
    let invite = Keys::parse(&"02".repeat(32))
        .expect("invite key")
        .public_key();
    // Match the JSON payload produced by iris-drive-web / nostr-identity JS.
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&serde_json::json!({
            "v": 1,
            "profileId": profile_id,
            "adminAppKeyNpub": admin.to_bech32().expect("admin npub"),
            "inviteNpub": invite.to_bech32().expect("invite npub"),
        }))
        .expect("invite JSON"),
    );
    for prefix in [
        "https://drive.iris.to/invite/",
        "nostr-identity://device-link/",
        "nostr:https://drive.iris.to/invite/",
        "NOSTR:NOSTR-IDENTITY://DEVICE-LINK/",
    ] {
        let url = format!("{prefix}{payload}?source=web#invite");
        let target = resolve_app_key_link_target(&url, None).expect("invite target");
        assert_eq!(target.profile_id, profile_id);
        assert_eq!(target.admin_app_key_hex, admin.to_hex());
        assert_eq!(target.invite_pubkey, invite.to_hex());

        let classification = classify_link_input(&url);
        assert_eq!(classification.kind, "invite", "{prefix}");
        assert!(classification.is_complete, "{prefix}");
        assert!(classification.is_valid, "{prefix}");
        assert!(classification.has_invite_pubkey, "{prefix}");
        assert_eq!(
            classification.admin_app_key_pubkey,
            admin.to_bech32().unwrap()
        );
    }
}

#[test]
fn partial_device_invite_completion_ignores_query_and_fragment() {
    for prefix in [
        "https://drive.iris.to/invite/",
        "nostr-identity://device-link/",
    ] {
        for suffix in [
            "",
            "?source=abcdefghijklmnopqrstuvwxyz123456",
            "#abcdefghijklmnopqrstuvwxyz123456",
        ] {
            let classification = classify_link_input(&format!("{prefix}demo{suffix}"));
            assert_eq!(classification.kind, "invite");
            assert!(!classification.is_complete);
            assert!(!classification.is_valid);
            assert_eq!(classification.error.len(), 0);
        }
    }
}

#[test]
fn classify_share_dialog_links_returns_folder_and_name() {
    let app_link =
        classify_link_input("iris-drive://share?path=My%20Drive%2FProjects&name=Projects");
    assert_eq!(app_link.kind, "share_dialog");
    assert!(app_link.is_complete);
    assert!(app_link.is_valid);
    assert_eq!(app_link.share_source_path, "My Drive/Projects");
    assert_eq!(app_link.share_display_name, "Projects");

    let hinted = classify_link_input(
        "https://drive.iris.to/share?path=Projects%2FAlpha&name=Alpha&recipient_npub=npub1alice&recipient_name=Alice&recipient_profile=123e4567-e89b-42d3-a456-426614174000",
    );
    assert_eq!(hinted.share_recipient_npub_hint, "npub1alice");
    assert_eq!(hinted.share_recipient_display_name, "Alice");
    assert_eq!(
        hinted.share_recipient_profile_id,
        "123e4567-e89b-42d3-a456-426614174000"
    );

    let web_link = classify_link_input("https://drive.iris.to/share?path=%2FShared%20Source");
    assert_eq!(web_link.kind, "share_dialog");
    assert!(web_link.is_complete);
    assert!(web_link.is_valid);
    assert_eq!(web_link.share_source_path, "/Shared Source");
    assert_eq!(web_link.share_display_name.len(), 0);

    let missing_path = classify_link_input("iris-drive://share?name=Nope");
    assert_eq!(missing_path.kind, "share_dialog");
    assert!(!missing_path.is_complete);
    assert!(!missing_path.is_valid);
}

#[test]
fn classify_drive_nhash_file_link_opens_immutable_content() {
    let input = "https://drive.iris.to/#/nhash1qqsyktrn6c5r444rhjt2qfv6a6uu5hcsrlcvk202whqhxyk3fwkl83s9yr8ngvg5489t2sqnpzqyk7um2ug688j42y57375qex7vgpc384vdv9mr60t/freenet.pdf?fullscreen=1";
    let file = classify_link_input(input);

    assert_eq!(file.kind, "nhash_file");
    assert!(file.is_complete);
    assert!(file.is_valid);
    assert_eq!(
        file.content_nhash,
        "nhash1qqsyktrn6c5r444rhjt2qfv6a6uu5hcsrlcvk202whqhxyk3fwkl83s9yr8ngvg5489t2sqnpzqyk7um2ug688j42y57375qex7vgpc384vdv9mr60t"
    );
    assert_eq!(file.content_path_hint, "freenet.pdf");
    assert_eq!(file.open_display_name, "freenet.pdf");
    assert_eq!(
        file.local_open_url,
        "http://nhash.iris.localhost:17321/nhash1qqsyktrn6c5r444rhjt2qfv6a6uu5hcsrlcvk202whqhxyk3fwkl83s9yr8ngvg5489t2sqnpzqyk7um2ug688j42y57375qex7vgpc384vdv9mr60t/freenet.pdf"
    );

    let encoded = classify_link_input(
        "https://drive.iris.to/#/nhash1qqsyktrn6c5r444rhjt2qfv6a6uu5hcsrlcvk202whqhxyk3fwkl83s9yr8ngvg5489t2sqnpzqyk7um2ug688j42y57375qex7vgpc384vdv9mr60t/Freenet%20paper.pdf",
    );
    assert_eq!(encoded.content_path_hint, "Freenet paper.pdf");
    assert_eq!(encoded.open_display_name, "Freenet paper.pdf");

    let traversal = classify_link_input(
        "https://drive.iris.to/#/nhash1qqsyktrn6c5r444rhjt2qfv6a6uu5hcsrlcvk202whqhxyk3fwkl83s9yr8ngvg5489t2sqnpzqyk7um2ug688j42y57375qex7vgpc384vdv9mr60t/../secret.pdf",
    );
    assert_eq!(traversal.kind, "nhash_file");
    assert!(!traversal.is_valid);
}

#[test]
fn classify_drive_npub_file_link_opens_mutable_content_path() {
    let input = format!(
        "https://drive.iris.to/#/{}/sites/docs/Freenet%20paper.pdf?fullscreen=1",
        crate::gateway::IRIS_SITES_PORTAL_NPUB
    );
    let file = classify_link_input(&input);

    assert_eq!(file.kind, "mutable_file");
    assert!(file.is_complete);
    assert!(file.is_valid);
    assert_eq!(file.content_path_hint, "docs/Freenet paper.pdf");
    assert_eq!(file.open_display_name, "Freenet paper.pdf");
    assert_eq!(
        file.local_open_url,
        format!(
            "http://iris.localhost:17321/{}/sites/docs/Freenet%20paper.pdf",
            crate::gateway::IRIS_SITES_PORTAL_NPUB
        )
    );
}

#[test]
fn classify_public_iris_app_link_opens_isolated_local_origin() {
    let app = classify_link_input("https://calendar.iris.to/events/today?view=week#selected");

    assert_eq!(app.kind, "iris_web");
    assert!(app.is_complete);
    assert!(app.is_valid);
    assert_eq!(app.open_display_name, "calendar");
    assert_eq!(
        app.local_open_url,
        format!(
            "http://calendar.{}.iris.localhost:17321/events/today?view=week#selected",
            crate::gateway::IRIS_SITES_PORTAL_NPUB
        )
    );

    let portal = classify_link_input("https://iris.to/?launcher=1");
    assert_eq!(portal.kind, "iris_web");
    assert!(portal.is_valid);
    assert_eq!(portal.open_display_name, "Iris Apps");
    assert_eq!(
        portal.local_open_url,
        format!(
            "http://sites.{}.iris.localhost:17321/?launcher=1",
            crate::gateway::IRIS_SITES_PORTAL_NPUB
        )
    );
}

#[test]
fn classify_local_iris_origin_stays_browser_only() {
    let local = classify_link_input("http://audio.npub1owner.iris.localhost:17321/album");

    assert_eq!(local.kind, "iris_web");
    assert!(local.is_valid);
    assert_eq!(
        local.local_open_url,
        "http://audio.npub1owner.iris.localhost:17321/album"
    );
}

#[test]
fn classify_public_iris_app_link_rejects_non_isolated_host_labels() {
    let nested = classify_link_input("https://admin.calendar.iris.to/");

    assert_eq!(nested.kind, "iris_web");
    assert!(nested.is_complete);
    assert!(!nested.is_valid);
    assert_eq!(nested.local_open_url.len(), 0);
    assert!(
        nested
            .error
            .contains("Iris app host is not an isolated app label")
    );
}

#[test]
fn resolve_app_key_link_target_accepts_invite_or_manual_profile_with_admin() {
    let profile_id = NostrIdentityId::new_v4();
    let admin = Keys::generate().public_key();
    let invite_key = Keys::generate().public_key();
    let invite = encode_app_key_link_invite(profile_id, &admin.to_hex(), &invite_key.to_hex())
        .expect("invite");

    let from_invite = resolve_app_key_link_target(&invite, None).expect("invite target");
    assert_eq!(from_invite.profile_id, profile_id);
    assert_eq!(from_invite.admin_app_key_hex, admin.to_hex());
    assert_eq!(from_invite.invite_pubkey, invite_key.to_hex());

    let from_manual = resolve_app_key_link_target(&profile_id.to_string(), Some(&admin.to_hex()))
        .expect("manual target");
    assert_eq!(from_manual.profile_id, profile_id);
    assert_eq!(from_manual.admin_app_key_hex, admin.to_hex());
    assert_eq!(from_manual.invite_pubkey.len(), 0);
}

#[test]
fn resolve_app_key_link_target_rejects_bare_app_key_as_identity() {
    let admin = Keys::generate().public_key();
    let error =
        resolve_app_key_link_target(&admin.to_bech32().expect("npub"), Some(&admin.to_hex()))
            .expect_err("bare app key is not an identity target");

    assert!(error.to_string().contains("NostrIdentity UUID"));
}
