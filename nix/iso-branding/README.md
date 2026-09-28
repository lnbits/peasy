# ISO branding contract

Applies to GNOME and Plasma installation media only. Preserve upstream NixOS
logos, installer layout and installation modules. Use pale green backgrounds,
`#5cd698` selection colour and the “Includes Peasy” caption.

| File | Purpose |
| --- | --- |
| `installer-welcome.svg` | NixOS logo and Peasy caption |
| `grub-background.svg`, `grub-selection.svg` | GRUB colour swatches |
| `grub-credit.txt` | Footer appended to the upstream theme |
| `default.nix` | Render assets with pinned fonts and customize the theme |

[Boot branding](../iso-boot-branding.nix) applies to the ISO boot menu.
[Installer integration](../installer.nix) retains upstream slides, translations,
icons and desktop previews. Assets are resolved at build time without runtime
network requests. Do not import ISO boot branding into the installed target or
the ordinary Peasy module; it must not configure the installed bootloader.

```console
nix build --no-link .#checks.x86_64-linux.installer-target .#checks.x86_64-linux.iso-config
```

For visual acceptance, rebuild the ISO, inspect BIOS and UEFI menus, and open the
installer. Existing ISO files are not updated in place.
