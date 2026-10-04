# Desktop support

| Desktop | Tray | Appearance |
| --- | --- | --- |
| GNOME | AppIndicator extension, enabled by Peasy's module | Accent, light/dark/system-default |
| Plasma | Built-in StatusNotifier host | Accent, light/dark |
| Hyprland | Compatible bar, such as Waybar with `tray` | Supported live compositor controls |
| XFCE / LXQt / other | Compatible StatusNotifier host required | No appearance adapter |
| Headless | CLI only | None |

Package management and other core functions are shared across desktops. Wallpaper
changes through AI are unsupported. Display controls have [separate limits](resources.md).

## Missing tray

Log out and back in after installing or updating the module. Ensure XDG autostart
runs and a tray host is available. GNOME needs the AppIndicator extension; Hyprland
needs its bar's tray. Peasy remains available from the application menu.

## Desktop behaviour

- Appearance follows the active generation. Removing Peasy's managed settings
  restores recorded user preferences; settings overwritten before recording was
  introduced may need a manual reset.
- GNOME and XFCE application discovery refreshes after package changes. Existing
  XFCE sessions need one logout/login after first adopting the patched library.
- Calendar events open in the default `text/calendar` application. Opening an event
  is not proof it was imported; a failed handoff retains the file for manual import.

Desktop VM tests check startup, registration and supported operations. They do not
prove tray visibility or compatibility with every compositor and hardware setup.
