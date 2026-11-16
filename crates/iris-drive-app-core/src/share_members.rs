//! What a member row under a shared folder says, decided once here so every
//! shell shows the same thing: the person's name, then their role.

use iris_drive_core::{NostrIdentityId, ShareMemberStatus, SharedFolderMemberView};

use crate::state::UiShareMember;

/// Shown for a member who has not given a name.
const UNNAMED_MEMBER: &str = "Someone";

/// The person using this app, whose own row reads "You".
pub(crate) struct You<'a> {
    pub profile_id: NostrIdentityId,
    pub name: Option<&'a str>,
}

/// A shared folder's member rows as Google Drive lists people with access:
/// you first, then everyone by name, and those without a name last.
pub(crate) fn ui_share_members(
    members: Vec<SharedFolderMemberView>,
    you: Option<&You<'_>>,
) -> Vec<UiShareMember> {
    let mut rows = members
        .into_iter()
        .map(|member| {
            let is_you = you.is_some_and(|you| you.profile_id == member.profile_id);
            let unnamed = given_name(member.name.as_deref()).is_none();
            ((!is_you, unnamed), ui_share_member(member, you))
        })
        .collect::<Vec<_>>();
    rows.sort_by_cached_key(|(rank, row)| (*rank, row.name.to_lowercase()));
    rows.into_iter().map(|(_, row)| row).collect()
}

pub(crate) fn ui_share_member(
    member: SharedFolderMemberView,
    you: Option<&You<'_>>,
) -> UiShareMember {
    UiShareMember {
        name: member_name(&member, you),
        detail: member_detail(&member),
        profile_id: member.profile_id.to_string(),
        display_name: member.display_name,
        representative_npub_hint: member.representative_npub_hint.unwrap_or_default(),
        role: member.role.as_str().to_owned(),
        role_label: member.role.label().to_owned(),
        status: member.status.as_str().to_owned(),
        status_label: member.status.label().to_owned(),
        app_key_count: member.app_key_count as u64,
        can_revoke: member.can_revoke,
        can_change_role: member.can_change_role,
    }
}

fn member_name(member: &SharedFolderMemberView, you: Option<&You<'_>>) -> String {
    if let Some(you) = you.filter(|you| you.profile_id == member.profile_id) {
        return given_name(you.name)
            .map_or_else(|| "You".to_owned(), |name| format!("{name} (you)"));
    }
    given_name(member.name.as_deref())
        .unwrap_or(UNNAMED_MEMBER)
        .to_owned()
}

fn given_name(name: Option<&str>) -> Option<&str> {
    name.map(str::trim).filter(|name| !name.is_empty())
}

fn member_detail(member: &SharedFolderMemberView) -> String {
    let role = member.role.label();
    if member.status == ShareMemberStatus::Active {
        role.to_owned()
    } else {
        format!("{role} \u{b7} {}", member.status.label())
    }
}

#[cfg(test)]
mod tests {
    use iris_drive_core::{NostrIdentityId, ShareMemberStatus, ShareRole, SharedFolderMemberView};

    use super::{You, ui_share_member, ui_share_members};

    fn member(
        name: Option<&str>,
        role: ShareRole,
        status: ShareMemberStatus,
    ) -> SharedFolderMemberView {
        let profile_id = NostrIdentityId::new_v4();
        SharedFolderMemberView {
            profile_id,
            role,
            status,
            name: name.map(str::to_owned),
            display_name: name.map_or_else(|| profile_id.to_string(), str::to_owned),
            representative_npub_hint: None,
            app_key_count: 1,
            can_revoke: true,
            can_change_role: true,
        }
    }

    #[test]
    fn a_member_row_shows_their_name_then_role() {
        let row = ui_share_member(
            member(Some("Ada"), ShareRole::Editor, ShareMemberStatus::Active),
            None,
        );

        assert_eq!(row.name, "Ada");
        assert_eq!(row.detail, "Editor");
    }

    #[test]
    fn a_member_without_a_name_is_someone_not_an_id() {
        for name in [None, Some(""), Some("  ")] {
            let view = member(name, ShareRole::Reader, ShareMemberStatus::Active);
            let profile_id = view.profile_id.to_string();

            let row = ui_share_member(view, None);

            assert_eq!(row.name, "Someone");
            assert!(!row.detail.contains(&profile_id), "{}", row.detail);
        }
    }

    #[test]
    fn your_own_row_says_you_with_your_name_when_you_have_one() {
        let view = member(Some("Mac"), ShareRole::Admin, ShareMemberStatus::Active);
        let profile_id = view.profile_id;

        let unnamed = ui_share_member(
            view.clone(),
            Some(&You {
                profile_id,
                name: None,
            }),
        );
        let named = ui_share_member(
            view,
            Some(&You {
                profile_id,
                name: Some(" Ada "),
            }),
        );

        assert_eq!(unnamed.name, "You");
        assert_eq!(named.name, "Ada (you)");
        assert_eq!(named.detail, "Admin");
    }

    #[test]
    fn someone_else_is_not_you() {
        let view = member(Some("Bob"), ShareRole::Editor, ShareMemberStatus::Active);

        let row = ui_share_member(
            view,
            Some(&You {
                profile_id: NostrIdentityId::new_v4(),
                name: Some("Ada"),
            }),
        );

        assert_eq!(row.name, "Bob");
    }

    #[test]
    fn you_come_first_then_people_by_name_then_those_without_one() {
        let you = member(Some("Mac"), ShareRole::Admin, ShareMemberStatus::Active);
        let profile_id = you.profile_id;
        let members = vec![
            member(None, ShareRole::Reader, ShareMemberStatus::Active),
            member(Some("maya"), ShareRole::Editor, ShareMemberStatus::Active),
            you,
            member(Some("Bob"), ShareRole::Reader, ShareMemberStatus::Active),
        ];

        let rows = ui_share_members(
            members,
            Some(&You {
                profile_id,
                name: None,
            }),
        );

        let names: Vec<_> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["You", "Bob", "maya", "Someone"]);
    }

    #[test]
    fn the_detail_names_a_status_only_when_it_is_not_active() {
        let pending = ui_share_member(
            member(Some("Ada"), ShareRole::Reader, ShareMemberStatus::Pending),
            None,
        );
        let revoked = ui_share_member(
            member(Some("Bob"), ShareRole::Editor, ShareMemberStatus::Revoked),
            None,
        );

        assert_eq!(pending.detail, "Reader \u{b7} Pending");
        assert_eq!(revoked.detail, "Editor \u{b7} Revoked");
    }
}
