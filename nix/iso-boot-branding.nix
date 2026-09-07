{ pkgs, ... }:
let
  artwork = import ./iso-branding { inherit pkgs; };
in
{
  # Deliberately not imported by installer-target.nix or the manual module.
  # Only the installation medium's menus change, never the installed loader.
  isoImage.grubTheme = "${artwork}/grub";
  # Also identify Peasy on legacy BIOS media, without replacing that menu.
  isoImage.appendToMenuLabel = " Installer - Includes Peasy";
}
