use std::{cell::RefCell, rc::Rc};

use gtk4::prelude::*;
use libadwaita::prelude::*;
use omniman_core::config::Config;

pub fn build(parent: Option<&impl IsA<gtk4::Window>>) {
    let config = Rc::new(RefCell::new(Config::load().unwrap_or_default()));

    let win = libadwaita::PreferencesWindow::builder()
        .title("Omniman Preferences")
        .default_width(560)
        .modal(true)
        .build();
    if let Some(p) = parent {
        win.set_transient_for(Some(p));
    }

    build_general_page(&win, Rc::clone(&config));
    build_index_page(&win, Rc::clone(&config));
    build_ai_page(&win, Rc::clone(&config));
    build_clipboard_page(&win, Rc::clone(&config));

    win.present();
}

fn build_general_page(win: &libadwaita::PreferencesWindow, config: Rc<RefCell<Config>>) {
    let page = libadwaita::PreferencesPage::builder()
        .title("General")
        .icon_name("preferences-system-symbolic")
        .build();
    win.add(&page);

    let group = libadwaita::PreferencesGroup::builder()
        .title("Hotkey")
        .description("Restart omnimand for this change to take effect")
        .build();
    page.add(&group);

    let row = libadwaita::EntryRow::builder()
        .title("Global shortcut")
        .text(config.borrow().hotkey.shortcut.as_str())
        .build();
    group.add(&row);

    row.connect_changed(move |r| {
        config.borrow_mut().hotkey.shortcut = r.text().to_string();
        let _ = config.borrow().save();
    });
}

fn build_index_page(win: &libadwaita::PreferencesWindow, config: Rc<RefCell<Config>>) {
    let page = libadwaita::PreferencesPage::builder()
        .title("Index")
        .icon_name("folder-symbolic")
        .build();
    win.add(&page);

    let group = libadwaita::PreferencesGroup::builder()
        .title("Exclusions")
        .description("Comma-separated directory names to skip. Restart omnimand to apply.")
        .build();
    page.add(&group);

    let current = config.borrow().index.exclude_dirs.join(", ");
    let row = libadwaita::EntryRow::builder()
        .title("Excluded directories")
        .text(current.as_str())
        .build();
    group.add(&row);

    row.connect_changed(move |r| {
        let dirs: Vec<String> = r
            .text()
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        config.borrow_mut().index.exclude_dirs = dirs;
        let _ = config.borrow().save();
    });
}

fn build_ai_page(win: &libadwaita::PreferencesWindow, config: Rc<RefCell<Config>>) {
    let page = libadwaita::PreferencesPage::builder()
        .title("AI")
        .icon_name("brain-augemnted-symbolic")
        .build();
    win.add(&page);

    // ── Gemini ────────────────────────────────────────────────────────────────
    let gemini_group = libadwaita::PreferencesGroup::builder()
        .title("Gemini (default)")
        .description("Set GEMINI_API_KEY in omnimand's environment. Restart omnimand to apply.")
        .build();
    page.add(&gemini_group);

    let models = ["gemini-2.5-flash", "gemini-2.5-pro", "gemini-1.5-flash", "gemini-1.5-pro"];
    let model_list = gtk4::StringList::new(&models);

    let current_idx = {
        let m = config.borrow().ai.model.clone();
        models.iter().position(|s| *s == m.as_str()).unwrap_or(0) as u32
    };

    let model_row = libadwaita::ComboRow::builder()
        .title("Model")
        .model(&model_list)
        .selected(current_idx)
        .build();
    gemini_group.add(&model_row);

    model_row.connect_selected_notify({
        let config = Rc::clone(&config);
        move |r| {
            let model = r
                .selected_item()
                .and_downcast::<gtk4::StringObject>()
                .map(|s| s.string().to_string())
                .unwrap_or_else(|| models[0].to_string());
            config.borrow_mut().ai.model = model;
            let _ = config.borrow().save();
        }
    });

    // ── OpenAI-compatible endpoint ────────────────────────────────────────────
    let oai_group = libadwaita::PreferencesGroup::builder()
        .title("OpenAI-compatible endpoint")
        .description("When set, overrides Gemini. Restart Omniman to apply.")
        .build();
    page.add(&oai_group);

    let endpoint_row = libadwaita::EntryRow::builder()
        .title("Base URL")
        .text(config.borrow().ai.openai_endpoint.as_deref().unwrap_or(""))
        .build();
    oai_group.add(&endpoint_row);

    let key_row = libadwaita::PasswordEntryRow::builder()
        .title("API key")
        .text(config.borrow().ai.openai_key.as_deref().unwrap_or(""))
        .build();
    oai_group.add(&key_row);

    let oai_model_row = libadwaita::EntryRow::builder()
        .title("Model")
        .text(config.borrow().ai.openai_model.as_str())
        .build();
    oai_group.add(&oai_model_row);

    endpoint_row.connect_changed({
        let config = Rc::clone(&config);
        move |r| {
            let v = r.text().to_string();
            config.borrow_mut().ai.openai_endpoint = if v.is_empty() { None } else { Some(v) };
            let _ = config.borrow().save();
        }
    });

    key_row.connect_changed({
        let config = Rc::clone(&config);
        move |r| {
            let v = r.text().to_string();
            config.borrow_mut().ai.openai_key = if v.is_empty() { None } else { Some(v) };
            let _ = config.borrow().save();
        }
    });

    oai_model_row.connect_changed({
        let config = Rc::clone(&config);
        move |r| {
            let v = r.text().to_string();
            if !v.is_empty() {
                config.borrow_mut().ai.openai_model = v;
                let _ = config.borrow().save();
            }
        }
    });
}

fn build_clipboard_page(win: &libadwaita::PreferencesWindow, config: Rc<RefCell<Config>>) {
    let page = libadwaita::PreferencesPage::builder()
        .title("Clipboard")
        .icon_name("edit-paste-symbolic")
        .build();
    win.add(&page);

    let group = libadwaita::PreferencesGroup::builder()
        .title("History")
        .build();
    page.add(&group);

    let adj = gtk4::Adjustment::new(
        config.borrow().clipboard.history_limit as f64,
        10.0,
        50_000.0,
        1.0,
        100.0,
        0.0,
    );
    let row = libadwaita::SpinRow::new(Some(&adj), 1.0, 0);
    row.set_title("Maximum entries");
    group.add(&row);

    adj.connect_value_changed(move |a| {
        config.borrow_mut().clipboard.history_limit = a.value() as usize;
        let _ = config.borrow().save();
    });
}
