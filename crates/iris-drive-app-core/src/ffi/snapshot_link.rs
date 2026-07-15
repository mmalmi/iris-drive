use hashtree_core::{Cid, NHashData, nhash_encode_full};
use iris_drive_core::AppConfig;

use crate::state::NativeAppState;

use super::DriveLinkForCid;

pub(super) fn drive_link_for_cid_value(root_cid: &str) -> DriveLinkForCid {
    match drive_iris_to_nhash_url_for_root(root_cid) {
        Some(url) => DriveLinkForCid {
            url,
            error: String::new(),
        },
        None => DriveLinkForCid {
            error: "invalid content id".to_owned(),
            ..DriveLinkForCid::default()
        },
    }
}

pub(super) fn update_snapshot_link(state: &mut NativeAppState, config: &AppConfig) {
    state.ui.snapshot_link = current_primary_root_cid(config)
        .and_then(|root| drive_iris_to_nhash_url_for_root(&root))
        .unwrap_or_default();
}

fn current_primary_root_cid(config: &AppConfig) -> Option<String> {
    config
        .profile
        .as_ref()
        .and_then(|account| {
            config
                .drive(iris_drive_core::PRIMARY_DRIVE_ID)
                .and_then(|drive| drive.app_key_roots.get(&account.app_key_pubkey))
                .map(|root| root.root_cid.clone())
        })
        .or_else(|| {
            config
                .drive(iris_drive_core::PRIMARY_DRIVE_ID)
                .and_then(|drive| drive.last_root_cid.clone())
        })
}

fn drive_iris_to_nhash_url_for_root(root_cid: &str) -> Option<String> {
    let cid = Cid::parse(root_cid).ok()?;
    let nhash = nhash_encode_full(&NHashData {
        hash: cid.hash,
        decrypt_key: cid.key,
    })
    .ok()?;
    Some(format!("https://drive.iris.to/#/{nhash}"))
}
