#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("flat");
    button.set_size_request(32, 32);
    button.set_tooltip_text(Some(tooltip));
    button
}

pub(crate) fn text_button(label: &str) -> gtk::Button {
    gtk::Button::with_label(label)
}

pub(crate) fn action_button(icon: &str, label: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_tooltip_text(Some(tooltip));
    let content = adw::ButtonContent::builder()
        .icon_name(icon)
        .label(label)
        .build();
    button.set_child(Some(&content));
    button
}

pub(crate) fn flow_section(css_class: &str, min_children: u32, max_children: u32) -> gtk::FlowBox {
    let flow = gtk::FlowBox::new();
    flow.add_css_class(css_class);
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_activate_on_single_click(false);
    flow.set_min_children_per_line(min_children);
    flow.set_max_children_per_line(max_children);
    flow.set_column_spacing(12);
    flow.set_row_spacing(12);
    flow.set_hexpand(true);
    flow
}

pub(crate) fn sidebar_button(icon: &str, label: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("iris-sidebar-button");
    button.set_halign(gtk::Align::Fill);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    row.set_hexpand(true);
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(16);
    row.append(&image);
    let text = gtk::Label::new(Some(label));
    text.set_xalign(0.0);
    text.set_hexpand(true);
    row.append(&text);
    button.set_child(Some(&row));
    button
}

pub(crate) fn update_sidebar_selection(stack: &gtk::Stack, buttons: &[(String, gtk::Button)]) {
    let visible = stack.visible_child_name();
    for (name, button) in buttons {
        if visible.as_deref() == Some(name.as_str()) {
            button.add_css_class("selected");
        } else {
            button.remove_css_class("selected");
        }
    }
}

pub(crate) fn page_box() -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_hexpand(true);
    page.set_vexpand(true);
    page
}

pub(crate) fn section_title(title: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(title));
    label.add_css_class("iris-section-title");
    label.set_xalign(0.0);
    label
}

pub(crate) fn field_title(title: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(title));
    label.add_css_class("iris-field-name");
    label.set_xalign(0.0);
    label
}

pub(crate) fn pill_button(label: &str) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.add_css_class("pill");
    button.set_height_request(44);
    button
}

pub(crate) fn primary_button(label: &str) -> gtk::Button {
    let button = pill_button(label);
    button.add_css_class("suggested-action");
    button
}

pub(crate) fn setup_entry(placeholder: &str) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(placeholder));
    entry.set_height_request(40);
    entry
}

pub(crate) fn readonly_entry(value: &str) -> gtk::Entry {
    let entry = setup_entry("");
    entry.set_text(value);
    entry.set_editable(false);
    entry.set_hexpand(true);
    entry
}

pub(crate) fn value_label() -> gtk::Label {
    let label = gtk::Label::new(Some("..."));
    label.add_css_class("iris-value");
    label.set_xalign(0.0);
    label.set_selectable(true);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::Char);
    label.set_max_width_chars(44);
    label
}

pub(crate) fn metric_value_label() -> gtk::Label {
    let label = gtk::Label::new(Some("0"));
    label.add_css_class("iris-metric-value");
    label.set_xalign(0.0);
    label
}

pub(crate) fn metric_tile(title: &str, value: &gtk::Label) -> gtk::Box {
    let tile = gtk::Box::new(gtk::Orientation::Vertical, 7);
    tile.add_css_class("iris-metric-card");
    tile.set_hexpand(true);
    tile.set_width_request(150);
    tile.set_margin_top(0);
    tile.set_margin_bottom(0);

    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("iris-field-name");
    title_label.set_xalign(0.0);
    tile.append(&title_label);
    tile.append(value);
    tile
}

pub(crate) fn endpoint_group(title: &str, list: &gtk::ListBox) -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let title_label = gtk::Label::new(Some(title));
    title_label.add_css_class("iris-field-name");
    title_label.set_xalign(0.0);
    group.append(&title_label);
    group.append(list);
    group
}

pub(crate) fn close_to_tray_enabled(model: &AppRef) -> bool {
    !model.quit_requested.get() && model.tray_available.get() && model.ui.tray_on_close.is_active()
}

pub(crate) fn add_field(grid: &gtk::Grid, row: i32, column: i32, name: &str, value: &gtk::Label) {
    let label = gtk::Label::new(Some(name));
    label.add_css_class("iris-field-name");
    label.set_xalign(0.0);
    grid.attach(&label, column * 2, row, 1, 1);
    grid.attach(value, column * 2 + 1, row, 1, 1);
}

pub(crate) fn add_copy_field(
    grid: &gtk::Grid,
    row: i32,
    name: &str,
    value: &gtk::Label,
    button: &gtk::Button,
) {
    let label = gtk::Label::new(Some(name));
    label.add_css_class("iris-field-name");
    label.set_xalign(0.0);
    grid.attach(&label, 0, row, 1, 1);
    value.set_hexpand(true);
    grid.attach(value, 1, row, 1, 1);
    grid.attach(button, 2, row, 1, 1);
}
