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
    if state.ui.devices.is_empty() {
        list.append(&simple_row("No AppKeys", ""));
        return;
    }
    for peer in &state.ui.devices {
        let title = if peer.display_label.is_empty() {
            "AppKey"
        } else {
            &peer.display_label
        };
        let device_npub = peer.pubkey.as_str();
        let mut metadata = Vec::new();
        if peer.is_current_device {
            metadata.push("this AppKey".to_string());
            if !device_npub.is_empty() {
                metadata.push(format!("AppKey: {device_npub}"));
            }
        }
        metadata.push(if peer.role_label.is_empty() {
            "Member".to_string()
        } else {
            peer.role_label.clone()
        });
        if !peer.state_label.is_empty() {
            metadata.push(peer.state_label.clone());
        }
        if !peer.detail.is_empty() {
            metadata.push(peer.detail.clone());
        }
        let connection = if peer.connection_label.is_empty() {
            "Offline"
        } else {
            &peer.connection_label
        };
        list.append(&peer_row(
            model,
            title,
            &metadata.join(" | "),
            connection,
            peer.is_online,
            device_npub,
            peer.can_appoint_admin,
            peer.can_demote_admin,
            peer.can_revoke,
        ));
    }
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
        list.append(&simple_row("No backup targets", ""));
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

    let mut metadata = Vec::new();
    if !share.role_label.is_empty() {
        metadata.push(share.role_label.clone());
    }
    if !share.key_status_label.is_empty() {
        metadata.push(share.key_status_label.clone());
    }
    metadata.push(format!("{} people", share.participant_count));
    if !share.shortcut_paths.is_empty() {
        metadata.push(format!("shortcut {}", short_text(&share.shortcut_paths[0])));
    }
    let subtitle = gtk::Label::new(Some(&metadata.join(" | ")));
    subtitle.add_css_class("iris-row-subtitle");
    subtitle.set_xalign(0.0);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    subtitle.set_max_width_chars(72);
    labels.append(&subtitle);
    header.append(&labels);

    if share.repair_needed || !share.missing_key_wraps.is_empty() {
        let repair = icon_button("emblem-synchronizing-symbolic", "Repair key wraps");
        let model = Rc::clone(model);
        let share_id = share.share_id.clone();
        repair.connect_clicked(move |_| repair_share_wraps(&model, share_id.clone()));
        header.append(&repair);
    }

    if share.shortcut_paths.is_empty() {
        let shortcut = icon_button("emblem-symbolic-link-symbolic", "Add shortcut");
        let model = Rc::clone(model);
        let share_id = share.share_id.clone();
        let display_name = share.display_name.clone();
        shortcut.connect_clicked(move |_| {
            add_share_shortcut(&model, share_id.clone(), display_name.clone());
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

    body.append(&header);

    let local_profile_id = state
        .ui
        .profile
        .as_ref()
        .map(|profile| profile.profile_id.as_str())
        .unwrap_or("");
    for member in &share.members {
        body.append(&share_member_row(
            model,
            share,
            member,
            local_profile_id,
            share.can_admin,
        ));
    }

    row.set_child(Some(&body));
    row
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
        "IrisProfile"
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
        blossom_list.append(&simple_row("No Blossom servers", ""));
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
    list.append(&simple_row("Other FIPS", &fips.other_peer_count.to_string()));
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
    device_npub: &str,
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

    let dot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    dot.add_css_class(if is_online {
        "iris-peer-online"
    } else {
        "iris-peer-offline"
    });
    dot.set_valign(gtk::Align::Center);
    dot.set_tooltip_text(Some(state));
    body.append(&dot);

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
        let device_npub = device_npub.to_string();
        appoint.connect_clicked(move |_| match appoint_admin(&device_npub) {
            Ok(()) => {
                restart_daemon(&model);
                model.ui.notice.set_text("AppKey made admin");
                refresh(&model);
            }
            Err(error) => model.ui.notice.set_text(&error),
        });
        body.append(&appoint);
    }

    if can_demote_admin {
        let demote = icon_button("changes-prevent-symbolic", "Remove admin");
        let model = Rc::clone(model);
        let device_npub = device_npub.to_string();
        demote.connect_clicked(move |_| match demote_admin(&device_npub) {
            Ok(()) => {
                restart_daemon(&model);
                model.ui.notice.set_text("Admin removed");
                refresh(&model);
            }
            Err(error) => model.ui.notice.set_text(&error),
        });
        body.append(&demote);
    }

    if can_revoke {
        let delete = icon_button("user-trash-symbolic", "Remove AppKey");
        let model = Rc::clone(model);
        let device_npub = device_npub.to_string();
        let title = title.to_string();
        delete.connect_clicked(move |_| {
            show_delete_device_dialog(&model, device_npub.clone(), title.clone());
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
