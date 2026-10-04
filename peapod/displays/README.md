# Displays pea

Inspect and change GNOME, Plasma or Hyprland display layouts using exact discovered outputs and modes. Propose position, refresh/mode, supported scale and primary output where the desktop supports it. Preserve other outputs. Hyprland has no primary flag; use false. No output disabling, mirrored GNOME layout edits, custom modes, compositor configuration files or claims of NixOS rollback.

Positions range from −16384 to 16384. Select **Keep** within 20 seconds; timeout,
Revert, client closure or failed apply triggers watchdog restoration. Hardware or
compositor failure can prevent recovery.

[Native adapter](native.rs) · [Shared resource contract](../../docs/resources.md)
