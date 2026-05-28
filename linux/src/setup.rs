#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn render_setup(model: &AppRef) {
    clear_box(&model.ui.setup);
    match *model.setup_screen.borrow() {
        SetupScreen::Welcome => render_setup_welcome(model),
        SetupScreen::Create => render_create_profile(model),
        SetupScreen::CreatePhoto => render_create_profile_photo(model),
        SetupScreen::Restore => render_restore_profile(model),
        SetupScreen::Link => render_link_device(model),
    }
}

pub(crate) fn render_awaiting_approval(model: &AppRef, json: &Value, sync_running: bool) {
    clear_box(&model.ui.setup);

    let container = gtk::Box::new(gtk::Orientation::Vertical, 14);
    container.set_halign(gtk::Align::Center);
    container.set_valign(gtk::Align::Center);
    container.set_width_request(420);

    let header = gtk::Label::new(Some("Awaiting approval"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    let account = account_json(json);
    let owner = readonly_entry(find_string(account, &["owner_npub"]).unwrap_or("-"));
    container.append(&field_title("Owner"));
    container.append(&owner);

    let device = readonly_entry(find_string(account, &["device_npub"]).unwrap_or("-"));
    container.append(&field_title("This device"));
    container.append(&device);

    let request = find_string(
        account.get("device_link_request").unwrap_or(&Value::Null),
        &["url"],
    )
    .unwrap_or("");
    let request_entry = readonly_entry(request);
    request_entry.set_height_request(82);
    container.append(&field_title("Request link"));
    container.append(&request_entry);

    let notice = setup_notice();
    notice.set_text(if sync_running {
        "Waiting for approval"
    } else {
        "Sync stopped"
    });

    let copy = primary_button("Copy request");
    {
        let request = request.to_string();
        let notice = notice.clone();
        copy.connect_clicked(move |_| {
            if request.is_empty() {
                notice.set_text("Nothing to copy");
            } else if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&request);
                notice.set_text("Request copied");
            } else {
                notice.set_text("Clipboard unavailable");
            }
        });
    }
    container.append(&copy);
    container.append(&notice);

    append_centered_setup(model, &container);
}

pub(crate) fn setup_container(model: &AppRef, title: &str) -> gtk::Box {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 14);
    container.set_halign(gtk::Align::Center);
    container.set_valign(gtk::Align::Center);
    container.set_width_request(340);

    let back = gtk::Button::new();
    back.add_css_class("flat");
    back.set_tooltip_text(Some("Back"));
    back.set_halign(gtk::Align::Start);
    let back_content = adw::ButtonContent::builder()
        .icon_name("go-previous-symbolic")
        .label("Back")
        .build();
    back.set_child(Some(&back_content));
    {
        let model = Rc::clone(model);
        back.connect_clicked(move |_| {
            let target = if *model.setup_screen.borrow() == SetupScreen::CreatePhoto {
                SetupScreen::Create
            } else {
                SetupScreen::Welcome
            };
            *model.setup_screen.borrow_mut() = target;
            render_setup(&model);
        });
    }
    container.append(&back);

    let header = gtk::Label::new(Some(title));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    container
}

pub(crate) fn render_setup_welcome(model: &AppRef) {
    let container = gtk::Box::new(gtk::Orientation::Vertical, 18);
    container.set_halign(gtk::Align::Center);
    container.set_width_request(340);

    let icon = gtk::Image::from_icon_name("iris-drive");
    icon.set_pixel_size(96);
    icon.set_margin_bottom(4);
    container.append(&icon);

    let title = gtk::Label::new(Some("Iris Drive"));
    title.add_css_class("title-1");
    title.set_halign(gtk::Align::Center);
    title.set_margin_bottom(10);
    container.append(&title);

    let create = welcome_button("Create profile", "list-add-symbolic", true);
    {
        let model = Rc::clone(model);
        create.connect_clicked(move |_| {
            *model.setup_screen.borrow_mut() = SetupScreen::Create;
            render_setup(&model);
        });
    }
    container.append(&create);

    let restore = welcome_button("Sign in", "system-log-in-symbolic", false);
    {
        let model = Rc::clone(model);
        restore.connect_clicked(move |_| {
            *model.setup_screen.borrow_mut() = SetupScreen::Restore;
            render_setup(&model);
        });
    }
    container.append(&restore);

    append_centered_setup(model, &container);
}

pub(crate) fn welcome_button(label: &str, icon_name: &str, primary: bool) -> gtk::Button {
    let button = if primary {
        primary_button(label)
    } else {
        pill_button(label)
    };
    button.set_width_request(340);

    let content = adw::ButtonContent::builder()
        .icon_name(icon_name)
        .label(label)
        .build();
    button.set_child(Some(&content));
    button
}

pub(crate) fn render_create_profile(model: &AppRef) {
    let container = setup_container(model, "Create profile");
    let username = setup_entry("Username (optional)");
    username.set_text(&model.setup_username.borrow());
    container.append(&username);

    let notice = setup_notice();
    let submit = primary_button("Create profile");
    {
        let model = Rc::clone(model);
        let username = username.clone();
        let notice = notice.clone();
        submit.connect_clicked(move |button| {
            let username_value = username.text().trim().to_string();
            if !username_value.is_empty() {
                *model.setup_username.borrow_mut() = username_value;
                *model.setup_screen.borrow_mut() = SetupScreen::CreatePhoto;
                render_setup(&model);
                return;
            }
            button.set_sensitive(false);
            match create_profile("", None) {
                Ok(()) => {
                    model.setup_username.borrow_mut().clear();
                    *model.setup_screen.borrow_mut() = SetupScreen::Welcome;
                    refresh(&model);
                }
                Err(error) => {
                    notice.set_text(&error);
                    button.set_sensitive(true);
                }
            }
        });
    }
    container.append(&submit);
    container.append(&notice);
    append_centered_setup(model, &container);

    username.grab_focus();
}

pub(crate) fn render_create_profile_photo(model: &AppRef) {
    let container = setup_container(model, "Profile photo");
    let photo = setup_entry("Photo path (optional)");
    container.append(&photo);

    let notice = setup_notice();
    let submit = primary_button("Create profile");
    {
        let model = Rc::clone(model);
        let photo = photo.clone();
        let notice = notice.clone();
        submit.connect_clicked(move |button| {
            button.set_sensitive(false);
            let username = model.setup_username.borrow().clone();
            let photo_value = photo.text().trim().to_string();
            let photo_arg = (!photo_value.is_empty()).then_some(photo_value.as_str());
            match create_profile(&username, photo_arg) {
                Ok(()) => {
                    model.setup_username.borrow_mut().clear();
                    *model.setup_screen.borrow_mut() = SetupScreen::Welcome;
                    refresh(&model);
                }
                Err(error) => {
                    notice.set_text(&error);
                    button.set_sensitive(true);
                }
            }
        });
    }
    container.append(&submit);
    let later = pill_button("Later");
    {
        let model = Rc::clone(model);
        let notice = notice.clone();
        later.connect_clicked(move |button| {
            button.set_sensitive(false);
            let username = model.setup_username.borrow().clone();
            match create_profile(&username, None) {
                Ok(()) => {
                    model.setup_username.borrow_mut().clear();
                    *model.setup_screen.borrow_mut() = SetupScreen::Welcome;
                    refresh(&model);
                }
                Err(error) => {
                    notice.set_text(&error);
                    button.set_sensitive(true);
                }
            }
        });
    }
    container.append(&later);
    container.append(&notice);
    append_centered_setup(model, &container);
}

pub(crate) fn render_restore_profile(model: &AppRef) {
    let container = setup_container(model, "Sign in");
    let nsec = setup_entry("Secret key");
    nsec.set_visibility(false);
    nsec.set_input_purpose(gtk::InputPurpose::Password);
    container.append(&nsec);

    let notice = setup_notice();
    let submit = primary_button("Sign in");
    {
        let model = Rc::clone(model);
        let nsec = nsec.clone();
        let notice = notice.clone();
        submit.connect_clicked(move |button| {
            let secret = nsec.text().trim().to_string();
            if secret.is_empty() {
                notice.set_text("Secret key is required.");
                return;
            }
            button.set_sensitive(false);
            match restore_profile(&secret) {
                Ok(()) => {
                    *model.setup_screen.borrow_mut() = SetupScreen::Welcome;
                    refresh(&model);
                }
                Err(error) => {
                    notice.set_text(&error);
                    button.set_sensitive(true);
                }
            }
        });
    }
    container.append(&submit);

    let link = pill_button("Link this device");
    link.set_width_request(340);
    let link_content = adw::ButtonContent::builder()
        .icon_name("computer-symbolic")
        .label("Link this device")
        .build();
    link.set_child(Some(&link_content));
    {
        let model = Rc::clone(model);
        link.connect_clicked(move |_| {
            *model.setup_screen.borrow_mut() = SetupScreen::Link;
            render_setup(&model);
        });
    }
    container.append(&link);

    container.append(&notice);
    append_centered_setup(model, &container);

    nsec.grab_focus();
}

pub(crate) fn render_link_device(model: &AppRef) {
    let container = setup_container(model, "Link this device");
    let owner = setup_entry("Owner public key");
    container.append(&owner);

    let notice = setup_notice();
    let submit = primary_button("Link device");
    {
        let model = Rc::clone(model);
        let owner = owner.clone();
        let notice = notice.clone();
        submit.connect_clicked(move |button| {
            let owner_value = owner.text().trim().to_string();
            if owner_value.is_empty() {
                notice.set_text("Owner public key is required.");
                return;
            }
            button.set_sensitive(false);
            match link_device(&owner_value) {
                Ok(()) => {
                    *model.setup_screen.borrow_mut() = SetupScreen::Welcome;
                    refresh(&model);
                }
                Err(error) => {
                    notice.set_text(&error);
                    button.set_sensitive(true);
                }
            }
        });
    }
    container.append(&submit);
    container.append(&notice);
    append_centered_setup(model, &container);

    owner.grab_focus();
}

pub(crate) fn append_centered_setup(model: &AppRef, child: &gtk::Box) {
    let top = gtk::Box::new(gtk::Orientation::Vertical, 0);
    top.set_vexpand(true);
    let bottom = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bottom.set_vexpand(true);
    model.ui.setup.append(&top);
    model.ui.setup.append(child);
    model.ui.setup.append(&bottom);
}

pub(crate) fn setup_notice() -> gtk::Label {
    let notice = gtk::Label::new(None);
    notice.add_css_class("iris-notice");
    notice.set_xalign(0.0);
    notice.set_wrap(true);
    notice
}

pub(crate) fn initialize_drive(model: &AppRef) {
    match create_profile("", None) {
        Ok(()) => {
            model.ui.notice.set_text("Initialized");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn create_profile(username: &str, photo_path: Option<&str>) -> Result<(), String> {
    let mut args = vec!["init".to_string(), "--force".to_string()];
    let username = username.trim();
    if !username.is_empty() {
        args.push("--username".to_string());
        args.push(username.to_string());
        if let Some(photo_path) = photo_path.map(str::trim).filter(|value| !value.is_empty()) {
            args.push("--profile-photo".to_string());
            args.push(photo_path.to_string());
        }
    }
    run_idrive_owned(&args)
}

pub(crate) fn restore_profile(secret: &str) -> Result<(), String> {
    run_idrive_owned(&["restore".to_string(), secret.to_string()])
}

pub(crate) fn link_device(owner: &str) -> Result<(), String> {
    run_idrive_owned(&["link".to_string(), owner.to_string()])
}

pub(crate) fn revoke_device(device: &str) -> Result<(), String> {
    run_idrive(["revoke", device])
}

pub(crate) fn appoint_admin(device: &str) -> Result<(), String> {
    run_idrive(["devices", "appoint-admin", device])
}

pub(crate) fn demote_admin(device: &str) -> Result<(), String> {
    run_idrive(["devices", "demote-admin", device])
}
