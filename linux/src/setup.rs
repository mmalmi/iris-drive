#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn render_setup(model: &AppRef) {
    clear_box(&model.ui.setup);
    match *model.setup_screen.borrow() {
        SetupScreen::Welcome => render_setup_welcome(model),
        SetupScreen::Create => render_create_profile(model),
        SetupScreen::CreatePhoto => render_create_profile_photo(model),
        SetupScreen::RestoreOptions => render_restore_options(model),
        SetupScreen::RestorePhrase => render_restore_phrase_profile(model),
        SetupScreen::RestoreSecretKey => render_restore_secret_key_profile(model),
        SetupScreen::Link => render_link_device(model),
    }
}

pub(crate) fn render_awaiting_approval(model: &AppRef, state: &NativeAppState, sync_running: bool) {
    clear_box(&model.ui.setup);

    let container = gtk::Box::new(gtk::Orientation::Vertical, 14);
    container.set_halign(gtk::Align::Center);
    container.set_valign(gtk::Align::Center);
    container.set_width_request(420);

    let header = gtk::Label::new(Some("Waiting for approval"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    let account = account(state);
    let owner = readonly_entry(
        account
            .map(|account| account.current_app_key_npub.as_str())
            .unwrap_or("-"),
    );
    container.append(&field_title("Owner"));
    container.append(&owner);

    let device = readonly_entry(
        account
            .map(|account| account.device_pubkey.as_str())
            .unwrap_or("-"),
    );
    container.append(&field_title("This device"));
    container.append(&device);

    let notice = setup_notice();
    notice.set_text(if sync_running {
        "Waiting for approval"
    } else {
        "Sync paused"
    });

    let copy = primary_button("Copy device ID");
    {
        let device = account
            .map(|account| account.device_pubkey.clone())
            .unwrap_or_default();
        let notice = notice.clone();
        copy.connect_clicked(move |_| {
            if device.is_empty() {
                notice.set_text("Nothing to copy");
            } else if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&device);
                notice.set_text("Device ID copied");
            } else {
                notice.set_text("Clipboard unavailable");
            }
        });
    }
    container.append(&copy);

    let logout_button = pill_button("Log out");
    logout_button.add_css_class("destructive-action");
    {
        let model = Rc::clone(model);
        logout_button.connect_clicked(move |_| logout(&model));
    }
    container.append(&logout_button);
    container.append(&notice);

    append_centered_setup(model, &container);
}

pub(crate) fn render_revoked_device(model: &AppRef, state: &NativeAppState) {
    clear_box(&model.ui.setup);

    let container = gtk::Box::new(gtk::Orientation::Vertical, 14);
    container.set_halign(gtk::Align::Center);
    container.set_valign(gtk::Align::Center);
    container.set_width_request(420);

    let header = gtk::Label::new(Some("Device removed"));
    header.add_css_class("title-2");
    header.set_halign(gtk::Align::Start);
    container.append(&header);

    let detail = gtk::Label::new(Some("This device no longer has access to Iris Drive."));
    detail.add_css_class("iris-muted");
    detail.set_wrap(true);
    detail.set_xalign(0.0);
    container.append(&detail);

    let account = account(state);
    let owner_npub = account
        .map(|account| account.current_app_key_npub.as_str())
        .unwrap_or("-");
    container.append(&field_title("Owner"));
    container.append(&readonly_entry(owner_npub));

    let device_npub = account
        .map(|account| account.device_pubkey.as_str())
        .unwrap_or("-");
    container.append(&field_title("This device"));
    container.append(&readonly_entry(device_npub));

    let notice = setup_notice();
    notice.set_text("Device removed");

    let relink = primary_button("Link this device again");
    {
        let model = Rc::clone(model);
        let owner = owner_npub.to_string();
        let notice = notice.clone();
        relink.connect_clicked(move |button| {
            if owner.trim().is_empty() || owner == "-" {
                notice.set_text("Owner key unavailable");
                return;
            }
            button.set_sensitive(false);
            match relink_device(&owner) {
                Ok(()) => refresh(&model),
                Err(error) => {
                    notice.set_text(&error);
                    button.set_sensitive(true);
                }
            }
        });
    }
    container.append(&relink);

    let copy = pill_button("Copy device ID");
    {
        let device = device_npub.to_string();
        let notice = notice.clone();
        copy.connect_clicked(move |_| {
            if device.trim().is_empty() || device == "-" {
                notice.set_text("Nothing to copy");
            } else if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&device);
                notice.set_text("Device ID copied");
            } else {
                notice.set_text("Clipboard unavailable");
            }
        });
    }
    container.append(&copy);

    let logout_button = pill_button("Log out");
    logout_button.add_css_class("destructive-action");
    {
        let model = Rc::clone(model);
        logout_button.connect_clicked(move |_| logout(&model));
    }
    container.append(&logout_button);
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
            let target = match *model.setup_screen.borrow() {
                SetupScreen::CreatePhoto => SetupScreen::Create,
                SetupScreen::RestorePhrase | SetupScreen::RestoreSecretKey | SetupScreen::Link => {
                    SetupScreen::RestoreOptions
                }
                _ => SetupScreen::Welcome,
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
            *model.setup_screen.borrow_mut() = SetupScreen::RestoreOptions;
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
    {
        let submit = submit.clone();
        username.connect_activate(move |_| submit.emit_clicked());
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
    {
        let submit = submit.clone();
        photo.connect_activate(move |_| submit.emit_clicked());
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

pub(crate) fn render_restore_options(model: &AppRef) {
    let container = setup_container(model, "Restore");

    let link = welcome_button("Link device", "computer-symbolic", false);
    {
        let model = Rc::clone(model);
        link.connect_clicked(move |_| {
            *model.setup_screen.borrow_mut() = SetupScreen::Link;
            render_setup(&model);
        });
    }
    container.append(&link);

    let phrase = welcome_button(
        "Restore from recovery phrase",
        "dialog-password-symbolic",
        false,
    );
    {
        let model = Rc::clone(model);
        phrase.connect_clicked(move |_| {
            model.setup_recovery_word_index.set(0);
            *model.setup_recovery_words.borrow_mut() =
                vec![String::new(); RECOVERY_PHRASE_WORD_COUNT];
            *model.setup_screen.borrow_mut() = SetupScreen::RestorePhrase;
            render_setup(&model);
        });
    }
    container.append(&phrase);

    let secret = welcome_button("Restore from secret key", "dialog-password-symbolic", false);
    {
        let model = Rc::clone(model);
        secret.connect_clicked(move |_| {
            *model.setup_screen.borrow_mut() = SetupScreen::RestoreSecretKey;
            render_setup(&model);
        });
    }
    container.append(&secret);

    append_centered_setup(model, &container);
}

pub(crate) fn render_restore_phrase_profile(model: &AppRef) {
    let container = setup_container(model, "Recovery phrase");
    clamp_recovery_word_index(model);
    let word_index = model.setup_recovery_word_index.get();

    container.append(&field_title(&format!(
        "Word {} of {}",
        word_index + 1,
        RECOVERY_PHRASE_WORD_COUNT
    )));
    let word = setup_entry(&format!("Word {}", word_index + 1));
    word.set_text(&current_recovery_word(model));
    container.append(&word);

    let notice = setup_notice();
    let paste = pill_button("Paste from clipboard");
    {
        let model = Rc::clone(model);
        let notice = notice.clone();
        paste.connect_clicked(move |_| paste_recovery_phrase_from_clipboard(&model, &notice));
    }
    container.append(&paste);

    let nav = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let back = pill_button("Back");
    back.set_sensitive(word_index > 0);
    {
        let model = Rc::clone(model);
        back.connect_clicked(move |_| {
            let next_index = model.setup_recovery_word_index.get().saturating_sub(1);
            model.setup_recovery_word_index.set(next_index);
            render_setup(&model);
        });
    }
    nav.append(&back);

    let submit = primary_button(if word_index == RECOVERY_PHRASE_WORD_COUNT - 1 {
        "Restore"
    } else {
        "Next"
    });
    submit.set_sensitive(can_advance_recovery_word(model));
    {
        let model = Rc::clone(model);
        let notice = notice.clone();
        submit.connect_clicked(move |button| {
            advance_or_restore_recovery_phrase(&model, &notice, button)
        });
    }
    nav.append(&submit);
    container.append(&nav);

    {
        let model = Rc::clone(model);
        let submit = submit.clone();
        word.connect_changed(move |entry| {
            apply_recovery_word_input(&model, entry.text().as_str());
            submit.set_sensitive(can_advance_recovery_word(&model));
        });
    }
    {
        let submit = submit.clone();
        word.connect_activate(move |_| submit.emit_clicked());
    }

    container.append(&notice);
    append_centered_setup(model, &container);

    word.grab_focus();
}

pub(crate) fn render_restore_secret_key_profile(model: &AppRef) {
    let container = setup_container(model, "Secret key");
    let nsec = setup_entry("nsec1... or hex secret key");
    nsec.set_visibility(false);
    nsec.set_input_purpose(gtk::InputPurpose::Password);
    container.append(&nsec);

    let notice = setup_notice();
    let submit = primary_button("Restore");
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
    {
        let submit = submit.clone();
        nsec.connect_activate(move |_| submit.emit_clicked());
    }
    container.append(&submit);
    container.append(&notice);
    append_centered_setup(model, &container);

    nsec.grab_focus();
}

pub(crate) fn render_link_device(model: &AppRef) {
    let container = setup_container(model, "Link this device");
    let owner = setup_entry("Owner public key or invite link");
    container.append(&owner);

    let notice = setup_notice();
    let submit = primary_button("Link device");
    let submitted_owner = Rc::new(RefCell::new(String::new()));
    {
        let model = Rc::clone(model);
        let notice = notice.clone();
        let submit = submit.clone();
        let submitted_owner = Rc::clone(&submitted_owner);
        owner.connect_changed(move |entry| {
            let owner_value = entry.text().trim().to_string();
            if !link_owner_input_is_complete(&owner_value)
                || *submitted_owner.borrow() == owner_value
            {
                return;
            }
            submitted_owner.replace(owner_value);
            submit_link_device(&model, entry, &notice, &submit);
        });
    }
    {
        let model = Rc::clone(model);
        let owner = owner.clone();
        let notice = notice.clone();
        submit.connect_clicked(move |button| {
            submit_link_device(&model, &owner, &notice, button);
        });
    }
    {
        let submit = submit.clone();
        owner.connect_activate(move |_| submit.emit_clicked());
    }
    container.append(&submit);
    container.append(&notice);
    append_centered_setup(model, &container);

    owner.grab_focus();
}

fn submit_link_device(
    model: &AppRef,
    owner: &gtk::Entry,
    notice: &gtk::Label,
    button: &gtk::Button,
) {
    let owner_value = owner.text().trim().to_string();
    if owner_value.is_empty() {
        notice.set_text("Owner public key or invite link is required.");
        return;
    }
    button.set_sensitive(false);
    match link_device(&owner_value) {
        Ok(()) => {
            *model.setup_screen.borrow_mut() = SetupScreen::Welcome;
            refresh(model);
        }
        Err(error) => {
            notice.set_text(&error);
            button.set_sensitive(true);
        }
    }
}

fn link_owner_input_is_complete(value: &str) -> bool {
    iris_drive_app_core::validate_link_input(value.to_string()).is_complete
}

fn clamp_recovery_word_index(model: &AppRef) {
    if model.setup_recovery_word_index.get() >= RECOVERY_PHRASE_WORD_COUNT {
        model
            .setup_recovery_word_index
            .set(RECOVERY_PHRASE_WORD_COUNT - 1);
    }
}

fn current_recovery_word(model: &AppRef) -> String {
    let words = model.setup_recovery_words.borrow();
    words
        .get(
            model
                .setup_recovery_word_index
                .get()
                .min(RECOVERY_PHRASE_WORD_COUNT - 1),
        )
        .cloned()
        .unwrap_or_default()
}

fn apply_recovery_word_input(model: &AppRef, input: &str) {
    let parts = input
        .split_whitespace()
        .map(|word| word.to_lowercase())
        .collect::<Vec<_>>();
    let word_index = model
        .setup_recovery_word_index
        .get()
        .min(RECOVERY_PHRASE_WORD_COUNT - 1);
    let mut words = model.setup_recovery_words.borrow_mut();
    if words.len() != RECOVERY_PHRASE_WORD_COUNT {
        words.resize(RECOVERY_PHRASE_WORD_COUNT, String::new());
    }
    if parts.len() <= 1 {
        words[word_index] = input.trim().to_lowercase();
        return;
    }
    for (offset, word) in parts.iter().enumerate() {
        let target = word_index + offset;
        if target < words.len() {
            words[target] = word.clone();
        }
    }
    drop(words);
    model
        .setup_recovery_word_index
        .set((word_index + parts.len() - 1).min(RECOVERY_PHRASE_WORD_COUNT - 1));
}

fn can_advance_recovery_word(model: &AppRef) -> bool {
    let words = model.setup_recovery_words.borrow();
    let word_index = model
        .setup_recovery_word_index
        .get()
        .min(RECOVERY_PHRASE_WORD_COUNT - 1);
    if word_index == RECOVERY_PHRASE_WORD_COUNT - 1 {
        words
            .iter()
            .take(RECOVERY_PHRASE_WORD_COUNT)
            .all(|word| !word.trim().is_empty())
    } else {
        words
            .get(word_index)
            .is_some_and(|word| !word.trim().is_empty())
    }
}

fn setup_recovery_phrase(model: &AppRef) -> String {
    model
        .setup_recovery_words
        .borrow()
        .iter()
        .take(RECOVERY_PHRASE_WORD_COUNT)
        .map(|word| word.trim().to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn advance_or_restore_recovery_phrase(model: &AppRef, notice: &gtk::Label, button: &gtk::Button) {
    let word_index = model
        .setup_recovery_word_index
        .get()
        .min(RECOVERY_PHRASE_WORD_COUNT - 1);
    if word_index < RECOVERY_PHRASE_WORD_COUNT - 1 {
        if can_advance_recovery_word(model) {
            model.setup_recovery_word_index.set(word_index + 1);
            render_setup(model);
        }
        return;
    }
    if !can_advance_recovery_word(model) {
        notice.set_text(&format!(
            "All {} words are required.",
            RECOVERY_PHRASE_WORD_COUNT
        ));
        return;
    }

    button.set_sensitive(false);
    match restore_profile(&setup_recovery_phrase(model)) {
        Ok(()) => {
            model.setup_recovery_words.borrow_mut().fill(String::new());
            model.setup_recovery_word_index.set(0);
            *model.setup_screen.borrow_mut() = SetupScreen::Welcome;
            refresh(model);
        }
        Err(error) => {
            notice.set_text(&error);
            button.set_sensitive(true);
        }
    }
}

fn paste_recovery_phrase_from_clipboard(model: &AppRef, notice: &gtk::Label) {
    let Some(display) = gtk::gdk::Display::default() else {
        notice.set_text("Clipboard unavailable");
        return;
    };
    let clipboard = display.clipboard();
    let model = Rc::clone(model);
    let notice = notice.clone();
    glib::MainContext::default().spawn_local(async move {
        match clipboard.read_text_future().await {
            Ok(Some(text)) if !text.trim().is_empty() => {
                apply_recovery_word_input(&model, text.as_str());
                render_setup(&model);
            }
            Ok(_) => notice.set_text("Clipboard is empty"),
            Err(error) => notice.set_text(&format!("Clipboard unavailable: {error}")),
        }
    });
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

pub(crate) fn relink_device(owner: &str) -> Result<(), String> {
    run_idrive_owned(&["link".to_string(), owner.to_string(), "--force".to_string()])
}

pub(crate) fn revoke_device(device: &str) -> Result<(), String> {
    dispatch_desktop_action(NativeAppAction::RevokeDevice {
        device_pubkey: device.to_string(),
    })
    .map(|_| ())
}

pub(crate) fn delete_device(device: &str) -> Result<(), String> {
    revoke_device(device)
}

pub(crate) fn reject_device(request: &str) -> Result<(), String> {
    dispatch_desktop_action(NativeAppAction::RejectDevice {
        request: request.to_string(),
    })
    .map(|_| ())
}

pub(crate) fn appoint_admin(device: &str) -> Result<(), String> {
    dispatch_desktop_action(NativeAppAction::AppointAdmin {
        device_pubkey: device.to_string(),
    })
    .map(|_| ())
}

pub(crate) fn demote_admin(device: &str) -> Result<(), String> {
    dispatch_desktop_action(NativeAppAction::DemoteAdmin {
        device_pubkey: device.to_string(),
    })
    .map(|_| ())
}
