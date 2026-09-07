# ISO branding

Shared by the GNOME and Plasma installation media. The upstream NixOS GRUB
design and logo stay, with a pale green background, **#5cd698** selection colour
and **Includes Peasy** below the menu. The installer keeps the NixOS branding
and layout, with a green sidebar and **Includes Peasy** below the main logo.

- `installer-welcome.svg`: upstream NixOS logo with the Includes Peasy caption.
- `grub-background.svg` and `grub-selection.svg`: GRUB colour swatches.
- `grub-credit.txt`: small footer appended to the upstream theme.
- `default.nix`: renders the SVGs with pinned fonts and customises the theme.

`../iso-boot-branding.nix` applies the boot-menu artwork only to the ISO.
`../installer.nix` customises upstream Calamares branding without replacing its
installation modules, slides, translations or desktop previews. The installer's
window and sidebar icons remain upstream's. All artwork is resolved at build
time; there are no network requests to load logos in the installer.

These files do not configure the installed system's bootloader. Do not import
the ISO boot module from `installer-target.nix` or the ordinary Peasy module.

Run the focused checks:

```console
nix build --no-link .#checks.x86_64-linux.installer-target .#checks.x86_64-linux.iso-config
```

For visual acceptance, rebuild
an ISO and check both BIOS and UEFI menus, then open the installer without
starting a disk installation. Existing ISO files are not modified in place.
