# Omniman

A Spotlight-like launcher for **Arch Linux + Wayland + GNOME**. Press `Ctrl+Space`, type to search files, browse clipboard history, or ask an AI question — all from a single floating bar.

![Files tab](docs/screenshot-files.png)

## Features

- **File search** — full-text-aware search over `$HOME` via a live Tantivy index. Results appear in under 100 ms.
- **Clipboard history** — persistent history of text, images, and file URIs. Press Enter to copy back to clipboard.
- **AI assistant** — queries that look like questions are routed to Gemini (Google AI Studio). Streaming response rendered in the AI tab.
- **Native GNOME integration** — registered as a GNOME custom keybinding, no portal dance, works out of the box on Mutter ≥ 45.

## Requirements

| Dependency | Minimum version | Notes |
|---|---|---|
| Rust + Cargo | 1.78 | `rustup` recommended |
| GTK4 | 4.12 | `gtk4` package |
| libadwaita | 1.5 | `libadwaita` package |
| GNOME / Mutter | 45 | Wayland session |
| SQLite | 3 | `sqlite` package |
| wl-clipboard | any | `wl-clipboard` package |
| gdbus | any | Part of `glib2` |

On Arch Linux:
```bash
sudo pacman -S gtk4 libadwaita sqlite wl-clipboard
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
```

The daemon indexes `$HOME` on first start (background, non-blocking). Check
progress:
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

# Auto-start the UI at login
cp data/omniman.desktop ~/.config/autostart/
```

### Gemini API key (optional)

Without a key the Files and Clipboard tabs work fully; the AI tab is disabled.

```bash
# Store the key in the Secret Service (never written to disk in plaintext)
secret-tool store --label "Omniman Gemini key" service omniman key gemini_api_key
```

To pass the key to the daemon, create a drop-in override:
```bash
systemctl --user edit omnimand
```
Add under `[Service]`:
```ini
Environment=GEMINI_API_KEY=your_key_here
```

Or export `GEMINI_API_KEY` in your session environment (`~/.config/environment.d/`).

## Usage

Press **`Ctrl+Space`** (or your configured shortcut) to open the launcher.

| Key | Action |
|---|---|
| Type | Search files (debounced, 200 ms) |
| `Enter` | Open selected file / copy clipboard entry |
| `↓` | Move focus to results list |
| `Ctrl+Tab` | Cycle tabs: Files → Clipboard → AI |
| `Ctrl+Enter` | Ask AI with the current query |
| `Ctrl+L` | Jump back to search field |
| `Escape` | Hide the launcher |

### Tabs

**Files** — searches filenames in `$HOME`. Selecting a result opens it with `xdg-open`.

**Clipboard** — shows the last 50 clipboard entries. Selecting one runs `wl-copy` to put it back on the clipboard.

**AI** — appears automatically when the query looks like a question (`?` suffix, starts with `how`/`what`/`why`/`comment`/`quoi`/`qui`, or is longer than 5 words). Click **Ask AI** or press `Ctrl+Enter` to submit. The response streams into the panel with basic Markdown rendering.

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

[hotkey]
# Informational only — the actual shortcut lives in gsettings (setup-shortcut.sh)
shortcut = "Ctrl+Space"
```

Data directory: `~/.local/share/omniman/`
- `index/` — Tantivy search index
- `clipboard.db` — clipboard history (SQLite)

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

- **`omnimand`** — indexes files, watches clipboard, answers D-Bus method calls, emits `ShowUi`.
- **`omniman`** — renders results, hides/shows on demand, stays alive with `hide-on-close`.

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

# Reset index (forces full re-crawl on next daemon start)
rm -rf ~/.local/share/omniman/index/
```

## License

MIT
