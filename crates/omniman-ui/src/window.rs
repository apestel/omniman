use std::{cell::{Cell, RefCell}, rc::Rc, time::Duration};

use gtk4::{gdk, glib, prelude::*};
use libadwaita::prelude::*;
use omniman_ai::ChatTurn;
use omniman_core::{config::Config, types::{ClipEntry, Hit}};

use crate::{
    chat::ChatStore,
    ChatMsg, ChatReq,
};

pub fn build(
    app: &libadwaita::Application,
    query_tx: async_channel::Sender<String>,
    result_rx: async_channel::Receiver<Vec<Hit>>,
    show_rx: async_channel::Receiver<()>,
    clip_req_tx: async_channel::Sender<()>,
    clip_result_rx: async_channel::Receiver<Vec<ClipEntry>>,
    chat_req_tx: async_channel::Sender<ChatReq>,
    chat_msg_rx: async_channel::Receiver<ChatMsg>,
) -> libadwaita::ApplicationWindow {
    load_css();

    // Open chat store — Config::data_dir() resolves XDG_DATA_HOME/omniman/
    let chat_db_path = Config::data_dir().join("chat.db");
    let chat_store = Rc::new(RefCell::new(
        ChatStore::open(&chat_db_path).expect("open chat.db"),
    ));

    let window = libadwaita::ApplicationWindow::builder()
        .application(app)
        .title("Omniman")
        .default_width(720)
        .default_height(520)
        .decorated(false)
        .css_classes(["omniman-launcher"])
        .hide_on_close(true)
        .build();

    // ── Root layout ───────────────────────────────────────────────────────────
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.add_css_class("omniman-root");

    // ── Search row ────────────────────────────────────────────────────────────
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

    // ── AI panel: sidebar + thread pane ──────────────────────────────────────
    let ai_panel = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);

    // — Sidebar —
    let sidebar = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    sidebar.add_css_class("omniman-chat-sidebar");

    let conv_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Single)
        .css_classes(["omniman-conv-list"])
        .build();
    let conv_scroll = gtk4::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(&conv_list)
        .build();

    let new_chat_btn = gtk4::Button::builder()
        .label("+ New chat")
        .css_classes(["omniman-newchat-btn", "flat"])
        .build();

    sidebar.append(&conv_scroll);
    sidebar.append(&new_chat_btn);

    // — Thread pane —
    let thread_pane = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    thread_pane.set_hexpand(true);

    let thread_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    thread_box.add_css_class("omniman-chat-thread");

    let thread_scroll = gtk4::ScrolledWindow::builder()
        .vexpand(true)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .child(&thread_box)
        .build();

    // — Input row —
    let input_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    input_row.add_css_class("omniman-chat-input");

    let chat_entry = gtk4::Entry::builder()
        .placeholder_text("Continue conversation…")
        .hexpand(true)
        .css_classes(["omniman-chat-entry"])
        .build();

    let send_btn = gtk4::Button::builder()
        .label("Send")
        .css_classes(["suggested-action", "omniman-send-btn"])
        .build();

    input_row.append(&chat_entry);
    input_row.append(&send_btn);

    thread_pane.append(&thread_scroll);
    thread_pane.append(&input_row);

    ai_panel.append(&sidebar);
    ai_panel.append(&gtk4::Separator::new(gtk4::Orientation::Vertical));
    ai_panel.append(&thread_pane);

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

    // ── State ─────────────────────────────────────────────────────────────────
    // current_conv_id: None = next user turn creates a new conversation
    let current_conv_id: Rc<Cell<Option<i64>>> = Rc::new(Cell::new(None));
    // in-flight streaming state: buffer + optional typing-dots widget
    let pending_assistant: Rc<RefCell<Option<(gtk4::TextBuffer, Option<gtk4::Box>)>>> =
        Rc::new(RefCell::new(None));
    // true while a request is in flight (prevents double-sends)
    let streaming: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    // holds the title placeholder for the current new conversation
    let new_conv_title_row: Rc<RefCell<Option<(i64, libadwaita::ActionRow)>>> =
        Rc::new(RefCell::new(None));

    // ── Sticky-bottom scroll state ───────────────────────────────────────────
    // True when the user is parked at the bottom and wants new content to
    // follow. Flips false the moment the user scrolls up.
    let at_bottom: Rc<Cell<bool>> = Rc::new(Cell::new(true));
    // Set while we perform a programmatic scroll so the value-changed handler
    // doesn't misread it as user intent.
    let scroll_lock: Rc<Cell<bool>> = Rc::new(Cell::new(false));

    let scroll_to_bottom: Rc<dyn Fn()> = {
        let scroll = thread_scroll.clone();
        let lock = Rc::clone(&scroll_lock);
        let at_bottom = Rc::clone(&at_bottom);
        Rc::new(move || {
            let adj = scroll.vadjustment();
            if adj.page_size() <= 0.0 { return; }
            lock.set(true);
            adj.set_value(adj.upper() - adj.page_size());
            lock.set(false);
            at_bottom.set(true);
        })
    };

    {
        let adj = thread_scroll.vadjustment();
        adj.connect_value_changed({
            let at_bottom = Rc::clone(&at_bottom);
            let lock = Rc::clone(&scroll_lock);
            move |a| {
                if lock.get() { return; }
                let bottom = (a.upper() - a.page_size()).max(0.0);
                at_bottom.set(bottom < 1.0 || (bottom - a.value()).abs() < 1.0);
            }
        });
        // Follow growing content during streaming (sticky-bottom).
        adj.connect_notify_local(Some("upper"), {
            let scroll_to_bottom = Rc::clone(&scroll_to_bottom);
            let at_bottom = Rc::clone(&at_bottom);
            move |_, _| { if at_bottom.get() { scroll_to_bottom(); } }
        });
    }

    // ── Load existing conversations into sidebar ───────────────────────────────
    {
        let convs = chat_store
            .borrow()
            .list_conversations(200)
            .unwrap_or_default();
        for conv in convs {
            let row = make_conv_row(&conv.title, conv.id);
            conv_list.append(&row);
        }
    }

    // ── Gear → preferences ───────────────────────────────────────────────────
    let prefs_open: Rc<Cell<bool>> = Rc::new(Cell::new(false));
    gear_btn.connect_clicked({
        let window_weak = window.downgrade();
        let prefs_open = prefs_open.clone();
        move |_| {
            let parent = window_weak.upgrade();
            prefs_open.set(true);
            let prefs_win =
                crate::prefs::build(parent.as_ref().map(|w| w.upcast_ref::<gtk4::Window>()));
            let flag = prefs_open.clone();
            prefs_win.connect_destroy(move |_| flag.set(false));
        }
    });

    // ── Helper: send a chat turn ──────────────────────────────────────────────
    // Returns false if nothing was sent (empty text or already streaming).
    let send_turn = {
        let chat_req_tx = chat_req_tx.clone();
        let chat_store = Rc::clone(&chat_store);
        let current_conv_id = Rc::clone(&current_conv_id);
        let pending_assistant = Rc::clone(&pending_assistant);
        let streaming = Rc::clone(&streaming);
        let thread_box = thread_box.clone();
        let conv_list = conv_list.clone();
        let new_conv_title_row = Rc::clone(&new_conv_title_row);

        Rc::new(move |text: String| -> bool {
            let text = text.trim().to_owned();
            if text.is_empty() || streaming.get() {
                return false;
            }
            streaming.set(true);

            // Ensure there's a current conversation
            let conv_id = if let Some(id) = current_conv_id.get() {
                id
            } else {
                let id = chat_store
                    .borrow()
                    .create_conversation("New conversation")
                    .expect("create conversation");
                current_conv_id.set(Some(id));
                // Add placeholder row to top of sidebar
                let row = make_conv_row("New conversation", id);
                if let Some(first) = conv_list.row_at_index(0) {
                    conv_list.insert(&row, 0);
                    let _ = first; // keep borrow alive
                } else {
                    conv_list.append(&row);
                }
                conv_list.select_row(conv_list.row_at_index(0).as_ref());
                new_conv_title_row.borrow_mut().replace((id, row));
                id
            };

            // Persist user message
            if let Err(e) = chat_store.borrow().append_message(conv_id, "user", &text) {
                tracing::warn!(conv_id, "failed to persist user message: {e}");
            }

            // Build history from store
            let stored = chat_store.borrow().messages(conv_id).unwrap_or_else(|e| {
                tracing::warn!(conv_id, "failed to load messages: {e}");
                vec![]
            });
            let history: Vec<ChatTurn> = stored
                .iter()
                .map(|m| {
                    if m.role == "user" {
                        ChatTurn::user(&m.content)
                    } else {
                        ChatTurn::assistant(&m.content)
                    }
                })
                .collect();

            tracing::debug!(conv_id, turns = history.len(), "sending NewTurn");

            // Render user bubble
            append_user_bubble(&thread_box, &text);

            // Create empty assistant bubble with typing-dots indicator
            let buf = gtk4::TextBuffer::new(None);
            setup_text_tags(&buf);
            let dots = append_assistant_bubble(&thread_box, &buf);
            *pending_assistant.borrow_mut() = Some((buf, Some(dots)));
            // Scroll follow is handled by the notify::upper handler when sticky.

            if let Err(e) = chat_req_tx.try_send(ChatReq::NewTurn { conv_id, history }) {
                tracing::warn!(conv_id, "chat_req channel full, dropping NewTurn: {e}");
                streaming.set(false);
                return false;
            }
            true
        })
    };

    // ── "Ask AI" button ───────────────────────────────────────────────────────
    ask_btn.connect_clicked({
        let search_entry = search_entry.clone();
        let btn_ai = btn_ai.clone();
        let send_turn = Rc::clone(&send_turn);
        let current_conv_id = Rc::clone(&current_conv_id);
        move |_| {
            let query = search_entry.text().to_string();
            if query.trim().is_empty() { return; }
            // Ask AI button always starts a new conversation
            current_conv_id.set(None);
            btn_ai.set_active(true);
            send_turn(query);
        }
    });

    // ── Tab switching ─────────────────────────────────────────────────────────
    btn_files.connect_toggled({
        let stack = stack.clone();
        move |btn| { if btn.is_active() { stack.set_visible_child_name("files"); } }
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
        move |btn| {
            if btn.is_active() {
                stack.set_visible_child_name("ai");
            }
        }
    });

    // ── Debounced file search ─────────────────────────────────────────────────
    let debounce: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    search_entry.connect_changed({
        let debounce = Rc::clone(&debounce);
        let query_tx = query_tx.clone();
        let btn_files = btn_files.clone();
        move |entry| {
            if let Some(id) = debounce.take() { id.remove(); }
            let text = entry.text().to_string();
            if text.trim().is_empty() { return; }
            if !btn_files.is_active() { btn_files.set_active(true); }
            let qt = query_tx.clone();
            let debounce_cb = Rc::clone(&debounce);
            let id = glib::timeout_add_local_once(Duration::from_millis(200), move || {
                debounce_cb.set(None);
                let _ = qt.try_send(text);
            });
            debounce.set(Some(id));
        }
    });

    // ── search_entry Enter → new conversation ─────────────────────────────────
    search_entry.connect_activate({
        let btn_ai = btn_ai.clone();
        let send_turn = Rc::clone(&send_turn);
        let current_conv_id = Rc::clone(&current_conv_id);
        let search_entry = search_entry.clone();
        move |entry| {
            let query = entry.text().to_string();
            if query.trim().is_empty() { return; }
            // Top entry always starts a new conversation
            current_conv_id.set(None);
            btn_ai.set_active(true);
            // Clear immediately so user sees the empty field
            search_entry.set_text("");
            send_turn(query);
        }
    });

    search_entry.connect_stop_search({
        let window_weak = window.downgrade();
        move |_entry| {
            if let Some(win) = window_weak.upgrade() { win.set_visible(false); }
        }
    });

    // ── Bottom chat entry ─────────────────────────────────────────────────────
    let send_from_chat_entry = {
        let send_turn = Rc::clone(&send_turn);
        let chat_entry = chat_entry.clone();
        Rc::new(move || {
            let text = chat_entry.text().to_string();
            if send_turn(text) {
                chat_entry.set_text("");
            }
        })
    };

    chat_entry.connect_activate({
        let f = Rc::clone(&send_from_chat_entry);
        move |_| f()
    });

    send_btn.connect_clicked({
        let f = Rc::clone(&send_from_chat_entry);
        move |_| f()
    });

    // ── "＋ New chat" button ───────────────────────────────────────────────────
    new_chat_btn.connect_clicked({
        let current_conv_id = Rc::clone(&current_conv_id);
        let thread_box = thread_box.clone();
        let search_entry = search_entry.clone();
        let conv_list = conv_list.clone();
        move |_| {
            current_conv_id.set(None);
            clear_thread(&thread_box);
            conv_list.unselect_all();
            search_entry.grab_focus();
        }
    });

    // ── Conversation list click → load thread ─────────────────────────────────
    conv_list.connect_row_activated({
        let chat_store = Rc::clone(&chat_store);
        let current_conv_id = Rc::clone(&current_conv_id);
        let thread_box = thread_box.clone();
        let thread_scroll = thread_scroll.clone();
        let chat_entry = chat_entry.clone();
        let btn_ai = btn_ai.clone();
        let stack = stack.clone();
        let at_bottom = Rc::clone(&at_bottom);
        let scroll_to_bottom = Rc::clone(&scroll_to_bottom);
        move |_, row| {
            let conv_id = conv_id_from_row(row);
            current_conv_id.set(Some(conv_id));
            // Reset offset and force sticky mode for the new conversation.
            thread_scroll.vadjustment().set_value(0.0);
            at_bottom.set(true);
            clear_thread(&thread_box);
            let msgs = chat_store.borrow().messages(conv_id).unwrap_or_default();
            for msg in &msgs {
                if msg.role == "user" {
                    append_user_bubble(&thread_box, &msg.content);
                } else {
                    let buf = gtk4::TextBuffer::new(None);
                    setup_text_tags(&buf);
                    render_markdown(&buf, &msg.content);
                    let dots = append_assistant_bubble(&thread_box, &buf);
                    dots.unparent();
                }
            }
            btn_ai.set_active(true);
            stack.set_visible_child_name("ai");
            chat_entry.grab_focus();
            // Tick callbacks fire at the *end* of each rendered frame, when
            // GTK has finished measuring & allocating everything for that
            // frame. Wrapped TextViews need a couple of frames to converge,
            // so we re-pin to the bottom for the first ~3 frames after a
            // conversation load, then disconnect.
            let remaining: Rc<Cell<u32>> = Rc::new(Cell::new(3));
            thread_box.add_tick_callback({
                let scroll_to_bottom = Rc::clone(&scroll_to_bottom);
                let remaining = Rc::clone(&remaining);
                move |_, _| {
                    scroll_to_bottom();
                    let n = remaining.get();
                    if n <= 1 {
                        glib::ControlFlow::Break
                    } else {
                        remaining.set(n - 1);
                        glib::ControlFlow::Continue
                    }
                }
            });
        }
    });

    // ── Right-click on conv row → delete ─────────────────────────────────────
    // Attached per-row in make_conv_row_with_menu below — we wire delete here.
    // We use a signal on the conv_list itself via GestureClick so we always
    // have access to chat_store / current_conv_id.
    {
        let chat_store = Rc::clone(&chat_store);
        let current_conv_id = Rc::clone(&current_conv_id);
        let thread_box_weak = thread_box.downgrade();
        let conv_list_weak = conv_list.downgrade();
        let delete_gesture = gtk4::GestureClick::new();
        delete_gesture.set_button(3);
        delete_gesture.connect_pressed({
            let chat_store = Rc::clone(&chat_store);
            let current_conv_id = Rc::clone(&current_conv_id);
            move |_gesture, _, _x, y| {
                let Some(conv_list) = conv_list_weak.upgrade() else { return };
                let Some(row) = conv_list.row_at_y(y as i32) else { return };
                let conv_id = conv_id_from_row(&row);

                // Build a simple popover with a delete button
                let delete_btn = gtk4::Button::builder()
                    .label("Delete")
                    .css_classes(["destructive-action"])
                    .build();
                let popover = gtk4::Popover::new();
                popover.set_child(Some(&delete_btn));
                popover.set_parent(&row);
                popover.popup();

                let store = Rc::clone(&chat_store);
                let cid = Rc::clone(&current_conv_id);
                let thread_box_weak2 = thread_box_weak.clone();
                let row_weak = row.downgrade();
                let popover_weak = popover.downgrade();
                delete_btn.connect_clicked(move |_| {
                    store.borrow().delete(conv_id).ok();
                    if cid.get() == Some(conv_id) {
                        cid.set(None);
                        if let Some(tb) = thread_box_weak2.upgrade() { clear_thread(&tb); }
                    }
                    if let Some(r) = row_weak.upgrade() {
                        if let Some(lb) = r.parent().and_then(|p| p.downcast::<gtk4::ListBox>().ok()) {
                            lb.remove(&r);
                        }
                    }
                    if let Some(p) = popover_weak.upgrade() { p.popdown(); }
                });
            }
        });
        conv_list.add_controller(delete_gesture);
    }

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
                while let Some(child) = files_list.first_child() { files_list.remove(&child); }
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
                while let Some(child) = clip_list.first_child() { clip_list.remove(&child); }
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

    // ── Receive chat messages ─────────────────────────────────────────────────
    glib::spawn_future_local({
        let pending_assistant = Rc::clone(&pending_assistant);
        let streaming = Rc::clone(&streaming);
        let chat_store = Rc::clone(&chat_store);
        let chat_req_tx = chat_req_tx.clone();
        let conv_list = conv_list.clone();
        let new_conv_title_row = Rc::clone(&new_conv_title_row);
        let chat_entry = chat_entry.clone();
        async move {
            let countdown_timer: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
            while let Ok(msg) = chat_msg_rx.recv().await {
                match msg {
                    ChatMsg::Start { .. } => {
                        if let Some(id) = countdown_timer.take() { id.remove(); }
                    }
                    ChatMsg::Chunk { text, .. } => {
                        if let Some((_, dots_opt)) = pending_assistant.borrow_mut().as_mut() {
                            if let Some(dots) = dots_opt.take() {
                                dots.unparent();
                            }
                        }
                        if let Some((buf, _)) = pending_assistant.borrow().as_ref() {
                            animate_into(buf, &text).await;
                        }
                        // notify::upper handler auto-scrolls when at_bottom.
                    }
                    ChatMsg::Done { conv_id, full_text } => {
                        tracing::debug!(conv_id, chars = full_text.len(), "Done received");
                        // Remove any remaining typing-dots then apply markdown rendering
                        if let Some((_, dots_opt)) = pending_assistant.borrow_mut().as_mut() {
                            if let Some(dots) = dots_opt.take() {
                                dots.unparent();
                            }
                        }
                        if let Some((buf, _)) = pending_assistant.borrow().as_ref() {
                            render_markdown(buf, &full_text);
                        }
                        *pending_assistant.borrow_mut() = None;
                        // thread_box.size_allocate fires after the re-render's
                        // layout pass and pins to the true bottom if at_bottom.

                        // Persist assistant message
                        if let Err(e) = chat_store.borrow().append_message(conv_id, "assistant", &full_text) {
                            tracing::warn!(conv_id, "failed to persist assistant message: {e}");
                        }

                        streaming.set(false);
                        chat_entry.grab_focus();

                        // Request title generation if this is the first reply for a new conv
                        let msg_count = chat_store.borrow().message_count(conv_id).unwrap_or(0);
                        if msg_count == 2 {
                            // exactly one user + one assistant message
                            let msgs = chat_store.borrow().messages(conv_id).unwrap_or_default();
                            if let (Some(u), Some(a)) = (msgs.first(), msgs.get(1)) {
                                let _ = chat_req_tx.try_send(ChatReq::Summarize {
                                    conv_id,
                                    user_msg: u.content.clone(),
                                    assistant_msg: a.content.clone(),
                                });
                            }
                        }
                    }
                    ChatMsg::RateLimit { conv_id: _, secs } => {
                        // Remove typing-dots and replace with rate-limit message
                        if let Some((_, dots_opt)) = pending_assistant.borrow_mut().as_mut() {
                            if let Some(dots) = dots_opt.take() {
                                dots.unparent();
                            }
                        }
                        if let Some((buf, _)) = pending_assistant.borrow().as_ref() {
                            buf.set_text(&format!("Rate limited — retry in {secs}s"));
                        }
                        *pending_assistant.borrow_mut() = None;
                        streaming.set(false);

                        let remaining = Rc::new(Cell::new(secs));
                        let id = glib::timeout_add_local(Duration::from_secs(1), move || {
                            let r = remaining.get().saturating_sub(1);
                            remaining.set(r);
                            if r == 0 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
                        });
                        countdown_timer.set(Some(id));
                    }
                    ChatMsg::Title { conv_id, title } => {
                        // Update DB
                        chat_store.borrow().rename(conv_id, &title).ok();
                        // Update sidebar row label if it's the placeholder row
                        let guard = new_conv_title_row.borrow();
                        if let Some((id, row)) = guard.as_ref() {
                            if *id == conv_id {
                                row.set_title(&title);
                            }
                        }
                        drop(guard);
                        // Also update any existing row in conv_list that matches
                        let mut child = conv_list.row_at_index(0);
                        while let Some(row) = child {
                            if conv_id_from_row(&row) == conv_id {
                                if let Some(ar) = row.downcast_ref::<libadwaita::ActionRow>() {
                                    ar.set_title(&title);
                                }
                                break;
                            }
                            child = row.next_sibling()
                                .and_then(|w| w.downcast::<gtk4::ListBoxRow>().ok());
                        }
                    }
                }
            }
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
        let window_weak = window.downgrade();
        key_ctrl.connect_key_pressed(move |_, key, _, mods| {
            match key {
                gdk::Key::Escape => {
                    if let Some(win) = window_weak.upgrade() { win.set_visible(false); }
                    glib::Propagation::Stop
                }
                gdk::Key::l if mods.contains(gdk::ModifierType::CONTROL_MASK) => {
                    search_entry.grab_focus();
                    search_entry.select_region(0, -1);
                    glib::Propagation::Stop
                }
                gdk::Key::Tab if mods.contains(gdk::ModifierType::CONTROL_MASK) => {
                    if btn_files.is_active() { btn_clipboard.set_active(true); }
                    else if btn_clipboard.is_active() { btn_ai.set_active(true); }
                    else { btn_files.set_active(true); }
                    glib::Propagation::Stop
                }
                gdk::Key::Down => {
                    let active = stack.visible_child_name().unwrap_or_default();
                    let list = if active == "files" { &files_list } else { &clip_list };
                    if let Some(row) = list.row_at_index(0) { row.grab_focus(); }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
    }
    window.add_controller(key_ctrl);

    // ── Hide on focus loss ────────────────────────────────────────────────────
    {
        let hide_timer: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
        window.connect_is_active_notify(move |win| {
            if win.is_active() {
                if let Some(id) = hide_timer.take() { id.remove(); }
                return;
            }
            let win_weak = win.downgrade();
            let timer = hide_timer.clone();
            let prefs_open = prefs_open.clone();
            let new_id = glib::timeout_add_local_once(Duration::from_millis(150), move || {
                timer.set(None);
                if prefs_open.get() { return; }
                if let Some(win) = win_weak.upgrade() {
                    if !win.is_active() { win.set_visible(false); }
                }
            });
            if let Some(old) = hide_timer.replace(Some(new_id)) { old.remove(); }
        });
    }

    // ── ShowUi signal ─────────────────────────────────────────────────────────
    glib::spawn_future_local({
        let window_weak = window.downgrade();
        let search_entry_weak = search_entry.downgrade();
        let clip_req_tx = clip_req_tx.clone();
        async move {
            while show_rx.recv().await.is_ok() {
                if let Some(win) = window_weak.upgrade() {
                    win.set_visible(true);
                    win.present();
                    if let Some(e) = search_entry_weak.upgrade() {
                        e.grab_focus();
                        e.set_position(-1);
                    }
                    let _ = clip_req_tx.try_send(());
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

// ── Widget helpers ────────────────────────────────────────────────────────────

fn make_conv_row(title: &str, conv_id: i64) -> libadwaita::ActionRow {
    let row = libadwaita::ActionRow::builder()
        .title(glib::markup_escape_text(title))
        .activatable(true)
        .build();
    // Store conv_id as widget name (cheapest way without unsafe data)
    row.set_widget_name(&conv_id.to_string());
    row
}

fn conv_id_from_row(row: &gtk4::ListBoxRow) -> i64 {
    row.widget_name().parse::<i64>().unwrap_or(-1)
}

fn clear_thread(thread_box: &gtk4::Box) {
    while let Some(child) = thread_box.first_child() {
        thread_box.remove(&child);
    }
}

fn append_user_bubble(thread_box: &gtk4::Box, text: &str) {
    let bubble = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    bubble.add_css_class("omniman-bubble-user");
    bubble.set_hexpand(true);

    let label = gtk4::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(gtk4::pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .hexpand(true)
        .build();

    bubble.append(&label);
    thread_box.append(&bubble);
}

fn append_assistant_bubble(thread_box: &gtk4::Box, buffer: &gtk4::TextBuffer) -> gtk4::Box {
    let bubble = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    bubble.add_css_class("omniman-bubble-assistant");
    bubble.set_hexpand(true);

    let dots = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    for extra_class in [None, Some("d2"), Some("d3")] {
        let lbl = gtk4::Label::new(Some("●"));
        let mut classes = vec!["omniman-typing-dot"];
        if let Some(c) = extra_class { classes.push(c); }
        lbl.set_css_classes(&classes);
        dots.append(&lbl);
    }
    bubble.append(&dots);

    let text_view = gtk4::TextView::builder()
        .buffer(buffer)
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk4::WrapMode::WordChar)
        .hexpand(true)
        .left_margin(4)
        .right_margin(4)
        .top_margin(2)
        .bottom_margin(2)
        .css_classes(["omniman-ai-text"])
        .build();

    bubble.append(&text_view);
    thread_box.append(&bubble);
    dots
}

async fn animate_into(buffer: &gtk4::TextBuffer, chunk: &str) {
    for word in chunk.split_inclusive(|c: char| c.is_whitespace()) {
        let mut end = buffer.end_iter();
        buffer.insert(&mut end, word);
        glib::timeout_future(Duration::from_millis(8)).await;
    }
}

// ── Markdown rendering (unchanged) ───────────────────────────────────────────

fn render_markdown(buffer: &gtk4::TextBuffer, text: &str) {
    use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

    buffer.set_text("");
    let mut iter = buffer.end_iter();

    let mut tag_stack: Vec<&'static str> = Vec::new();
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    let mut item_counters: Vec<u64> = Vec::new();
    let mut in_list_item = false;
    let mut in_code_block = false;

    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(text, opts);

    for event in parser {
        match event {
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
            Event::Start(Tag::CodeBlock(_)) => { in_code_block = true; tag_stack.push("code_block"); }
            Event::Start(Tag::List(start)) => {
                list_stack.push(start);
                item_counters.push(start.unwrap_or(1));
            }
            Event::Start(Tag::Item) => {
                in_list_item = true;
                let prefix = match list_stack.last() {
                    Some(None) => "  • ".to_owned(),
                    Some(Some(_)) => {
                        let n = item_counters.last_mut().map(|c| { let v = *c; *c += 1; v }).unwrap_or(1);
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
            Event::Start(Tag::Paragraph) | Event::Start(_) => {}
            Event::End(TagEnd::Heading(_)) => { tag_stack.pop(); buffer.insert(&mut iter, "\n\n"); }
            Event::End(TagEnd::Strong) | Event::End(TagEnd::Emphasis) => { tag_stack.pop(); }
            Event::End(TagEnd::CodeBlock) => { in_code_block = false; tag_stack.pop(); buffer.insert(&mut iter, "\n"); }
            Event::End(TagEnd::List(_)) => { list_stack.pop(); item_counters.pop(); buffer.insert(&mut iter, "\n"); }
            Event::End(TagEnd::Item) => { in_list_item = false; buffer.insert(&mut iter, "\n"); }
            Event::End(TagEnd::BlockQuote(_)) => { tag_stack.pop(); }
            Event::End(TagEnd::Paragraph) => { if !in_list_item { buffer.insert(&mut iter, "\n\n"); } }
            Event::End(_) => {}
            Event::Text(t) => {
                let tags: Vec<&str> = tag_stack.clone();
                if tags.is_empty() { buffer.insert(&mut iter, &t); }
                else { buffer.insert_with_tags_by_name(&mut iter, &t, &tags); }
            }
            Event::Code(t) => { buffer.insert_with_tags_by_name(&mut iter, &t, &["code"]); }
            Event::SoftBreak => { buffer.insert(&mut iter, if in_code_block { "\n" } else { " " }); }
            Event::HardBreak => { buffer.insert(&mut iter, "\n"); }
            Event::Rule => { buffer.insert(&mut iter, "\n"); }
            _ => {}
        }
    }
}

fn setup_text_tags(buffer: &gtk4::TextBuffer) {
    let table = buffer.tag_table();

    macro_rules! tag {
        ($name:expr, $($prop:ident : $val:expr),+ $(,)?) => {{
            let t = gtk4::TextTag::builder().name($name) $( .$prop($val) )+ .build();
            table.add(&t);
        }};
    }

    tag!("h1", weight: 700, scale: 1.5, foreground: "#f2f2f7");
    tag!("h2", weight: 700, scale: 1.3, foreground: "#f2f2f7");
    tag!("h3", weight: 700, scale: 1.1, foreground: "#f2f2f7");
    tag!("bold", weight: 700);
    tag!("italic", style: gtk4::pango::Style::Italic);
    tag!("code", family: "monospace", foreground: "#a8d8a8", background: "rgba(0,0,0,0.3)");
    tag!("code_block", family: "monospace", foreground: "#a8d8a8", background: "rgba(0,0,0,0.4)", left_margin: 24);
    tag!("bullet", foreground: "#8e8ea0");
    tag!("blockquote", foreground: "#8e8ea0", style: gtk4::pango::Style::Italic, left_margin: 16);
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
