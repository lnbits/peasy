# Desktop compatibility and audit

The core daemon, typed protocol, bounded zero-import Wasm policy, package
management, proposal tokens and administrator authorization are shared across
desktops. GTK/libadwaita is the UI toolkit, not a requirement to run GNOME Shell.

| Desktop | Tray host | Appearance through Peasy |
| --- | --- | --- |
| GNOME | AppIndicator extension, enabled only on configured GNOME | Light/dark/system-default, nine accent colours |
| KDE Plasma | Built-in StatusNotifier host | BreezeLight/BreezeDark, nine accent colours |
| Hyprland | User's compatible bar, e.g. Waybar tray | Existing bounded gaps/borders/radius/opacity/blur/animation controls and safe dispatchers |
| XFCE / LXQt | Compatible StatusNotifier host required | Explicitly unsupported |
| Other / headless | Host required / no graphical tray | Explicitly unsupported |

Generic light/dark/accent actions are not mapped speculatively to Hyprland, XFCE
or LXQt. Wallpaper changes requested through AI are unsupported. The live ISO's
bundled wallpaper and green accent are trusted build-time defaults, not a new
model capability. Plasma's system-default mode is unsupported rather than guessed.

## What was audited and changed

The audit covered Rust crates, Nix modules/packages/tests, assets, website and
documentation, including desktop names, gsettings/dconf, session environment,
autostart filters, GTK, calendar/ICS and appearance references.

| Area | Previous assumption | Current implementation |
| --- | --- | --- |
| System profile | GNOME/Hyprland only | Bounded normalized GNOME, Plasma, Hyprland, XFCE, LXQt, other/headless enum |
| Session detection | Only current-desktop plus Hyprland substring/signature | Exact allowlisted colon-separated tokens; three standard session variables; no raw environment sent to AI |
| Tray module | GNOME panel launcher plus Hyprland-only SNI autostart | One existing SNI binary, generic XDG autostart; GNOME-only AppIndicator compatibility |
| Appearance | Generic enums described/applied as GNOME-only | Small fixed GNOME/Plasma adapters, explicit capabilities; existing Hyprland controls retained |
| Generated Nix | Unconditional GNOME dconf on theme changes | GNOME defaults gated on GNOME configuration; shared generation theme record retained |
| Calendar | Generic ICS/GIO, unclear handler failure | Still ICS/default application, now UTC DTSTAMP, UTF-8 line folding and recoverable-file error |
| Documentation/UI copy | GNOME theme/panel claims | Desktop-neutral where supported, desktop-specific limitations explicit |

Legacy GNOME extension assets remain packaged for compatibility, but the module
disables their launcher on GNOME upgrade. The separate GNOME-only autostart file
enables the standard tray host; it is not a second Peasy tray. GTK/GLib and the
existing optional Hyprland Polkit agent are legitimate dependencies, not GNOME
tray dependencies imposed on Plasma.

## Trusted appearance routing

`ThemeSettings` still contains only closed colour and mode enums. Native code
checks the actual session's capabilities before proposal creation and again
before applying it. The existing Wasm/approval/system-generation flow is intact.

GNOME uses fixed `org.gnome.desktop.interface` keys. Plasma uses its installed
`plasma-apply-colorscheme`: fixed Breeze scheme names and fixed colour literals.
The one fixed `kdeglobals` → `General` → `AccentColor` key is saved with
`kwriteconfig6`, because the upstream accent CLI recolours the palette without
persisting that preference. No other KDE file/group/key is exposed.
Scheme and accent are separate calls because the upstream CLI treats them as
separate modes ([KDE implementation](https://github.com/KDE/plasma-workspace/blob/master/kcms/colors/plasma-apply-colorscheme.cpp)).
The model never selects executable paths, arguments, gsettings/KDE keys, config
files, D-Bus methods or scripts. Unsupported adapters do not fall back to GNOME.
The module does not pull Plasma tools into GNOME or GNOME Shell into Plasma.

Peasy keeps one managed Nix source file and the existing generation snapshot,
not a new settings database. The graphical-session sync service reapplies that
generation's typed appearance to the active supported desktop. Desktop-native
settings can still differ by user, and direct desktop changes are not claimed to
be NixOS state. A saved system-default GNOME mode is rejected on Plasma until a
supported explicit light/dark mode is chosen.

## Calendar and session integration

Events are private `.ics` files with validated title/start/duration, escaped text,
UTC DTSTAMP and folded UTF-8 lines. Start time remains floating local time. Fixed
`gio open` uses the freedesktop default application for `text/calendar`, not a
hardcoded GNOME Calendar or KDE PIM service. No Akonadi dependency is added.
If there is no handler, the error identifies the retained file for manual import;
successful launch does not claim the calendar has already imported the event.

GNOME and Plasma VMs exercise the same SNI ID, host registration, activation of
the real UI, provider setup without credentials, ISO appearance defaults, and
live theme changes. Hyprland/other configurations have evaluation and typed-action
tests; this is not a claim of runtime testing every compositor or tray host.
