#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn render_drives(list: &gtk::ListBox, state: &NativeAppState) {
    clear_list(list);
    if state.ui.roots.is_empty() {
        list.append(&drive_row("main", &drive_mount_text(state), "-"));
        return;
    }
    for drive in &state.ui.roots {
        let name = if drive.name.is_empty() {
            "main"
        } else {
            &drive.name
        };
        let path = mounted_dir(state)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Not mounted".to_string());
        let status = if drive.status.is_empty() {
            "configured".to_string()
        } else {
            short_text(&drive.status)
        };
        list.append(&drive_row(name, &path, &status));
    }
}

pub(crate) fn render_peers(model: &AppRef, state: &NativeAppState) {
    let list = &model.ui.peers;
    clear_list(list);
    let app_actors = &state.ui.app_actors;
    let device_actors = app_actors
        .iter()
        .filter(|actor| actor.actor_kind == "device")
        .collect::<Vec<_>>();
    let recovery_key_actors = app_actors
        .iter()
        .filter(|actor| actor.actor_kind != "device")
        .collect::<Vec<_>>();
    if device_actors.is_empty() {
        list.append(&simple_row("No devices yet", ""));
    } else {
        for actor in device_actors {
            append_peer_actor_row(model, list, actor, true);
        }
    }
    if !recovery_key_actors.is_empty() {
        list.append(&simple_row(
            "Recovery Keys",
            &recovery_key_actors.len().to_string(),
        ));
        for actor in recovery_key_actors {
            append_peer_actor_row(model, list, actor, false);
        }
    }
}

pub(crate) fn render_add_device_section(model: &AppRef, state: &NativeAppState) {
    let account = profile(state);
    let requests = account
        .map(|account| account.inbound_app_key_link_requests.as_slice())
        .unwrap_or_default();
    let expander_label = match requests.len() {
        0 => "Add Device".to_string(),
        1 => "Add Device (1 request)".to_string(),
        count => format!("Add Device ({count} requests)"),
    };
    model.ui.add_device_expander.set_label(Some(&expander_label));

    clear_list(&model.ui.add_device_requests);
    model.ui.add_device_requests.set_visible(!requests.is_empty());
    for request in requests {
        model
            .ui
            .add_device_requests
            .append(&app_key_link_request_row(model, request));
    }
}

fn app_key_link_request_row(
    model: &AppRef,
    request: &iris_drive_app_core::state::UiAppKeyLinkRequest,
) -> gtk::ListBoxRow {
    let request_url = request.request_link.clone();
    let request_label = if request.label.is_empty() {
        "New device".to_string()
    } else {
        request.label.clone()
    };
    let request_device = request.app_key_pubkey.clone();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_valign(gtk::Align::Center);
    row.set_margin_top(8);
    row.set_margin_bottom(8);
    row.set_margin_start(10);
    row.set_margin_end(10);

    let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
    labels.set_hexpand(true);
    let title = gtk::Label::new(Some(&request_label));
    title.set_xalign(0.0);
    title.add_css_class("iris-row-title");
    labels.append(&title);
    let subtitle = gtk::Label::new(Some(&request_device));
    subtitle.set_xalign(0.0);
    subtitle.add_css_class("iris-row-subtitle");
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    labels.append(&subtitle);
    row.append(&labels);

    let reject = pill_button("Reject");
    reject.add_css_class("destructive-action");
    let add_request = primary_button("Add");
    {
        let model = Rc::clone(model);
        let request_url = request_url.clone();
        reject.connect_clicked(move |_| match reject_device(&request_url) {
            Ok(()) => {
                model.ui.notice.set_text("Device request rejected");
                refresh(&model);
            }
            Err(error) => model.ui.notice.set_text(&error),
        });
    }
    {
        let model = Rc::clone(model);
        let request_url = request_url.clone();
        let request_label = request_label.clone();
        add_request.connect_clicked(move |_| {
            approve_device_values(&model, request_url.clone(), request_label.clone());
        });
    }
    row.append(&reject);
    row.append(&add_request);

    let list_row = gtk::ListBoxRow::new();
    list_row.set_child(Some(&row));
    list_row
}

fn append_peer_actor_row(
    model: &AppRef,
    list: &gtk::ListBox,
    actor: &iris_drive_app_core::state::UiAppActor,
    show_status_dot: bool,
) {
    let title = if actor.display_label.is_empty() {
        "Device"
    } else {
        &actor.display_label
    };
    let app_key_pubkey = actor.pubkey.as_str();
    let mut metadata = Vec::new();
    if actor.is_current_app_key {
        metadata.push("this device".to_string());
        if !app_key_pubkey.is_empty() {
            metadata.push(format!("Device key: {app_key_pubkey}"));
        }
    }
    metadata.push(if actor.role_label.is_empty() {
        "Member".to_string()
    } else {
        actor.role_label.clone()
    });
    if !actor.state_label.is_empty() {
        metadata.push(actor.state_label.clone());
    }
    if !actor.detail.is_empty() {
        metadata.push(actor.detail.clone());
    }
    let connection = if actor.connection_label.is_empty() {
        "Offline"
    } else {
        &actor.connection_label
    };
    list.append(&peer_row(
        model,
        title,
        &metadata.join(" | "),
        connection,
        actor.is_online,
        show_status_dot,
        app_key_pubkey,
        actor.can_appoint_admin,
        actor.can_demote_admin,
        actor.can_revoke,
    ));
}

pub(crate) fn render_backups(model: &AppRef, state: &NativeAppState) {
    let list = &model.ui.backups;
    clear_list(list);
    for target in &state.ui.backups {
        let title = if target.label.is_empty() {
            "Backup"
        } else {
            &target.label
        };
        let subtitle = if target.state.is_empty() {
            target.detail.clone()
        } else if target.detail.is_empty() {
            target.state.clone()
        } else {
            format!("{} | {}", target.state, target.detail)
        };
        list.append(&backup_row(model, title, &subtitle, &target.target));
    }
    if list.first_child().is_none() {
        list.append(&simple_row("No backups configured", ""));
    }
}

pub(crate) fn render_shares(model: &AppRef, state: &NativeAppState) {
    let list = &model.ui.shares;
    clear_list(list);
    for share in &state.ui.shares {
        list.append(&share_row(model, state, share));
    }
    if list.first_child().is_none() {
        list.append(&simple_row("No shared folders", ""));
    }
}

fn share_row(
    model: &AppRef,
    state: &NativeAppState,
    share: &iris_drive_app_core::state::UiShare,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_selectable(false);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.set_valign(gtk::Align::Center);

    let icon = gtk::Image::from_icon_name("folder-publicshare-symbolic");
    header.append(&icon);

    let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
    labels.set_hexpand(true);
    let title = gtk::Label::new(Some(if share.display_name.is_empty() {
        "Shared folder"
    } else {
        &share.display_name
    }));
    title.add_css_class("iris-row-title");
    title.set_xalign(0.0);
    labels.append(&title);

    if !share.source_path.trim().is_empty() {
        labels.append(&share_detail_label(&share.source_path, 72));
    }

    let mut metadata = Vec::new();
    if !share.role_label.is_empty() {
        metadata.push(share.role_label.clone());
    }
    if !share.key_status_label.is_empty() {
        metadata.push(share.key_status_label.clone());
    }
    metadata.push(share_participant_text(share.participant_count));
    if !share.shortcut_paths.is_empty() {
        metadata.push(format!("My Drive {}", short_text(&share.shortcut_paths[0])));
    }
    labels.append(&share_detail_label(&metadata.join(" | "), 72));
    let repair_text = share_repair_text(share);
    if let Some(repair_text) = repair_text.as_deref() {
        labels.append(&share_detail_label(&repair_text, 72));
    }
    header.append(&labels);

    let open_path = share_open_path(share);
    if !open_path.is_empty() {
        let open = icon_button("folder-open-symbolic", "Open share folder");
        let model = Rc::clone(model);
        open.connect_clicked(move |_| open_share_folder(&model, open_path.clone()));
        header.append(&open);
    }

    if repair_text.is_some() {
        let repair = icon_button("emblem-synchronizing-symbolic", "Repair key wraps");
        let model = Rc::clone(model);
        let share_id = share.share_id.clone();
        repair.connect_clicked(move |_| repair_share_wraps(&model, share_id.clone()));
        header.append(&repair);
    }

    if share.shortcut_paths.is_empty() {
        let shortcut = icon_button("folder-new-symbolic", "Add to My Drive");
        let model = Rc::clone(model);
        let share_id = share.share_id.clone();
        shortcut.connect_clicked(move |_| {
            add_share_shortcut(&model, share_id.clone());
        });
        header.append(&shortcut);
    }

    if share.can_admin {
        let invite = icon_button("list-add-symbolic", "Invite member");
        let model = Rc::clone(model);
        let share_id = share.share_id.clone();
        let display_name = share.display_name.clone();
        invite.connect_clicked(move |_| {
            show_invite_share_member_dialog(&model, share_id.clone(), display_name.clone());
        });
        header.append(&invite);
    }

    let delete = icon_button("user-trash-symbolic", "Delete share");
    delete.add_css_class("destructive-action");
    let delete_model = Rc::clone(model);
    let share_id = share.share_id.clone();
    let display_name = if share.display_name.is_empty() {
        "Shared folder".to_string()
    } else {
        share.display_name.clone()
    };
    delete.connect_clicked(move |_| {
        show_delete_share_dialog(&delete_model, share_id.clone(), display_name.clone());
    });
    header.append(&delete);

    body.append(&header);

    let local_profile_id = state
        .ui
        .profile
        .as_ref()
        .map(|profile| profile.profile_id.as_str())
        .unwrap_or("");
    for member in &share.members {
        body.append(&share_member_row(
            &model,
            share,
            member,
            local_profile_id,
            share.can_admin,
        ));
    }
    for invite in &share.pending_invites {
        body.append(&pending_share_invite_row(invite));
    }

    row.set_child(Some(&body));
    row
}

fn share_open_path(share: &iris_drive_app_core::state::UiShare) -> String {
    share
        .shortcut_paths
        .first()
        .cloned()
        .filter(|path| !path.trim().is_empty())
        .or_else(|| (!share.source_path.trim().is_empty()).then(|| share.source_path.clone()))
        .or_else(|| {
            (!share.shared_with_me_path.trim().is_empty())
                .then(|| share.shared_with_me_path.clone())
        })
        .unwrap_or_default()
}

fn share_detail_label(text: &str, max_width_chars: i32) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("iris-row-subtitle");
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_max_width_chars(max_width_chars);
    label
}

fn share_participant_text(count: u64) -> String {
    let noun = if count == 1 { "person" } else { "people" };
    format!("{count} {noun}")
}

fn share_repair_text(share: &iris_drive_app_core::state::UiShare) -> Option<String> {
    if !share.repair_needed && share.missing_key_wrap_count == 0 {
        return None;
    }
    if share.missing_key_wrap_count > 0 {
        let plural = if share.missing_key_wrap_count == 1 {
            ""
        } else {
            "s"
        };
        return Some(format!(
            "{} missing access wrap{}",
            share.missing_key_wrap_count, plural
        ));
    }
    Some("Repair needed".to_string())
}

fn share_member_row(
    model: &AppRef,
    share: &iris_drive_app_core::state::UiShare,
    member: &iris_drive_app_core::state::UiShareMember,
    local_profile_id: &str,
    can_admin: bool,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_margin_start(28);
    row.set_valign(gtk::Align::Center);

    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let title = gtk::Label::new(Some(if member.display_name.is_empty() {
        "NostrIdentity"
    } else {
        &member.display_name
    }));
    title.add_css_class("iris-row-title");
    title.set_xalign(0.0);
    labels.append(&title);

    let mut metadata = Vec::new();
    if !member.role_label.is_empty() {
        metadata.push(member.role_label.clone());
    }
    if !member.status_label.is_empty() {
        metadata.push(member.status_label.clone());
    }
    if !member.representative_npub_hint.is_empty() {
        metadata.push(short_text(&member.representative_npub_hint));
    } else {
        metadata.push(short_text(&member.profile_id));
    }
    let subtitle = gtk::Label::new(Some(&metadata.join(" | ")));
    subtitle.add_css_class("iris-row-subtitle");
    subtitle.set_xalign(0.0);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    subtitle.set_max_width_chars(68);
    labels.append(&subtitle);
    row.append(&labels);

    if can_admin && member.status != "revoked" && member.profile_id != local_profile_id {
        let revoke = icon_button("user-trash-symbolic", "Revoke member");
        revoke.add_css_class("destructive-action");
        let model = Rc::clone(model);
        let share_id = share.share_id.clone();
        let profile_id = member.profile_id.clone();
        let display_name = if member.display_name.is_empty() {
            short_text(&member.profile_id)
        } else {
            member.display_name.clone()
        };
        revoke.connect_clicked(move |_| {
            show_revoke_share_member_dialog(
                &model,
                share_id.clone(),
                profile_id.clone(),
                display_name.clone(),
            );
        });
        row.append(&revoke);
    }

    row
}

fn pending_share_invite_row(invite: &iris_drive_app_core::state::UiPendingShareInvite) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_margin_start(28);
    row.set_valign(gtk::Align::Center);

    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let title = gtk::Label::new(Some(if invite.display_name.is_empty() {
        "Pending contact"
    } else {
        &invite.display_name
    }));
    title.add_css_class("iris-row-title");
    title.set_xalign(0.0);
    labels.append(&title);

    let mut metadata = Vec::new();
    if !invite.role_label.is_empty() {
        metadata.push(invite.role_label.clone());
    }
    if !invite.status_label.is_empty() {
        metadata.push(invite.status_label.clone());
    }
    metadata.push(short_text(&invite.representative_npub_hint));
    let subtitle = gtk::Label::new(Some(&metadata.join(" | ")));
    subtitle.add_css_class("iris-row-subtitle");
    subtitle.set_xalign(0.0);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    subtitle.set_max_width_chars(68);
    labels.append(&subtitle);
    row.append(&labels);

    row
}

fn backup_row(model: &AppRef, title: &str, subtitle: &str, target: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_selectable(false);
    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    outer.set_margin_top(10);
    outer.set_margin_bottom(10);
    outer.set_margin_start(12);
    outer.set_margin_end(12);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.set_hexpand(true);
    let title_label = gtk::Label::new(Some(title));
    title_label.set_xalign(0.0);
    title_label.add_css_class("iris-row-title");
    let subtitle_label = gtk::Label::new(Some(subtitle));
    subtitle_label.set_xalign(0.0);
    subtitle_label.add_css_class("iris-row-subtitle");
    subtitle_label.set_wrap(true);
    text.append(&title_label);
    text.append(&subtitle_label);
    outer.append(&text);

    let check = icon_button("emblem-default-symbolic", "Check");
    let target_for_check = target.to_string();
    let check_model = model.clone();
    check.connect_clicked(move |_| check_backup_target(&check_model, target_for_check.clone()));
    outer.append(&check);

    let remove = icon_button("user-trash-symbolic", "Remove backup");
    let target_for_remove = target.to_string();
    let remove_model = model.clone();
    remove.connect_clicked(move |_| remove_backup_target(&remove_model, target_for_remove.clone()));
    outer.append(&remove);

    row.set_child(Some(&outer));
    row
}

pub(crate) fn render_network(
    fips_list: &gtk::ListBox,
    relays_list: &gtk::ListBox,
    blossom_list: &gtk::ListBox,
    state: &NativeAppState,
) {
    clear_list(fips_list);
    clear_list(relays_list);
    clear_list(blossom_list);
    render_fips_network(fips_list, &state.ui);
    render_relay_statuses(relays_list, &state.ui);
    render_blossom_endpoints(blossom_list, &state.ui);

    if blossom_list.first_child().is_none() {
        blossom_list.append(&simple_row("No file servers", ""));
    }
}

fn render_blossom_endpoints(list: &gtk::ListBox, ui: &UiState) {
    for target in &ui.backups {
        if target.kind == "blossom" {
            list.append(&simple_row(&target.label, &target.target));
        }
    }
}

pub(crate) fn render_fips_network(list: &gtk::ListBox, ui: &UiState) {
    let fips = &ui.fips;
    let state = if fips.state_label.is_empty() {
        "Paused"
    } else {
        &fips.state_label
    };
    let roster = if fips.roster_label.is_empty() {
        "0/0 online"
    } else {
        &fips.roster_label
    };
    list.append(&simple_row("State", state));
    list.append(&simple_row("Roster FIPS", roster));
    list.append(&simple_row(
        "Other FIPS",
        &fips.other_peer_count.to_string(),
    ));
    list.append(&simple_row(
        "Direct FIPS",
        &fips.direct_device_count.to_string(),
    ));
    if !fips.endpoint_npub.is_empty() {
        list.append(&simple_row("Endpoint", &fips.endpoint_npub));
    }
    if !fips.discovery_scope.is_empty() {
        list.append(&simple_row("Scope", &fips.discovery_scope));
    }
    for peer in &fips.peer_statuses {
        let npub = if peer.npub.is_empty() {
            "peer"
        } else {
            &peer.npub
        };
        let label = if peer.connection_label.is_empty() {
            "Online"
        } else {
            &peer.connection_label
        };
        list.append(&simple_row(&format!("Peer {}", short_text(npub)), label));
    }
    if !fips.error.is_empty() {
        list.append(&simple_row("Error", &fips.error));
    }
}

fn render_relay_statuses(list: &gtk::ListBox, ui: &UiState) {
    for status in &ui.relay_statuses {
        let relay = if status.url.is_empty() {
            "relay"
        } else {
            &status.url
        };
        let label = if !status.status_label.is_empty() {
            &status.status_label
        } else if !status.status.is_empty() {
            &status.status
        } else {
            "saved"
        };
        list.append(&simple_row(relay, label));
    }
    if list.first_child().is_none() {
        list.append(&simple_row("No relays", ""));
    }
}

pub(crate) fn clear_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

pub(crate) fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

pub(crate) fn simple_row(title: &str, subtitle: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 3);
    body.set_margin_top(10);
    body.set_margin_bottom(10);
    body.set_margin_start(12);
    body.set_margin_end(12);

    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("iris-row-title");
    title_label.set_xalign(0.0);
    title_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    title_label.set_max_width_chars(44);
    body.append(&title_label);

    if !subtitle.is_empty() {
        let subtitle_label = gtk::Label::new(Some(subtitle));
        subtitle_label.add_css_class("iris-row-subtitle");
        subtitle_label.set_xalign(0.0);
        subtitle_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        subtitle_label.set_max_width_chars(44);
        body.append(&subtitle_label);
    }

    row.set_child(Some(&body));
    row
}

pub(crate) fn peer_row(
    model: &AppRef,
    title: &str,
    subtitle: &str,
    state: &str,
    is_online: bool,
    show_status_dot: bool,
    app_key_pubkey: &str,
    can_appoint_admin: bool,
    can_demote_admin: bool,
    can_revoke: bool,
) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    body.set_valign(gtk::Align::Center);
    body.set_margin_top(10);
    body.set_margin_bottom(10);
    body.set_margin_start(12);
    body.set_margin_end(12);

    if show_status_dot {
        let dot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        dot.add_css_class(if is_online {
            "iris-peer-online"
        } else {
            "iris-peer-offline"
        });
        dot.set_valign(gtk::Align::Center);
        dot.set_tooltip_text(Some(state));
        body.append(&dot);
    }

    let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
    labels.set_hexpand(true);
    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("iris-row-title");
    title_label.set_xalign(0.0);
    title_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    title_label.set_max_width_chars(44);
    labels.append(&title_label);

    if !subtitle.is_empty() {
        let subtitle_label = gtk::Label::new(Some(subtitle));
        subtitle_label.add_css_class("iris-row-subtitle");
        subtitle_label.set_xalign(0.0);
        subtitle_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        subtitle_label.set_max_width_chars(44);
        labels.append(&subtitle_label);
    }
    body.append(&labels);

    if can_appoint_admin {
        let appoint = icon_button("contact-new-symbolic", "Make admin");
        let model = Rc::clone(model);
        let app_key_pubkey = app_key_pubkey.to_string();
        appoint.connect_clicked(move |_| match appoint_admin(&app_key_pubkey) {
            Ok(()) => {
                model.ui.notice.set_text("Device made admin");
                refresh(&model);
            }
            Err(error) => model.ui.notice.set_text(&error),
        });
        body.append(&appoint);
    }

    if can_demote_admin {
        let demote = icon_button("changes-prevent-symbolic", "Remove admin");
        let model = Rc::clone(model);
        let app_key_pubkey = app_key_pubkey.to_string();
        demote.connect_clicked(move |_| match demote_admin(&app_key_pubkey) {
            Ok(()) => {
                model.ui.notice.set_text("Admin removed");
                refresh(&model);
            }
            Err(error) => model.ui.notice.set_text(&error),
        });
        body.append(&demote);
    }

    if can_revoke {
        let delete = icon_button("user-trash-symbolic", "Remove Device");
        let model = Rc::clone(model);
        let app_key_pubkey = app_key_pubkey.to_string();
        let title = title.to_string();
        delete.connect_clicked(move |_| {
            show_delete_device_dialog(&model, app_key_pubkey.clone(), title.clone());
        });
        body.append(&delete);
    }

    row.set_child(Some(&body));
    row
}

pub(crate) fn drive_row(name: &str, path: &str, status: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    body.set_margin_top(10);
    body.set_margin_bottom(10);
    body.set_margin_start(12);
    body.set_margin_end(12);

    let icon = gtk::Image::from_icon_name("folder-symbolic");
    body.append(&icon);

    let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
    labels.set_hexpand(true);
    let title = gtk::Label::new(Some(name));
    title.add_css_class("iris-row-title");
    title.set_xalign(0.0);
    let subtitle = gtk::Label::new(Some(path));
    subtitle.add_css_class("iris-row-subtitle");
    subtitle.set_xalign(0.0);
    subtitle.set_hexpand(true);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    subtitle.set_max_width_chars(44);
    labels.append(&title);
    labels.append(&subtitle);
    body.append(&labels);

    let state = gtk::Label::new(Some(status));
    state.add_css_class("iris-row-state");
    state.set_xalign(1.0);
    body.append(&state);

    row.set_child(Some(&body));
    row
}
