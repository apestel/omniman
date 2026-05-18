# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Implementation in progress. Core crates are written and building. Always read `roadmap.md` first: it is the source of truth for architecture, scope, and validated decisions.

## What this project is

**Omniman** — a Spotlight-like floating search bar for Linux Wayland/GNOME, written in Rust. Combines:
- Filename/metadata search over an indexed `$HOME` (Tantivy).
- Auto-routed AI questions to Google AI Studio (Gemini) when the query looks like a natural-language question.
- Persistent clipboard history (text, images, file URIs).

Target platform is **Arch Linux + Wayland + GNOME (Mutter)**. This constrains tooling — see "Platform constraints" below.

## Architecture

Two-process design:

- **`omnimand`** (daemon) — long-running systemd user service. Owns the Tantivy index, the clipboard SQLite store, AI clients (Gemini + OpenAI-compatible), and exposes a D-Bus interface on `org.adrien.OmnimanDaemon` (zbus) with methods `Search`, `ClipboardHistory`, `StoreClipEntry`, `ChatStreaming`, `RequestShowUi` and signals `ShowUi`, `ClipboardChanged`, `ChatChunk`, `ChatDone`, `ChatError`.
- **`omniman`** (GTK4 client) — runs persistently in the background (hide-on-close). Connects over D-Bus, listens for the `ShowUi` signal, renders results, dispatches actions (open file, paste clip). AI requests are routed through the daemon via `ChatStreaming` D-Bus method + `ChatChunk`/`ChatDone`/`ChatError` signals.

Cargo workspace under `crates/`:

| Crate | Role |
|---|---|
| `omniman-core` | shared types, config schema, D-Bus interface definitions |
| `omniman-index` | Tantivy index + `notify` watcher |
| `omniman-clipboard` | SQLite store only (clipboard polling moved to UI via GTK4) |
| `omniman-ai` | Gemini + OpenAI-compatible REST clients + question heuristic |
| `omniman-daemon` | `omnimand` binary |
| `omniman-ui` | `omniman` binary (GTK4 + libadwaita) |

## Global shortcut — gsettings approach

The hotkey is registered as a **GNOME custom keybinding** via gsettings, not via the XDG GlobalShortcuts portal (the portal requires a valid Wayland surface as parent for its consent dialog, which the daemon cannot provide).

Run once after install:
```bash
data/setup-shortcut.sh          # registers Ctrl+Space (default)
data/setup-shortcut.sh '<Super>space'   # or a custom trigger
data/remove-shortcut.sh         # removes it
```

When the shortcut fires, GNOME executes:
```bash
gdbus call --session --dest org.adrien.OmnimanDaemon \
  --object-path /org/adrien/Omniman \
  --method org.adrien.Omniman1.RequestShowUi
```

`RequestShowUi` is a regular D-Bus method on the daemon that emits the `ShowUi` broadcast signal. The UI process receives the signal and calls `win.set_visible(true)` + `win.present()`.

## Platform constraints (important)

- **Mutter does not implement `wlr-layer-shell`.** Do not pull in `gtk4-layer-shell`, `layer-shika`, or any wlroots-only crate. The launcher window is a regular `xdg-toplevel`. Tools like Anyrun/Fuzzel/tofi/Walker would not work here — that's why this project exists.
- **Hotkey** is registered via GNOME gsettings (`data/setup-shortcut.sh`), not via the XDG GlobalShortcuts portal. The portal approach was attempted but GNOME rejects `bind_shortcuts` without a live Wayland surface as parent.
- **Clipboard** polling runs in the UI process via GTK4's `gdk4::Clipboard::read_text_future()` (uses standard `data-device` protocol, works on Mutter). The daemon only owns the SQLite store and exposes `StoreClipEntry` / `ClipboardHistory` D-Bus methods.
- **API keys** (Gemini, OpenAI) are stored in `~/.config/omniman/config.toml` and managed via the UI preferences. The daemon reads them at startup; the UI has no direct access to keys.

## Key decisions already locked

These were chosen with the user and should not be revisited without asking:

- GUI: GTK4 + libadwaita (gtk4-rs), not iced/Slint.
- Index: custom Tantivy, not TinySPARQL/Tracker3.
- Hotkey: GNOME gsettings custom keybinding (`data/setup-shortcut.sh`) → `gdbus call RequestShowUi` → daemon emits `ShowUi` → UI presents.
- IPC: zbus over D-Bus session bus.
- Clipboard scope: text + images + file URIs, persisted on disk.
- AI trigger: heuristic on query (`?` suffix, interrogative prefix `qui|quoi|comment|why|how|what`, > 5 words) shows an "Ask AI" button; explicit only.

## Build & run

```bash
cargo build --release --workspace
cargo test --workspace
cargo run -p omniman-daemon            # foreground daemon (dev)
OMNIMAN_SHOW=1 cargo run -p omniman-ui # show UI immediately on launch
```

Single test: `cargo test -p <crate> <test_name>`.

Service install: `data/systemd/omnimand.service` → `~/.config/systemd/user/`, then `systemctl --user enable --now omnimand`.

## UI behaviour

- Window hides on Escape (via `connect_stop_search` when search entry is focused; key controller fallback otherwise).
- Window shows on `ShowUi` D-Bus signal: `set_visible(true)` + `present()` + `search_entry.grab_focus()`.
- `hide_on_close(true)` — closing the window keeps the process alive.

## Implementation order

Follow the phases in `roadmap.md` — they're ordered to keep each step demoable end-to-end:
1. Workspace skeleton + D-Bus ping
2. Tantivy indexing + `Search` method
3. Minimal GTK4 UI wired to `Search`
4. Global shortcut via gsettings → `RequestShowUi` → `ShowUi` signal
5. Clipboard watcher + tab
6. Gemini client + AI tab
7. Preferences, packaging, tests
