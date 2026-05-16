#!/usr/bin/env bash
# Remove the Omniman GNOME shortcut registered by setup-shortcut.sh.
set -euo pipefail

SCHEMA="org.gnome.settings-daemon.plugins.media-keys"
CUSTOM_SCHEMA="${SCHEMA}.custom-keybinding"
BINDING_ID="omniman"
BINDING_PATH="/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/${BINDING_ID}/"

# Reset the individual keybinding keys
gsettings reset "${CUSTOM_SCHEMA}:${BINDING_PATH}" name    2>/dev/null || true
gsettings reset "${CUSTOM_SCHEMA}:${BINDING_PATH}" command 2>/dev/null || true
gsettings reset "${CUSTOM_SCHEMA}:${BINDING_PATH}" binding 2>/dev/null || true

# Remove our path from the list
EXISTING=$(gsettings get "$SCHEMA" custom-keybindings)
NEW_LIST=$(echo "$EXISTING" \
    | sed "s|, *'${BINDING_PATH}'||g" \
    | sed "s|'${BINDING_PATH}' *, *||g" \
    | sed "s|'${BINDING_PATH}'||g")
gsettings set "$SCHEMA" custom-keybindings "$NEW_LIST"

echo "Omniman shortcut removed."
