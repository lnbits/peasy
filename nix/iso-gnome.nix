{ modulesPath, ... }:
{
  imports = [
    (modulesPath + "/installer/cd-dvd/installation-cd-graphical-calamares-gnome.nix")
    ./iso-common.nix
  ];
}
