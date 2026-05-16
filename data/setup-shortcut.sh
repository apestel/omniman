#!/usr/bin/env bash
# Register (or update) the Omniman global shortcut in GNOME via gsettings.
# Run once after installing omniman.  Safe to re-run — idempotent.
set -euo pipefail

SCHEMA="org.gnome.settings-daemon.plugins.media-keys"
CUSTOM_SCHEMA="${SCHEMA}.custom-keybinding"
BINDING_ID="omniman"
BINDING_PATH="/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/${BINDING_ID}/"
SHORTCUT="${1:-<Control>space}"

# The GNOME keybinding command calls RequestShowUi on the daemon via D-Bus.
# omnimand must be running (systemd --user start omnimand); omniman (UI) must
# also be running and listening for the ShowUi signal.
COMMAND="gdbus call --session \
  --dest org.adrien.OmnimanDaemon \
  --object-path /org/adrien/Omniman \
  --method org.adrien.Omniman1.RequestShowUi"

# ── Add our path to the custom-keybindings list if not already present ────────
EXISTING=$(gsettings get "$SCHEMA" custom-keybindings)
if [[ "$EXISTING" != *"$BINDING_PATH"* ]]; then
    if [[ "$EXISTING" == "@as []" || "$EXISTING" == "[]" ]]; then
        NEW_LIST="['${BINDING_PATH}']"
    else
        # Strip trailing ] and append
        NEW_LIST="${EXISTING%]}, '${BINDING_PATH}']"
    fi
    gsettings set "$SCHEMA" custom-keybindings "$NEW_LIST"
fi

# ── Set name / command / binding ──────────────────────────────────────────────
gsettings set "${CUSTOM_SCHEMA}:${BINDING_PATH}" name    "Omniman"
gsettings set "${CUSTOM_SCHEMA}:${BINDING_PATH}" command "$COMMAND"
gsettings set "${CUSTOM_SCHEMA}:${BINDING_PATH}" binding "$SHORTCUT"

echo "Shortcut registered: ${SHORTCUT} → Omniman"
echo "To change the shortcut:  $0 '<Super>space'"
echo "To remove:               data/remove-shortcut.sh"
