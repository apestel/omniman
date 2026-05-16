# Roadmap: Omniman — Spotlight-like Rust app (Wayland/GNOME)

## Context

Build a Rust desktop application providing:
1. A Spotlight-like floating search bar triggered by a global keyboard shortcut.
2. Full-text-aware search over an indexed `$HOME` (filenames + metadata).
3. Auto-detection of natural-language questions → Google AI Studio (Gemini) call.
4. Clipboard manager with persistent history (text, images, file URIs).

Target: Arch Linux récent, Wayland session, GNOME (Mutter). Project directory `/home/adrien/projects/omniman` is empty (greenfield).

Why custom: existing tools (Onagre, Anyrun, Walker, Rofi 2.0) are launcher-only or rely on `wlr-layer-shell` which **Mutter does not implement**. Combining indexing + AI + clipboard in one tool with first-class Mutter support warrants a dedicated app.

## Existing tools (reference, not adopted)

- **Onagre** — Rust+iced, Spotlight-like, X11+Wayland, plugin via pop-launcher. Closest analogue.
- **Anyrun** — Wayland-native, plugin-based, krunner-like. Uses `wlr-layer-shell` (KO Mutter).
- **Walker / Fuzzel / tofi** — wlroots only.
- **Rofi 2.0 beta (Aug 2025)** — gained Wayland support.
- **Pop Launcher (System76)** — daemon backend, IPC over stdio. Used by Onagre.
- **Clipcat / Clapboard / Wayclip / cliphist** — clipboard managers (pas de search ni AI).
- **GNOME Tracker3 / TinySPARQL** — already running, indexes `$HOME` ; pas réutilisé (choix utilisateur : Tantivy).

## Decisions (validées)

| Domaine | Choix |
|---|---|
| Architecture | Daemon (systemd user service) + client GUI à la demande |
| GUI | GTK4 + libadwaita (gtk4-rs) |
| Hotkey global | GNOME gsettings custom keybinding → `gdbus call RequestShowUi` → daemon emits `ShowUi` signal |
| Indexation | Tantivy custom, scope `$HOME` avec exclusions standards (.cache, node_modules, .git, target, .venv…) |
| AI | Gemini via `gemini-rs` ou `google-generative-ai-rs`. Trigger heuristique (?, mots interrogatifs, longueur) |
| Clipboard | wl-clipboard-rs / `wl-clipboard` watcher. Texte + images + URI fichiers, persistance disque chiffrée |
| IPC daemon ↔ client | D-Bus session bus (zbus) |
| Config | TOML (xdg-config) via `figment` ou `serde + toml` |

## Architecture

```
omniman/
├── Cargo.toml                    # workspace
├── crates/
│   ├── omniman-core/             # types partagés, config, IPC schema
│   ├── omniman-index/            # Tantivy + watcher inotify (notify crate)
│   ├── omniman-clipboard/        # daemon clipboard, store sqlite chiffré
│   ├── omniman-ai/               # client Gemini, heuristique question
│   ├── omniman-daemon/           # binaire `omnimand` (systemd user service)
│   └── omniman-ui/               # binaire `omniman` (GTK4 client)
├── data/
│   ├── omniman.desktop           # entry XDG
│   └── systemd/omnimand.service  # unit user
└── README.md
```

### Daemon (`omnimand`)

Long-running process. Démarré au login via systemd user service.

Responsabilités :
- **Indexation** : crawl initial `$HOME` (respecte exclusions), maintien incrémental via `notify` (inotify). Index Tantivy stocké dans `$XDG_DATA_HOME/omniman/index/`. Schéma : path, filename, parent, mtime, size, mime, content_excerpt (option future).
- **Clipboard watcher** : utilise `wl-paste --watch` ou `wl-clipboard-rs` (protocole `zwlr_data_control_manager_v1`, supporté par Mutter ≥ 45). Stocke historique dans SQLite (`rusqlite`) chiffré (clé via libsecret/Secret Service).
- **Bus D-Bus** : expose `org.adrien.Omniman` (zbus) avec méthodes `Search(query)`, `ClipboardHistory(limit)`, `AskAI(prompt)`, signal `ShowUi`.

### Client UI (`omniman`)

Process GTK4 lancé/réveillé à la demande.

Comportement :
- Au lancement : connexion D-Bus, attend signal `ShowUi` ou s'affiche immédiatement si `--toggle`.
- Fenêtre `Adw.ApplicationWindow`, sans décoration, centrée, `set_modal(true)`, transparente, blur via CSS Adwaita.
- **Note Mutter** : pas de `wlr-layer-shell`. La fenêtre sera positionnée comme un `xdg-toplevel` flottant. Aucun moyen propre de la fixer toujours-au-dessus sous Mutter pur ; acceptable pour un launcher invoqué à la demande.
- Champ recherche `Adw.EntryRow` + `ListView` résultats.
- Onglets/filtres : Files | Clipboard | AI (Tab pour switcher).
- Heuristique AI : bouton "Ask AI" apparaît si query ressemble à question (regex `\?$`, prefix `qui|quoi|comment|why|how|what`, longueur > 5 mots).
- Sélection résultat : `Enter` → ouvre fichier (xdg-open) / colle clipboard / affiche réponse AI streaming.

### Crates Rust clés

| Fonction | Crate |
|---|---|
| GUI | `gtk4`, `libadwaita`, `gtk4-rs` (workspace gtk-rs) |
| D-Bus | `zbus` (async, pure-Rust) |
| Index | `tantivy` |
| File watcher | `notify` |
| Clipboard | `wl-clipboard-rs` |
| HTTP/Gemini | `reqwest` + `gemini-rs` (ou impl directe REST) |
| SQLite | `rusqlite` + `rusqlite_migration` |
| Crypto clé | `secret-service` (libsecret) |
| Config | `serde`, `toml`, `directories` (XDG paths) |
| Async runtime | `tokio` |
| Logs | `tracing` + `tracing-subscriber` |
| Erreurs | `thiserror` (libs), `anyhow` (binaires) |

## Phases d'implémentation

### Phase 1 — Squelette workspace + IPC
- `cargo new --bin` pour chaque crate, workspace `Cargo.toml`.
- Schéma D-Bus zbus dans `omniman-core`.
- Daemon vide qui écoute, client vide qui ping.
- Service systemd user, `.desktop` entry.

### Phase 2 — Indexation
- `omniman-index` : init Tantivy, config exclusions.
- Crawl initial concurrent (rayon) du `$HOME`.
- Watcher `notify` pour mises à jour incrémentales.
- Méthode D-Bus `Search(query) -> Vec<Hit>`.

### Phase 3 — UI GTK4 minimale
- Fenêtre Adwaita avec `EntryRow` + `ListView`.
- Connexion D-Bus, recherche debounced (200 ms).
- Sélection ouvre fichier via `xdg-open`.
- CSS sombre style Spotlight.

### Phase 4 — Hotkey (gsettings)
- `data/setup-shortcut.sh` enregistre keybinding GNOME (défaut `Ctrl+Space`).
- Shortcut exécute `gdbus call RequestShowUi` sur le daemon.
- Daemon émet signal `ShowUi` ; client présente fenêtre, focus champ.

### Phase 5 — Clipboard
- Watcher `wl-clipboard-rs` (protocole `zwlr_data_control_manager_v1`).
- Store SQLite, table `clip_entries (id, kind, content, mime, created_at)`.
- Chiffrement at-rest via clé Secret Service.
- Onglet UI Clipboard, paste = `wl-copy` puis simulation `Ctrl+V` (via portal RemoteDesktop si dispo, sinon copy seulement).

### Phase 6 — AI Studio (Gemini)
- Crate `omniman-ai` : client REST `generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent`.
- Clé API : variable env `GEMINI_API_KEY` ou Secret Service.
- Streaming SSE pour réponse progressive.
- Heuristique question : module `is_question(query: &str) -> bool`.
- UI : panneau réponse avec rendu markdown (gtk4 `TextView` + tags).

### Phase 7 — Polish
- Préférences (`Adw.PreferencesWindow`) : raccourci, exclusions index, modèle Gemini, taille historique clipboard.
- Empty states, animations, raccourcis clavier (Tab, Esc, Ctrl+L).
- Packaging : `PKGBUILD` Arch + flatpak manifest.
- Tests : unitaires sur indexation/heuristique, intégration daemon via `tokio::test`.

## Fichiers critiques à créer

- `/home/adrien/projects/omniman/Cargo.toml` (workspace)
- `/home/adrien/projects/omniman/crates/omniman-core/src/lib.rs` (types IPC, config)
- `/home/adrien/projects/omniman/crates/omniman-index/src/lib.rs` (Tantivy)
- `/home/adrien/projects/omniman/crates/omniman-clipboard/src/lib.rs`
- `/home/adrien/projects/omniman/crates/omniman-ai/src/lib.rs`
- `/home/adrien/projects/omniman/crates/omniman-daemon/src/main.rs`
- `/home/adrien/projects/omniman/crates/omniman-ui/src/main.rs`
- `/home/adrien/projects/omniman/data/systemd/omnimand.service`
- `/home/adrien/projects/omniman/data/omniman.desktop`
- `/home/adrien/projects/omniman/README.md`

## Risques / points d'attention

- **Mutter sans layer-shell** : la fenêtre ne peut pas être "vraiment toujours-au-dessus" comme Spotlight ; acceptable car invoquée à la demande. Alternative future : extension GNOME Shell pour positioning précis.
- **GlobalShortcuts portal sous GNOME** : disponible depuis xdg-desktop-portal-gnome ≥ 45 ; vérifier au runtime, fallback message si absent.
- **`zwlr_data_control_manager_v1` sous Mutter** : protocole supporté depuis GNOME 45 ; vérifier sur la version Arch installée. Sinon fallback `wl-paste --watch` (process séparé).
- **Quota Gemini free tier** : limiter rate (1 appel par requête utilisateur, debounce 500 ms après dernière frappe).
- **Sécurité clé API** : jamais en clair dans config TOML. Secret Service obligatoire.
- **Permissions FS** : daemon tourne en user, pas de privilèges spéciaux.

## Vérification end-to-end

1. `cargo build --release` au root du workspace réussit sans warning fatal.
2. `systemctl --user start omnimand` → service actif, `journalctl --user -u omnimand` montre crawl initial terminé.
3. Touche `Ctrl+Space` → fenêtre apparaît centrée, focus sur champ recherche.
4. Taper un nom de fichier dans `~/Documents` → résultats < 100 ms, Enter ouvre le fichier.
5. Ctrl+Tab → onglet Clipboard montre 10 dernières entrées texte + images.
6. Taper `?comment trier un Vec en Rust` → bouton "Ask AI" actif, Enter → réponse Gemini streamée.
7. Réinitialiser : `rm -rf ~/.local/share/omniman/` puis relancer le daemon → ré-indexation propre.
8. Tests : `cargo test --workspace`.
