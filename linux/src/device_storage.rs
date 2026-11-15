//! The Devices page's Storage section: each of the profile's devices and how
//! much of the Drive it holds, as Iris Drive web's Settings > Storage shows it.
//! The app core words all of it (`ui.device_storage`); this only lays it out.

#[allow(clippy::wildcard_imports)]
use super::*;
use std::collections::BTreeSet;

use iris_drive_app_core::state::{UiDeviceStorage, UiDeviceStorageFolder, UiDeviceStorageRow};

/// Folder details start under the row's text, past the 16px icon and its gap.
const TEXT_COLUMN: i32 = 28;
const FOLDER_INDENT: i32 = 12;

#[derive(Clone)]
pub(crate) struct DeviceStorageSection {
    pub(crate) root: gtk::Box,
    summary: gtk::Label,
    list: gtk::ListBox,
    /// What the list shows now, so an unchanged refresh leaves it alone.
    shown: Rc<RefCell<Option<UiDeviceStorage>>>,
    rows: Rc<RefCell<Vec<StorageRowParts>>>,
    /// Devices whose folders are open; kept across refreshes.
    open: Rc<RefCell<BTreeSet<String>>>,
}

struct StorageRowParts {
    pubkey: String,
    row: gtk::ListBoxRow,
    /// The folder details and the chevron, for a device that has reported.
    details: Option<(gtk::Revealer, gtk::Image)>,
}

pub(crate) fn device_storage_section() -> DeviceStorageSection {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_visible(false);
    root.append(&field_title("Storage"));
    let summary = gtk::Label::new(None);
    summary.add_css_class("dim-label");
    summary.set_xalign(0.0);
    summary.set_wrap(true);
    root.append(&summary);
    let list = gtk::ListBox::new();
    list.add_css_class("iris-drive-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    root.append(&list);

    let section = DeviceStorageSection {
        root,
        summary,
        list,
        shown: Rc::default(),
        rows: Rc::default(),
        open: Rc::default(),
    };
    {
        let rows = Rc::clone(&section.rows);
        let open = Rc::clone(&section.open);
        section.list.connect_row_activated(move |_, activated| {
            let rows = rows.borrow();
            let Some(parts) = rows.iter().find(|parts| parts.row == *activated) else {
                return;
            };
            let Some((revealer, chevron)) = &parts.details else {
                return;
            };
            let expanded = !revealer.reveals_child();
            set_open(&parts.row, revealer, chevron, expanded);
            let mut open = open.borrow_mut();
            if expanded {
                open.insert(parts.pubkey.clone());
            } else {
                open.remove(&parts.pubkey);
            }
        });
    }
    section
}

pub(crate) fn render_device_storage(section: &DeviceStorageSection, state: &NativeAppState) {
    let storage = &state.ui.device_storage;
    if section.shown.borrow().as_ref() == Some(storage) {
        return;
    }
    section.root.set_visible(!storage.devices.is_empty());
    section.summary.set_text(&storage.summary);
    section.summary.set_visible(!storage.summary.is_empty());
    clear_list(&section.list);
    let rows: Vec<StorageRowParts> = {
        let open = section.open.borrow();
        storage
            .devices
            .iter()
            .map(|device| {
                let parts = storage_row(device, open.contains(&device.pubkey));
                section.list.append(&parts.row);
                parts
            })
            .collect()
    };
    *section.rows.borrow_mut() = rows;
    *section.shown.borrow_mut() = Some(storage.clone());
}

pub(crate) fn clear_device_storage(section: &DeviceStorageSection) {
    section.root.set_visible(false);
    clear_list(&section.list);
    section.rows.borrow_mut().clear();
    section.shown.borrow_mut().take();
}

fn storage_row(device: &UiDeviceStorageRow, open: bool) -> StorageRowParts {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(device.has_report);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body.set_margin_top(10);
    body.set_margin_bottom(10);
    body.set_margin_start(12);
    body.set_margin_end(12);

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let icon = gtk::Image::from_icon_name(device_icon_name(&device.icon));
    icon.set_valign(gtk::Align::Start);
    icon.set_margin_top(2);
    top.append(&icon);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 6);
    text.set_hexpand(true);
    text.append(&storage_row_header(device));
    if !device.is_current {
        text.append(&storage_row_status(device));
    }
    if device.has_report {
        text.append(&storage_bar(device.held_permille, "iris-storage-bar"));
        text.append(&storage_row_space(device));
    } else if device.is_current && !device.age_label.is_empty() {
        text.append(&small_label(&device.age_label, "dim-label"));
    }
    top.append(&text);
    body.append(&top);

    let details = device.has_report.then(|| {
        let chevron = gtk::Image::from_icon_name("pan-end-symbolic");
        chevron.add_css_class("dim-label");
        chevron.set_valign(gtk::Align::Start);
        chevron.set_margin_top(2);
        top.append(&chevron);
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        revealer.set_child(Some(&storage_row_folders(device)));
        body.append(&revealer);
        set_open(&row, &revealer, &chevron, open);
        (revealer, chevron)
    });

    row.set_child(Some(&body));
    StorageRowParts {
        pubkey: device.pubkey.clone(),
        row,
        details,
    }
}

fn set_open(row: &gtk::ListBoxRow, revealer: &gtk::Revealer, chevron: &gtk::Image, open: bool) {
    revealer.set_reveal_child(open);
    chevron.set_icon_name(Some(if open {
        "pan-down-symbolic"
    } else {
        "pan-end-symbolic"
    }));
    row.update_state(&[gtk::accessible::State::Expanded(Some(open))]);
}

fn storage_row_header(device: &UiDeviceStorageRow) -> gtk::Box {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = gtk::Label::new(Some(&device.name));
    name.add_css_class("iris-row-title");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    header.append(&name);
    if device.is_current {
        header.append(&small_label("This device", "dim-label"));
    }
    let held = gtk::Label::new(Some(&device.held_label));
    held.add_css_class(if device.holds_all {
        "success"
    } else {
        "dim-label"
    });
    held.set_hexpand(true);
    held.set_xalign(1.0);
    header.append(&held);
    header
}

fn storage_row_status(device: &UiDeviceStorageRow) -> gtk::Box {
    let status = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let dot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    dot.add_css_class(if device.connected {
        "iris-peer-online"
    } else {
        "iris-peer-offline"
    });
    dot.set_valign(gtk::Align::Center);
    status.append(&dot);
    let connection = small_label(&device.connection_label, "dim-label");
    set_hint(&connection, &device.connection_detail);
    status.append(&connection);
    if !device.sync_label.is_empty() {
        status.append(&small_label("·", "dim-label"));
        let sync = small_label(&device.sync_label, sync_tone(&device.sync_state));
        set_hint(&sync, &device.sync_detail);
        status.append(&sync);
    }
    status
}

fn storage_row_space(device: &UiDeviceStorageRow) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let space = small_label(&device.space_label, "dim-label");
    space.set_hexpand(true);
    space.set_ellipsize(gtk::pango::EllipsizeMode::End);
    set_hint(&space, &device.space_detail);
    line.append(&space);
    let age = small_label(&device.age_label, "dim-label");
    age.set_xalign(1.0);
    line.append(&age);
    line
}

fn storage_row_folders(device: &UiDeviceStorageRow) -> gtk::Box {
    let folders = gtk::Box::new(gtk::Orientation::Vertical, 8);
    folders.set_margin_top(10);
    folders.set_margin_start(TEXT_COLUMN);
    for folder in &device.folders {
        folders.append(&storage_folder_line(folder));
    }
    for note in &device.notes {
        let note = small_label(note, "dim-label");
        note.set_wrap(true);
        folders.append(&note);
    }
    folders
}

fn storage_folder_line(folder: &UiDeviceStorageFolder) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Vertical, 3);
    line.set_margin_start(
        i32::try_from(folder.depth).map_or(0, |depth| depth.saturating_mul(FOLDER_INDENT)),
    );
    let labels = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(&folder.label));
    label.add_css_class("caption");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    labels.append(&label);
    let detail = small_label(&folder.detail, "dim-label");
    detail.set_xalign(1.0);
    labels.append(&detail);
    line.append(&labels);
    line.append(&storage_bar(
        folder.held_permille,
        "iris-storage-folder-bar",
    ));
    line
}

/// A thin bar filled to `permille / 1000`; the label beside it says the same.
fn storage_bar(permille: u32, css_class: &str) -> gtk::ProgressBar {
    let bar = gtk::ProgressBar::new();
    bar.add_css_class(css_class);
    bar.set_hexpand(true);
    bar.set_fraction(f64::from(permille.min(1000)) / 1000.0);
    bar.update_state(&[gtk::accessible::State::Hidden(true)]);
    bar
}

fn small_label(text: &str, tone: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("caption");
    label.add_css_class(tone);
    label.set_xalign(0.0);
    label
}

fn set_hint(label: &gtk::Label, detail: &str) {
    if !detail.is_empty() {
        label.set_tooltip_text(Some(detail));
    }
}

fn device_icon_name(icon: &str) -> &'static str {
    match icon {
        "web" => "web-browser-symbolic",
        "phone" => "phone-symbolic",
        "computer" => "computer-symbolic",
        _ => "drive-harddisk-symbolic",
    }
}

fn sync_tone(sync_state: &str) -> &'static str {
    match sync_state {
        "in-sync" => "success",
        "behind" | "apart" => "warning",
        _ => "dim-label",
    }
}
