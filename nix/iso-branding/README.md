# ISO branding

Preserve upstream NixOS logos, installer layout and modules. Use pale green,
`#5cd698` selections and “Includes Peasy”. `default.nix` renders bundled assets
with pinned fonts; no runtime downloads are needed.

Branding applies to installation media only. Do not import it into the installed
system or change its bootloader.

```sh
nix build --no-link .#checks.x86_64-linux.installer-target .#checks.x86_64-linux.iso-config
```

Rebuild the ISO, then visually check BIOS/UEFI menus and the installer.
