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

## Application search

GNOME application search follows the selected NixOS generation without copying or
renaming desktop launchers into users' home directories. Nixpkgs' GLib profile
watcher can notify GNOME before activation makes a new application's executable
available. The fixed `peasy-applications-refresh` system service watches the
containing `/run` directory for replacement of `/run/current-system`, and updates
only the system-profile symlink's timestamp
(not its store target) to notify that existing watcher after activation. It
accepts no AI input and does not restart the desktop. This also covers ordinary
switches and rollbacks while the Peasy desktop module is enabled.

The GNOME VM regression test checks the running Shell's cached app list after
install, removal and rollback, including a slow-activation negative control and
duplicate-ID checks. Shell introspection is enabled only in that disposable
test VM, never in the ISO or installed system.

XFCE uses Garcon rather than GNOME's application cache. Its application-directory
monitors also follow symlinks into the old immutable generation. On Peasy-enabled
XFCE systems only, the module applies `nix/patches/garcon-nixos-generation.patch`:
Garcon watches for replacement of `/run/current-system`, discards stale menu items,
and emits its existing coalesced reload signal. Unrelated `/run` events are ignored.
This uses the existing menu process, without polling, launcher copies, or panel
restarts. The patch requires an initial rebuild of Garcon and its XFCE dependents;
it adds no extra Peasy compilation to ordinary package installs.
An existing XFCE session must log out and back in once after first adopting the
patched library; subsequent application installs/removals do not require logout.

The XFCE VM regression test keeps the real App Finder open and checks its visible
search results across the same slow profile/activation sequence, removal, and
rollback. It also checks for duplicate results and unchanged App Finder/panel
process IDs. OCR is a test-driver dependency only, not part of the installed system.

## Export dialog

The graphical package supplies GTK's runtime schema paths through Nixpkgs'
`wrapGAppsHook4`, including `org.gtk.gtk4.Settings.FileChooser`. Export must not
depend on settings-schema paths inherited from a particular desktop or terminal.
The desktop VM tests open and cancel the real export folder picker twice after
launching Peasy with empty inherited data paths. They use a disposable host
configuration and never accept an export destination. Accessibility tooling is
only a test dependency; the headless package does not gain GTK runtime wrapping.

## Calendar and session integration

Events are private `.ics` files with validated title/start/duration, escaped text,
UTC DTSTAMP and folded UTF-8 lines. Start time remains floating local time. Fixed
`gio open` uses the freedesktop default application for `text/calendar`, not a
hardcoded GNOME Calendar or KDE PIM service. No Akonadi dependency is added.
If there is no handler, the error identifies the retained file for manual import;
successful launch does not claim the calendar has already imported the event.

GNOME and Plasma VMs exercise the same SNI ID, host registration, activation of
the real UI, provider setup without credentials, ISO appearance defaults, and
live theme changes. The XFCE VM checks autostart, SNI registration, UI activation,
bundled export-source availability, and rejection of unsupported appearance changes.
The module explicitly links `/share/peasy` for export rather than relying on a
desktop linking every shared-data directory. Hyprland/other configurations have evaluation and typed-action
tests; this is not a claim of runtime testing every compositor or tray host.
