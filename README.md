# Omniman

A Spotlight-like launcher for **Arch Linux + Wayland + GNOME**. Press `Ctrl+Space`, type to search files, browse clipboard history, or ask an AI question — all from a single floating bar.

![Files tab](docs/screenshot-files.png)

## Features

- **File search** — fuzzy filename search over `$HOME` via a live Tantivy index with inotify-based incremental updates.
- **Clipboard history** — persistent history of text and file URIs. Press Enter to copy back to clipboard.
- **AI chat** — multi-turn conversations with Gemini or any OpenAI-compatible endpoint. Streaming responses with Markdown rendering. Conversations are persisted in SQLite.
- **Native GNOME integration** — registered as a GNOME custom keybinding, no portal dance, works out of the box on Mutter ≥ 45.

## Requirements

| Dependency | Minimum version | Notes |
|---|---|---|
| Rust + Cargo | 1.78 | `rustup` recommended |
| GTK4 | 4.12 | `gtk4` package |
| libadwaita | 1.5 | `libadwaita` package |
| GNOME / Mutter | 45 | Wayland session |
| SQLite | 3 | `sqlite` package |
| gdbus | any | Part of `glib2` |

On Arch Linux:
```bash
sudo pacman -S gtk4 libadwaita sqlite dbus xdg-utils glib2
```

## Installation

### Option A — PKGBUILD (Arch Linux, recommended)

```bash
git clone https://github.com/adrien/omniman
cd omniman

# Build and install the package
makepkg -si
```

`makepkg -si` builds, then calls `pacman -U` to install. pacman will print
post-install instructions automatically.

After install, complete setup:

```bash
# 1. Enable the daemon
systemctl --user enable --now omnimand

# 2. Register Ctrl+Space as the global shortcut
/usr/share/omniman/setup-shortcut.sh
# or a custom key:
/usr/share/omniman/setup-shortcut.sh '<Super>space'

# 3. Auto-start the UI at login
cp /usr/share/applications/omniman.desktop ~/.config/autostart/
# The UI stays resident in the background (hide-on-close).
# To show the window immediately on launch, set OMNIMAN_SHOW=1 in the desktop entry.
```

The daemon performs an incremental index sweep on every start (only new or
modified files), so startup is fast. A full crawl can be triggered via the
`Reindex` D-Bus method. Check progress:
```bash
journalctl --user -u omnimand -f
```

### Option B — Manual (from source)

```bash
git clone https://github.com/adrien/omniman
cd omniman
cargo build --release --workspace
```

```bash
# Install binaries
install -Dm755 target/release/omnimand ~/.local/bin/omnimand
install -Dm755 target/release/omniman  ~/.local/bin/omniman

# Install and enable the systemd unit
install -Dm644 data/systemd/omnimand.service ~/.config/systemd/user/omnimand.service
systemctl --user daemon-reload
systemctl --user enable --now omnimand

# Register the global shortcut
bash data/setup-shortcut.sh

# Auto-start the UI at login (stays resident, hide-on-close)
cp data/omniman.desktop ~/.config/autostart/
```

### AI API key (optional)

Without a key the Files and Clipboard tabs work fully; the AI tab is disabled.

You can use either **Gemini** (Google AI Studio) or any **OpenAI-compatible** endpoint.

#### Gemini

Set the API key via environment variable:
```bash
systemctl --user edit omnimand
```
Add under `[Service]`:
```ini
Environment=GEMINI_API_KEY=your_key_here
```

Or export `GEMINI_API_KEY` in your session environment (`~/.config/environment.d/`).

Alternatively, set `gemini_api_key` directly in `~/.config/omniman/config.toml`.

#### OpenAI-compatible endpoint

Configure in `~/.config/omniman/config.toml`:
```toml
[ai]
openai_endpoint = "https://api.openai.com/v1"
openai_key = "your_key_here"
openai_model = "gpt-4o"
```

## Usage

Press **`Ctrl+Space`** (or your configured shortcut) to open the launcher.

| Key | Action |
|---|---|
| Type | Search files (debounced, 200 ms) |
| `Enter` (search entry) | Send query as AI chat message |
| `Enter` (file row) | Open selected file with `xdg-open` |
| `Enter` (clipboard row) | Copy entry back to clipboard via `wl-copy` |
| `↓` | Move focus to results list |
| `Ctrl+Tab` | Cycle tabs: Files → Clipboard → AI |
| `Ctrl+L` | Jump back to search field |
| `Escape` | Hide the launcher |

The window also hides automatically 150 ms after losing focus.

### Tabs

**Files** — fuzzy search over filenames in `$HOME`. Selecting a result opens it with `xdg-open`.

**Clipboard** — shows recent clipboard entries. Selecting one runs `wl-copy` to put it back on the clipboard.

**AI** — appears when the query looks like a question (`?` suffix, >5 words, or starts with an interrogative word such as `how`, `what`, `why`, `comment`, `quoi`, `qui`, etc.). Click **Ask AI** or press Enter in the search entry to start a multi-turn conversation. Responses stream with Markdown rendering. Conversations are persisted and accessible from a sidebar. Supports both Gemini and OpenAI-compatible endpoints.

## Configuration

Config file: `~/.config/omniman/config.toml` (created on first write via Preferences).

```toml
[index]
# Directories to skip during crawl (relative to $HOME)
exclude_dirs = [".cache", ".git", "node_modules", "target", ".venv", "__pycache__", ".cargo"]

[clipboard]
# Maximum number of entries to retain
history_limit = 500

[ai]
# Gemini model to use
model = "gemini-2.5-flash"
# Gemini API key (or set GEMINI_API_KEY environment variable)
# gemini_api_key = "your_key_here"

# OpenAI-compatible endpoint (alternative to Gemini)
# openai_endpoint = "https://api.openai.com/v1"
# openai_key = "your_key_here"
# openai_model = "gpt-4o"

[hotkey]
# Informational only — the actual shortcut lives in gsettings (setup-shortcut.sh)
shortcut = "Ctrl+Space"
```

Data directory: `~/.local/share/omniman/`
- `index/` — Tantivy search index
- `clipboard.db` — clipboard history (SQLite)
- `chat.db` — AI conversation history (SQLite)

## Architecture

```
Ctrl+Space
    │
    ▼
GNOME keybinding (gsettings)
    │  gdbus call RequestShowUi
    ▼
omnimand  ──── D-Bus session bus ────  omniman (GTK4 UI)
(systemd)   ShowUi signal →             win.present()
            Search / ClipboardHistory ← search entry
            AskAI ←                     AI tab
```

Two processes communicate over D-Bus (`org.adrien.OmnimanDaemon`):

- **`omnimand`** — indexes files, watches clipboard, answers D-Bus method calls, emits `ShowUi` and `ClipboardChanged` signals. Exposes methods: `Search`, `ClipboardHistory`, `AskAI`, `RequestShowUi`, `ListModels`, `Reindex`.
- **`omniman`** — renders results, hides/shows on demand, stays alive with `hide-on-close`.

The daemon also supports D-Bus auto-activation via `org.adrien.OmnimanDaemon.service`.

## Development

```bash
# Run daemon in foreground (verbose)
RUST_LOG=omnimand=debug cargo run -p omniman-daemon

# Run UI and show it immediately
OMNIMAN_SHOW=1 cargo run -p omniman-ui

# Manually trigger show (daemon must be running)
gdbus call --session \
  --dest org.adrien.OmnimanDaemon \
  --object-path /org/adrien/Omniman \
  --method org.adrien.Omniman1.RequestShowUi

# Tests
cargo test --workspace

# Force full re-index (without deleting index dir)
gdbus call --session \
  --dest org.adrien.OmnimanDaemon \
  --object-path /org/adrien/Omniman \
  --method org.adrien.Omniman1.Reindex

# Reset index (forces full re-crawl on next daemon start)
rm -rf ~/.local/share/omniman/index/
```

## License

MIT
