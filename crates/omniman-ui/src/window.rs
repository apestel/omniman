use std::{cell::Cell, rc::Rc, time::Duration};

use gtk4::{gdk, glib, prelude::*};
use libadwaita::prelude::*;
use omniman_core::types::{ClipEntry, Hit};

use crate::AiMsg;

pub fn build(
    app: &libadwaita::Application,
    query_tx: async_channel::Sender<String>,
    result_rx: async_channel::Receiver<Vec<Hit>>,
    show_rx: async_channel::Receiver<()>,
    clip_req_tx: async_channel::Sender<()>,
    clip_result_rx: async_channel::Receiver<Vec<ClipEntry>>,
    ai_req_tx: async_channel::Sender<String>,
    ai_result_rx: async_channel::Receiver<AiMsg>,
) -> libadwaita::ApplicationWindow {
    load_css();

    let window = libadwaita::ApplicationWindow::builder()
        .application(app)
        .title("Omniman")
        .default_width(660)
        .default_height(480)
        .decorated(false)
        .css_classes(["omniman-launcher"])
        .hide_on_close(true)
        .build();

    // ── Root layout ───────────────────────────────────────────────────────────
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.add_css_class("omniman-root");

    // ── Search row (entry + Ask AI button) ───────────────────────────────────
    let search_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);

    let search_entry = gtk4::SearchEntry::builder()
        .placeholder_text("Search files, clipboard, ask AI…")
        .hexpand(true)
        .margin_top(14)
        .margin_bottom(14)
        .margin_start(18)
        .margin_end(8)
        .css_classes(["omniman-search"])
        .build();

    let ask_btn = gtk4::Button::builder()
        .label("Ask AI")
        .margin_top(10)
        .margin_bottom(10)
        .margin_end(12)
        .css_classes(["omniman-ask-btn", "suggested-action"])
        .build();

    search_row.append(&search_entry);
    search_row.append(&ask_btn);

    // ── Tab bar ───────────────────────────────────────────────────────────────
    let tab_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    tab_bar.add_css_class("omniman-tabbar");

    let btn_files = gtk4::ToggleButton::builder()
        .label("Files")
        .active(true)
        .css_classes(["omniman-tab"])
        .build();
    let btn_clipboard = gtk4::ToggleButton::builder()
        .label("Clipboard")
        .group(&btn_files)
        .css_classes(["omniman-tab"])
        .build();
    let btn_ai = gtk4::ToggleButton::builder()
        .label("AI")
        .group(&btn_files)
        .css_classes(["omniman-tab"])
        .build();

    let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);

    let gear_btn = gtk4::Button::builder()
        .icon_name("preferences-system-symbolic")
        .css_classes(["omniman-gear-btn", "flat"])
        .tooltip_text("Preferences")
        .margin_top(4)
        .margin_bottom(4)
        .margin_end(6)
        .build();

    tab_bar.append(&btn_files);
    tab_bar.append(&btn_clipboard);
    tab_bar.append(&btn_ai);
    tab_bar.append(&spacer);
    tab_bar.append(&gear_btn);

    // ── Files list ────────────────────────────────────────────────────────────
    let files_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Browse)
        .css_classes(["omniman-list"])
        .build();
    let placeholder = gtk4::Label::builder()
        .label("No results")
        .css_classes(["omniman-placeholder"])
        .margin_top(32)
        .build();
    files_list.set_placeholder(Some(&placeholder));

    let files_scroll = gtk4::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(&files_list)
        .build();

    // ── Clipboard list ────────────────────────────────────────────────────────
    let clip_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Browse)
        .css_classes(["omniman-list"])
        .build();
    let clip_placeholder = gtk4::Label::builder()
        .label("No clipboard history")
        .css_classes(["omniman-placeholder"])
        .margin_top(32)
        .build();
    clip_list.set_placeholder(Some(&clip_placeholder));

    let clip_scroll = gtk4::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(&clip_list)
        .build();

    // ── AI panel ─────────────────────────────────────────────────────────────
    let ai_panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    // Loading state: spinner + label
    let spinner_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    spinner_box.set_valign(gtk4::Align::Center);
    spinner_box.set_vexpand(true);
    let spinner = gtk4::Spinner::new();
    spinner.set_size_request(32, 32);
    spinner.set_halign(gtk4::Align::Center);
    let spinner_label = gtk4::Label::builder()
        .label("Asking Gemini…")
        .css_classes(["omniman-placeholder"])
        .halign(gtk4::Align::Center)
        .build();
    spinner_box.append(&spinner);
    spinner_box.append(&spinner_label);

    // Response text view
    let ai_buffer = gtk4::TextBuffer::new(None);
    setup_text_tags(&ai_buffer);
    let ai_text_view = gtk4::TextView::builder()
        .buffer(&ai_buffer)
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk4::WrapMode::WordChar)
        .left_margin(16)
        .right_margin(16)
        .top_margin(12)
        .bottom_margin(12)
        .css_classes(["omniman-ai-text"])
        .build();

    let ai_scroll = gtk4::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(&ai_text_view)
        .build();

    // Stack: loading | response
    let ai_stack = gtk4::Stack::new();
    ai_stack.add_named(&spinner_box, Some("loading"));
    ai_stack.add_named(&ai_scroll, Some("response"));
    ai_stack.set_visible_child_name("response");
    ai_panel.append(&ai_stack);

    // ── Main stack ────────────────────────────────────────────────────────────
    let stack = gtk4::Stack::new();
    stack.add_named(&files_scroll, Some("files"));
    stack.add_named(&clip_scroll, Some("clipboard"));
    stack.add_named(&ai_panel, Some("ai"));
    stack.set_visible_child_name("files");

    root.append(&search_row);
    root.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
    root.append(&tab_bar);
    root.append(&gtk4::Separator::new(gtk4::Orientation::Horizontal));
    root.append(&stack);
    window.set_content(Some(&root));

    // ── Gear button → preferences window ─────────────────────────────────────
    gear_btn.connect_clicked({
        let window_weak = window.downgrade();
        move |_| {
            let parent = window_weak.upgrade();
            crate::prefs::build(parent.as_ref().map(|w| w.upcast_ref::<gtk4::Window>()));
        }
    });

    // ── "Ask AI" button → trigger AI request ─────────────────────────────────
    ask_btn.connect_clicked({
        let search_entry = search_entry.clone();
        let ai_req_tx = ai_req_tx.clone();
        let btn_ai = btn_ai.clone();
        let ai_stack = ai_stack.clone();
        let spinner = spinner.clone();
        let ai_buffer = ai_buffer.clone();
        move |_| {
            let query = search_entry.text().to_string();
            if query.trim().is_empty() {
                return;
            }
            btn_ai.set_active(true);
            start_ai_request(&ai_req_tx, &ai_stack, &spinner, &ai_buffer, query);
        }
    });

    // ── Tab switching ─────────────────────────────────────────────────────────
    btn_files.connect_toggled({
        let stack = stack.clone();
        move |btn| {
            if btn.is_active() {
                stack.set_visible_child_name("files");
            }
        }
    });

    btn_clipboard.connect_toggled({
        let stack = stack.clone();
        let clip_req_tx = clip_req_tx.clone();
        move |btn| {
            if btn.is_active() {
                stack.set_visible_child_name("clipboard");
                let _ = clip_req_tx.try_send(());
            }
        }
    });

    btn_ai.connect_toggled({
        let stack = stack.clone();
        let search_entry = search_entry.clone();
        let ai_req_tx = ai_req_tx.clone();
        let ai_stack = ai_stack.clone();
        let spinner = spinner.clone();
        let ai_buffer = ai_buffer.clone();
        move |btn| {
            if btn.is_active() {
                stack.set_visible_child_name("ai");
                let query = search_entry.text().to_string();
                if !query.trim().is_empty() {
                    start_ai_request(&ai_req_tx, &ai_stack, &spinner, &ai_buffer, query);
                }
            }
        }
    });

    // ── Debounced file search + Ask AI visibility ─────────────────────────────
    let debounce: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));

    search_entry.connect_changed({
        let debounce = Rc::clone(&debounce);
        let query_tx = query_tx.clone();
        let btn_files = btn_files.clone();
        move |entry| {
            if let Some(id) = debounce.take() {
                id.remove();
            }
            let text = entry.text().to_string();

            if text.trim().is_empty() {
                return;
            }
            if !btn_files.is_active() {
                btn_files.set_active(true);
            }
            let qt = query_tx.clone();
            let debounce_cb = Rc::clone(&debounce);
            let id = glib::timeout_add_local_once(Duration::from_millis(200), move || {
                // Clear the stored ID before firing — if connect_changed runs
                // after this, debounce.take() returns None and avoids a stale
                // SourceId::remove() panic.
                debounce_cb.set(None);
                let _ = qt.try_send(text);
            });
            debounce.set(Some(id));
        }
    });

    // ── Keyboard navigation ───────────────────────────────────────────────────
    let key_ctrl = gtk4::EventControllerKey::new();
    {
        let stack = stack.clone();
        let btn_files = btn_files.clone();
        let btn_clipboard = btn_clipboard.clone();
        let btn_ai = btn_ai.clone();
        let files_list = files_list.clone();
        let clip_list = clip_list.clone();
        let search_entry = search_entry.clone();
        let ai_req_tx = ai_req_tx.clone();
        let ai_stack = ai_stack.clone();
        let spinner = spinner.clone();
        let ai_buffer = ai_buffer.clone();
        let window_weak = window.downgrade();
        key_ctrl.connect_key_pressed(move |_, key, _, mods| {
            match key {
                // Escape when focus is on a list row or any non-entry widget.
                // When the search entry has focus it handles Escape itself and
                // emits stop-search (see connect_stop_search below) — that
                // signal stops propagation so this branch is never reached then.
                gdk::Key::Escape => {
                    if let Some(win) = window_weak.upgrade() {
                        win.set_visible(false);
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::l if mods.contains(gdk::ModifierType::CONTROL_MASK) => {
                    search_entry.grab_focus();
                    search_entry.select_region(0, -1);
                    glib::Propagation::Stop
                }
                gdk::Key::Tab if mods.contains(gdk::ModifierType::CONTROL_MASK) => {
                    if btn_files.is_active() {
                        btn_clipboard.set_active(true);
                    } else if btn_clipboard.is_active() {
                        btn_ai.set_active(true);
                    } else {
                        btn_files.set_active(true);
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Return | gdk::Key::KP_Enter
                    if mods.contains(gdk::ModifierType::CONTROL_MASK) =>
                {
                    // Ctrl+Enter → Ask AI.
                    let query = search_entry.text().to_string();
                    if !query.trim().is_empty() {
                        btn_ai.set_active(true);
                        start_ai_request(&ai_req_tx, &ai_stack, &spinner, &ai_buffer, query);
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Down => {
                    let active = stack.visible_child_name().unwrap_or_default();
                    let list = if active == "files" { &files_list } else { &clip_list };
                    if let Some(row) = list.row_at_index(0) {
                        row.grab_focus();
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    let active = stack.visible_child_name().unwrap_or_default();
                    match active.as_str() {
                        "files" => {
                            let row = files_list
                                .selected_row()
                                .or_else(|| files_list.row_at_index(0));
                            if let Some(row) = row {
                                row.activate();
                            }
                        }
                        "clipboard" => {
                            let row = clip_list
                                .selected_row()
                                .or_else(|| clip_list.row_at_index(0));
                            if let Some(row) = row {
                                row.activate();
                            }
                        }
                        _ => {}
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
    }
    window.add_controller(key_ctrl);

    // SearchEntry captures Escape and emits stop-search without bubbling the
    // event further, so the key controller above never sees it.  Handle it here.
    search_entry.connect_stop_search({
        let window_weak = window.downgrade();
        move |entry| {
            entry.set_text("");
            if let Some(win) = window_weak.upgrade() {
                win.set_visible(false);
            }
        }
    });

    // ── File row activation ───────────────────────────────────────────────────
    files_list.connect_row_activated(|_, row| {
        if let Some(ar) = row.downcast_ref::<libadwaita::ActionRow>() {
            if let Some(path) = ar.subtitle() {
                let _ = std::process::Command::new("xdg-open")
                    .arg(path.as_str())
                    .spawn();
            }
        }
    });

    // ── Clipboard row activation → wl-copy ───────────────────────────────────
    clip_list.connect_row_activated(|_, row| {
        if let Some(ar) = row.downcast_ref::<libadwaita::ActionRow>() {
            if let Some(content) = ar.subtitle() {
                let _ = std::process::Command::new("wl-copy")
                    .arg("--")
                    .arg(content.as_str())
                    .spawn();
            }
        }
    });

    // ── Receive file search results ───────────────────────────────────────────
    glib::spawn_future_local({
        let files_list = files_list.clone();
        async move {
            while let Ok(hits) = result_rx.recv().await {
                while let Some(child) = files_list.first_child() {
                    files_list.remove(&child);
                }
                for hit in &hits {
                    let row = libadwaita::ActionRow::builder()
                        .title(glib::markup_escape_text(&hit.filename))
                        .subtitle(glib::markup_escape_text(&hit.path))
                        .activatable(true)
                        .build();
                    let icon = gtk4::Image::from_icon_name(mime_icon(&hit.path));
                    icon.set_pixel_size(24);
                    row.add_prefix(&icon);
                    files_list.append(&row);
                }
            }
        }
    });

    // ── Receive clipboard results ─────────────────────────────────────────────
    glib::spawn_future_local({
        let clip_list = clip_list.clone();
        async move {
            while let Ok(entries) = clip_result_rx.recv().await {
                while let Some(child) = clip_list.first_child() {
                    clip_list.remove(&child);
                }
                for entry in &entries {
                    let preview: String = entry.content.chars().take(120).collect();
                    let row = libadwaita::ActionRow::builder()
                        .title(glib::markup_escape_text(&preview))
                        .subtitle(glib::markup_escape_text(&entry.content))
                        .activatable(true)
                        .build();
                    let icon_name = match entry.kind.as_str() {
                        "Uri" => "folder-symbolic",
                        "Image" => "image-x-generic-symbolic",
                        _ => "edit-paste-symbolic",
                    };
                    let icon = gtk4::Image::from_icon_name(icon_name);
                    icon.set_pixel_size(20);
                    row.add_prefix(&icon);
                    clip_list.append(&row);
                }
            }
        }
    });

    // ── Receive AI response ───────────────────────────────────────────────────
    glib::spawn_future_local({
        let ai_buffer = ai_buffer.clone();
        let ai_stack = ai_stack.clone();
        let spinner = spinner.clone();
        async move {
            let mut accumulated = String::new();
            while let Ok(msg) = ai_result_rx.recv().await {
                match msg {
                    AiMsg::Start => {
                        accumulated.clear();
                        ai_buffer.set_text("");
                        spinner.start();
                        ai_stack.set_visible_child_name("loading");
                    }
                    AiMsg::Chunk(chunk) => {
                        if accumulated.is_empty() {
                            spinner.stop();
                            ai_stack.set_visible_child_name("response");
                        }
                        accumulated.push_str(&chunk);
                        let mut end = ai_buffer.end_iter();
                        ai_buffer.insert(&mut end, &chunk);
                    }
                    AiMsg::Done => {
                        spinner.stop();
                        ai_stack.set_visible_child_name("response");
                        render_markdown(&ai_buffer, &accumulated);
                    }
                }
            }
        }
    });

    // ── ShowUi signal ─────────────────────────────────────────────────────────
    glib::spawn_future_local({
        let window_weak = window.downgrade();
        let search_entry_weak = search_entry.downgrade();
        async move {
            while show_rx.recv().await.is_ok() {
                if let Some(win) = window_weak.upgrade() {
                    if let Some(e) = search_entry_weak.upgrade() {
                        e.set_text("");
                    }
                    // set_visible ensures the window is un-hidden before present()
                    // raises it; present() alone may not re-show a widget-hidden
                    // window on some GTK4 Wayland backends.
                    win.set_visible(true);
                    win.present();
                    if let Some(e) = search_entry_weak.upgrade() {
                        e.grab_focus();
                    }
                }
            }
        }
    });

    if std::env::var_os("OMNIMAN_SHOW").is_some() {
        window.present();
        search_entry.grab_focus();
    }

    window
}

fn start_ai_request(
    ai_req_tx: &async_channel::Sender<String>,
    ai_stack: &gtk4::Stack,
    spinner: &gtk4::Spinner,
    ai_buffer: &gtk4::TextBuffer,
    query: String,
) {
    ai_buffer.set_text("");
    ai_stack.set_visible_child_name("loading");
    spinner.start();
    let _ = ai_req_tx.try_send(query);
}

/// Populate `buffer` with markdown rendered to styled text via pulldown-cmark.
fn render_markdown(buffer: &gtk4::TextBuffer, text: &str) {
    use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

    buffer.set_text("");
    let mut iter = buffer.end_iter();

    // Active inline tag names (bold, italic, code_block). Applied to each text insertion.
    let mut tag_stack: Vec<&'static str> = Vec::new();
    // Stack of list kinds: None = unordered, Some(n) = ordered starting at n.
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    // Per-list item counter (incremented on each Item start).
    let mut item_counters: Vec<u64> = Vec::new();
    let mut in_list_item = false;
    let mut in_code_block = false;

    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(text, opts);

    for event in parser {
        match event {
            // ── Block opens ──────────────────────────────────────────────────
            Event::Start(Tag::Heading { level, .. }) => {
                let name: &'static str = match level {
                    HeadingLevel::H1 => "h1",
                    HeadingLevel::H2 => "h2",
                    _ => "h3",
                };
                tag_stack.push(name);
            }
            Event::Start(Tag::Strong) => tag_stack.push("bold"),
            Event::Start(Tag::Emphasis) => tag_stack.push("italic"),
            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
                tag_stack.push("code_block");
            }
            Event::Start(Tag::List(start)) => {
                list_stack.push(start);
                item_counters.push(start.unwrap_or(1));
            }
            Event::Start(Tag::Item) => {
                in_list_item = true;
                let prefix = match list_stack.last() {
                    Some(None) => "  • ".to_owned(),
                    Some(Some(_)) => {
                        let n = item_counters.last_mut().map(|c| {
                            let v = *c;
                            *c += 1;
                            v
                        }).unwrap_or(1);
                        format!("  {}. ", n)
                    }
                    None => "  • ".to_owned(),
                };
                buffer.insert_with_tags_by_name(&mut iter, &prefix, &["bullet"]);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                tag_stack.push("blockquote");
                buffer.insert_with_tags_by_name(&mut iter, "▎ ", &["blockquote"]);
            }
            Event::Start(Tag::Paragraph) => {}
            Event::Start(_) => {}

            // ── Block closes ─────────────────────────────────────────────────
            Event::End(TagEnd::Heading(_)) => {
                tag_stack.pop();
                buffer.insert(&mut iter, "\n\n");
            }
            Event::End(TagEnd::Strong) | Event::End(TagEnd::Emphasis) => {
                tag_stack.pop();
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                tag_stack.pop();
                buffer.insert(&mut iter, "\n");
            }
            Event::End(TagEnd::List(_)) => {
                list_stack.pop();
                item_counters.pop();
                buffer.insert(&mut iter, "\n");
            }
            Event::End(TagEnd::Item) => {
                in_list_item = false;
                buffer.insert(&mut iter, "\n");
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                tag_stack.pop();
            }
            Event::End(TagEnd::Paragraph) => {
                if !in_list_item {
                    buffer.insert(&mut iter, "\n\n");
                }
            }
            Event::End(_) => {}

            // ── Inline content ───────────────────────────────────────────────
            Event::Text(t) => {
                let tags: Vec<&str> = tag_stack.clone();
                if tags.is_empty() {
                    buffer.insert(&mut iter, &t);
                } else {
                    buffer.insert_with_tags_by_name(&mut iter, &t, &tags);
                }
            }
            Event::Code(t) => {
                buffer.insert_with_tags_by_name(&mut iter, &t, &["code"]);
            }
            Event::SoftBreak => {
                if in_code_block {
                    buffer.insert(&mut iter, "\n");
                } else {
                    buffer.insert(&mut iter, " ");
                }
            }
            Event::HardBreak => {
                buffer.insert(&mut iter, "\n");
            }
            Event::Rule => {
                buffer.insert(&mut iter, "\n");
            }
            _ => {}
        }
    }
}

fn setup_text_tags(buffer: &gtk4::TextBuffer) {
    let table = buffer.tag_table();

    let h1 = gtk4::TextTag::builder()
        .name("h1")
        .weight(700)
        .scale(1.5)
        .foreground("#f2f2f7")
        .build();
    let h2 = gtk4::TextTag::builder()
        .name("h2")
        .weight(700)
        .scale(1.3)
        .foreground("#f2f2f7")
        .build();
    let h3 = gtk4::TextTag::builder()
        .name("h3")
        .weight(700)
        .scale(1.1)
        .foreground("#f2f2f7")
        .build();
    let bold = gtk4::TextTag::builder()
        .name("bold")
        .weight(700)
        .build();
    let italic = gtk4::TextTag::builder()
        .name("italic")
        .style(gtk4::pango::Style::Italic)
        .build();
    let code = gtk4::TextTag::builder()
        .name("code")
        .family("monospace")
        .foreground("#a8d8a8")
        .background("rgba(0,0,0,0.3)")
        .build();
    let code_block = gtk4::TextTag::builder()
        .name("code_block")
        .family("monospace")
        .foreground("#a8d8a8")
        .background("rgba(0,0,0,0.4)")
        .left_margin(24)
        .build();
    let bullet = gtk4::TextTag::builder()
        .name("bullet")
        .foreground("#8e8ea0")
        .build();
    let blockquote = gtk4::TextTag::builder()
        .name("blockquote")
        .foreground("#8e8ea0")
        .style(gtk4::pango::Style::Italic)
        .left_margin(16)
        .build();

    table.add(&h1);
    table.add(&h2);
    table.add(&h3);
    table.add(&bold);
    table.add(&italic);
    table.add(&code);
    table.add(&code_block);
    table.add(&bullet);
    table.add(&blockquote);
}

fn load_css() {
    let provider = gtk4::CssProvider::new();
    provider.load_from_string(include_str!("style.css"));
    gtk4::style_context_add_provider_for_display(
        &gdk::Display::default().expect("no display"),
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn mime_icon(path: &str) -> &'static str {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "pdf" => "application-pdf",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" => "image-x-generic",
        "mp3" | "flac" | "ogg" | "wav" | "opus" => "audio-x-generic",
        "mp4" | "mkv" | "webm" | "avi" => "video-x-generic",
        "rs" | "py" | "js" | "ts" | "c" | "cpp" | "go" | "sh" => "text-x-script",
        "md" | "txt" | "org" => "text-x-generic",
        _ => "text-x-generic",
    }
}
