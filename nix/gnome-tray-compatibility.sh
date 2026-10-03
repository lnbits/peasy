# Sourced by the generated GNOME autostart script with absolute tool paths.
# Shell/extension discovery can lag XDG autostart on the first login.
"$timeout" 2 "$extensions" disable peasy@peasy-nixos.github.io || true
for ((attempt = 0; attempt < 15; attempt++)); do
    if "$timeout" 2 "$extensions" enable "$extension_uuid"; then
        exit 0
    fi
    "$sleep" 1
done
echo 'Peasy: GNOME tray support could not be enabled; check GNOME Extensions.' >&2
exit 1
