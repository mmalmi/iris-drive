use serde::{Deserialize, Serialize};

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinkInputClassification {
    pub kind: String,
    pub is_complete: bool,
    pub is_valid: bool,
    pub normalized_input: String,
    pub app_key_pubkey: String,
    pub admin_app_key_pubkey: String,
    pub has_link_secret: bool,
    pub share_source_path: String,
    pub share_display_name: String,
    pub share_recipient_npub_hint: String,
    pub share_recipient_display_name: String,
    pub share_recipient_profile_id: String,
    pub content_nhash: String,
    pub content_path_hint: String,
    pub open_display_name: String,
    pub local_open_url: String,
    pub error: String,
}

impl From<iris_drive_core::LinkInputClassification> for LinkInputClassification {
    fn from(value: iris_drive_core::LinkInputClassification) -> Self {
        Self {
            kind: value.kind,
            is_complete: value.is_complete,
            is_valid: value.is_valid,
            normalized_input: value.normalized_input,
            app_key_pubkey: value.app_key_pubkey,
            admin_app_key_pubkey: value.admin_app_key_pubkey,
            has_link_secret: value.has_link_secret,
            share_source_path: value.share_source_path,
            share_display_name: value.share_display_name,
            share_recipient_npub_hint: value.share_recipient_npub_hint,
            share_recipient_display_name: value.share_recipient_display_name,
            share_recipient_profile_id: value.share_recipient_profile_id,
            content_nhash: value.content_nhash,
            content_path_hint: value.content_path_hint,
            open_display_name: value.open_display_name,
            local_open_url: value.local_open_url,
            error: value.error,
        }
    }
}

#[uniffi::export]
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn classify_link_input(input: String) -> LinkInputClassification {
    iris_drive_core::classify_link_input(&input).into()
}

#[uniffi::export]
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn validate_link_input(input: String) -> LinkInputClassification {
    let mut classification: LinkInputClassification =
        iris_drive_core::classify_link_input(&input).into();
    if !matches!(
        classification.kind.as_str(),
        "invite" | "app_key_pubkey" | "app_key_approval"
    ) {
        classification.is_complete = false;
        classification.is_valid = false;
    }
    classification
}

#[cfg(test)]
mod tests {
    use super::{classify_link_input, validate_link_input};

    #[test]
    fn classify_nhash_file_exposes_native_open_target() {
        let file = classify_link_input(
            "https://drive.iris.to/#/nhash1qqsyktrn6c5r444rhjt2qfv6a6uu5hcsrlcvk202whqhxyk3fwkl83s9yr8ngvg5489t2sqnpzqyk7um2ug688j42y57375qex7vgpc384vdv9mr60t/freenet.pdf?fullscreen=1".to_owned(),
        );

        assert_eq!(file.kind, "nhash_file");
        assert!(file.is_valid);
        assert_eq!(file.open_display_name, "freenet.pdf");
        assert!(file.local_open_url.ends_with("/freenet.pdf"));
    }

    #[test]
    fn validate_link_input_does_not_accept_browser_only_iris_links() {
        let browser = validate_link_input("https://calendar.iris.to/".to_owned());

        assert_eq!(browser.kind, "iris_web");
        assert!(!browser.is_complete);
        assert!(!browser.is_valid);
    }
}
