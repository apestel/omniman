use std::{cell::RefCell, rc::Rc};

use gtk4::prelude::*;
use libadwaita::prelude::*;
use omniman_core::config::Config;

pub fn build(parent: Option<&impl IsA<gtk4::Window>>) -> libadwaita::PreferencesWindow {
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
    win
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
        .description("Used for AI queries. Restart omnimand to apply.")
        .build();
    page.add(&gemini_group);

    let api_key_row = libadwaita::PasswordEntryRow::builder()
        .title("API key")
        .text(config.borrow().ai.gemini_api_key.as_deref().unwrap_or(""))
        .build();
    gemini_group.add(&api_key_row);

    let docs_row = libadwaita::ActionRow::builder()
        .title("Get an API key")
        .subtitle("Google AI Studio — aistudio.google.com")
        .activatable(true)
        .build();
    let link_icon = gtk4::Image::from_icon_name("external-link-symbolic");
    docs_row.add_suffix(&link_icon);
    docs_row.connect_activated(|_| {
        let _ = std::process::Command::new("xdg-open")
            .arg("https://aistudio.google.com/app/apikey")
            .spawn();
    });
    gemini_group.add(&docs_row);

    api_key_row.connect_changed({
        let config = Rc::clone(&config);
        move |r| {
            let v = r.text().to_string();
            config.borrow_mut().ai.gemini_api_key = if v.is_empty() { None } else { Some(v) };
            let _ = config.borrow().save();
        }
    });

    let model_row = libadwaita::ComboRow::builder()
        .title("Model")
        .subtitle("Fetching available models…")
        .sensitive(false)
        .build();
    gemini_group.add(&model_row);

    // Fetch available models from the daemon (which holds the API key) and
    // populate the combo once done. Falls back to a short hardcoded list when
    // the daemon is unreachable or no key is configured.
    {
        let model_row = model_row.clone();
        let config = Rc::clone(&config);
        let current_model = config.borrow().ai.model.clone();

        let (tx, rx) = async_channel::bounded::<Vec<omniman_core::types::ModelEntry>>(1);

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            rt.block_on(async move {
                let models = async {
                    use omniman_core::ipc::OmnimanProxy;
                    let conn = zbus::Connection::session().await?;
                    let proxy = OmnimanProxy::new(&conn).await?;
                    proxy.list_models().await
                }
                .await
                .unwrap_or_default();
                let _ = tx.send(models).await;
            });
        });

        gtk4::glib::spawn_future_local(async move {
            const FALLBACK: &[(&str, &str)] = &[
                ("gemini-2.5-flash", "Gemini 2.5 Flash"),
                ("gemini-2.5-pro", "Gemini 2.5 Pro"),
                ("gemini-1.5-flash", "Gemini 1.5 Flash"),
                ("gemini-1.5-pro", "Gemini 1.5 Pro"),
            ];

            let fetched = rx.recv().await.unwrap_or_default();
            let (ids, display_names, subtitle): (Vec<String>, Vec<String>, &str) =
                if fetched.is_empty() {
                    let ids = FALLBACK.iter().map(|(id, _)| id.to_string()).collect();
                    let names = FALLBACK.iter().map(|(_, n)| n.to_string()).collect();
                    (ids, names, "Daemon unreachable or no API key configured")
                } else {
                    let ids = fetched.iter().map(|m| m.id.clone()).collect();
                    let names = fetched.iter().map(|m| m.display_name.clone()).collect();
                    (ids, names, "")
                };

            let selected_idx =
                ids.iter().position(|id| *id == current_model).unwrap_or(0) as u32;

            let display_refs: Vec<&str> = display_names.iter().map(String::as_str).collect();
            model_row.set_model(Some(&gtk4::StringList::new(&display_refs)));
            model_row.set_selected(selected_idx);
            model_row.set_subtitle(subtitle);
            model_row.set_sensitive(true);

            model_row.connect_selected_notify(move |r| {
                let idx = r.selected() as usize;
                if let Some(id) = ids.get(idx) {
                    config.borrow_mut().ai.model = id.clone();
                    let _ = config.borrow().save();
                }
            });
        });
    }

    // ── OpenAI-compatible endpoint ────────────────────────────────────────────
    let oai_group = libadwaita::PreferencesGroup::builder()
        .title("OpenAI-compatible endpoint")
        .description("When set, overrides Gemini. Restart omnimand to apply.")
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
