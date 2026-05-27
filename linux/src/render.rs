#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn render_drives(list: &gtk::ListBox, json: &Value) {
    clear_list(list);
    let Some(drives) = json.get("drives").and_then(Value::as_array) else {
        list.append(&drive_row("main", &drive_mount_text(json), "-"));
        return;
    };
    if drives.is_empty() {
        list.append(&drive_row("main", &drive_mount_text(json), "-"));
        return;
    }
    for drive in drives {
        let name = find_string(drive, &["name", "drive_id"]).unwrap_or("main");
        let path = mounted_dir(json)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Not mounted".to_string());
        let status = find_string(drive, &["status", "root_cid"])
            .map(short_text)
            .unwrap_or_else(|| "configured".to_string());
        list.append(&drive_row(name, &path, &status));
    }
}

pub(crate) fn render_peers(model: &AppRef, json: &Value) {
    let list = &model.ui.peers;
    clear_list(list);
    let Some(peers) = json.get("peers").and_then(Value::as_array) else {
        list.append(&simple_row("No authorized devices", ""));
        return;
    };
    if peers.is_empty() {
        list.append(&simple_row("No authorized devices", ""));
        return;
    }
    let can_manage_devices = json
        .get("account")
        .and_then(|account| account.get("has_owner_signing_authority"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    for peer in peers {
        let title =
            find_string(peer, &["label", "device_npub", "device_pubkey"]).unwrap_or("Device");
        let device_npub = find_string(peer, &["device_npub"]).unwrap_or("");
        let is_current_device = peer
            .get("is_current_device")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut metadata = Vec::new();
        if is_current_device {
            metadata.push("this device".to_string());
        }
        if let Some(sync_state) = find_string(peer, &["sync_state"]) {
            metadata.push(sync_state.to_string());
        }
        if let Some(last_sync) = peer.get("last_block_sync")
            && let (Some(transport), Some(total)) = (
                find_string(last_sync, &["transport"]),
                find_number(last_sync, &["total_hashes"]),
            )
        {
            let fetched = find_number(last_sync, &["fetched"]).unwrap_or(0);
            metadata.push(format!("{transport} {fetched}/{total}"));
        }
        if let Some(root) = find_string(peer, &["root_cid"]) {
            metadata.push(short_text(root));
        }
        if let Some(generation) = find_number(peer, &["dck_generation"]) {
            metadata.push(format!("DCK {generation}"));
        }
        let fips_online = peer
            .get("fips_online")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let state = if fips_online { "Online" } else { "Offline" };
        list.append(&peer_row(
            model,
            title,
            &metadata.join(" | "),
            state,
            fips_online,
            device_npub,
            can_manage_devices && !is_current_device && !device_npub.is_empty(),
        ));
    }
}

pub(crate) fn render_backups(list: &gtk::ListBox, json: &Value) {
    clear_list(list);
    let network = json.get("network").unwrap_or(&Value::Null);
    if let Some(targets) = network.get("backup_targets").and_then(Value::as_array) {
        for target in targets {
            let kind = find_string(target, &["kind"]).unwrap_or("backup");
            let target_value = find_string(target, &["target"]).unwrap_or("");
            let title = find_string(target, &["label"])
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if kind == "fips" {
                        short_text(target_value)
                    } else {
                        target_value.to_string()
                    }
                });
            let last_sync = target.get("last_sync").unwrap_or(&Value::Null);
            let state = find_string(last_sync, &["state"]).unwrap_or(if kind == "fips" {
                "pending"
            } else {
                "ready"
            });
            let mut detail = if kind == "fips" {
                short_text(target_value)
            } else {
                target_value.to_string()
            };
            if let (Some(uploaded), Some(total)) = (
                find_number(last_sync, &["uploaded"]),
                find_number(last_sync, &["total_hashes"]),
            ) {
                detail = format!("{detail} | {uploaded}/{total}");
            }
            if let Some(check) = target.get("last_check").and_then(Value::as_object) {
                let check = Value::Object(check.clone());
                if let Some(check_state) = find_string(&check, &["state"]) {
                    detail = format!("{detail} | check {check_state}");
                }
                if let Some(latency) = find_number(&check, &["latency_ms"]) {
                    detail = format!("{detail} | {latency} ms");
                }
                if let Some(bytes_per_second) =
                    find_number(&check, &["download_bytes_per_second"])
                {
                    detail = format!("{detail} | {}/s", format_bytes(bytes_per_second));
                }
            }
            list.append(&simple_row(&title, &format!("{state} | {detail}")));
        }
    }
    if list.first_child().is_none() {
        list.append(&simple_row("No backup targets", ""));
    }
}

pub(crate) fn render_network(
    fips_list: &gtk::ListBox,
    relays_list: &gtk::ListBox,
    blossom_list: &gtk::ListBox,
    json: &Value,
) {
    clear_list(fips_list);
    clear_list(relays_list);
    clear_list(blossom_list);
    let network = json.get("network").unwrap_or(&Value::Null);
    render_fips_network(fips_list, network);

    if let Some(relays) = network.get("relays").and_then(Value::as_array) {
        for relay in relays.iter().filter_map(Value::as_str) {
            relays_list.append(&simple_row(relay, relay_status(relay, network)));
        }
    }
    if relays_list.first_child().is_none() {
        relays_list.append(&simple_row("No relays", ""));
    }

    if let Some(servers) = network.get("blossom_servers").and_then(Value::as_array) {
        for server in servers.iter().filter_map(Value::as_str) {
            blossom_list.append(&simple_row(server, ""));
        }
    }
    if blossom_list.first_child().is_none() {
        blossom_list.append(&simple_row("No Blossom servers", ""));
    }
}

pub(crate) fn render_fips_network(list: &gtk::ListBox, network: &Value) {
    let fips = network.get("fips").unwrap_or(&Value::Null);
    let state = fips_state_text(fips);
    let roster = format!(
        "{}/{} direct",
        find_number(fips, &["roster_connected_peer_count"]).unwrap_or(0),
        find_number(fips, &["roster_peer_count"]).unwrap_or(0)
    );
    let other = find_number(fips, &["other_peer_count"])
        .unwrap_or(0)
        .to_string();
    let connected = find_number(fips, &["connected_peer_count"])
        .unwrap_or(0)
        .to_string();
    list.append(&simple_row("State", &state));
    list.append(&simple_row("Roster FIPS", &roster));
    list.append(&simple_row("Other FIPS", &other));
    list.append(&simple_row("Connected", &connected));
    if let Some(endpoint) = find_string(fips, &["endpoint_npub"]).filter(|value| !value.is_empty())
    {
        list.append(&simple_row("Endpoint", endpoint));
    }
    if let Some(scope) = find_string(fips, &["discovery_scope"]).filter(|value| !value.is_empty()) {
        list.append(&simple_row("Scope", scope));
    }
    if let Some(error) = find_string(fips, &["error"]).filter(|value| !value.is_empty()) {
        list.append(&simple_row("Error", error));
    }
}

pub(crate) fn fips_state_text(fips: &Value) -> String {
    if find_string(fips, &["error"]).is_some_and(|error| !error.is_empty()) {
        return "Error".to_string();
    }
    let enabled = find_bool(fips, &["enabled"]).unwrap_or(false);
    let running = find_bool(fips, &["running"]).unwrap_or(false);
    let fresh = find_bool(fips, &["fresh"]).unwrap_or(false);
    if enabled && fresh {
        "Running".to_string()
    } else if enabled || running {
        "Stale".to_string()
    } else {
        "Stopped".to_string()
    }
}

pub(crate) fn relay_status<'a>(relay: &str, network: &'a Value) -> &'a str {
    network
        .get("relay_statuses")
        .and_then(Value::as_array)
        .and_then(|statuses| {
            statuses.iter().find_map(|status| {
                let url = status.get("url").and_then(Value::as_str)?;
                if url == relay {
                    status.get("status").and_then(Value::as_str)
                } else {
                    None
                }
            })
        })
        .unwrap_or("saved")
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

    if can_revoke {
        let revoke = icon_button("user-trash-symbolic", "Revoke device");
        let model = Rc::clone(model);
        let device_npub = device_npub.to_string();
        revoke.connect_clicked(move |_| match revoke_device(&device_npub) {
            Ok(()) => {
                restart_daemon(&model);
                model.ui.notice.set_text("Device revoked");
                refresh(&model);
            }
            Err(error) => model.ui.notice.set_text(&error),
        });
        body.append(&revoke);
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
