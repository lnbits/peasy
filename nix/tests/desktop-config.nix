{
  pkgs,
  module,
  package,
}:
let
  evaluate =
    desktop:
    (import (pkgs.path + "/nixos/lib/eval-config.nix") {
      system = pkgs.stdenv.hostPlatform.system;
      modules = [
        module
        {
          services.peasy = {
            enable = true;
            inherit package;
          };
          system.stateVersion = "26.05";
        }
        desktop
      ];
    }).config;
  gnome = evaluate { services.desktopManager.gnome.enable = true; };
  plasma = evaluate { services.desktopManager.plasma6.enable = true; };
  hyprland = evaluate { programs.hyprland.enable = true; };
  generic = evaluate { };
  genericTray = cfg: cfg.environment.etc."xdg/autostart/peasy-tray.desktop".text;
  profile = cfg: builtins.fromJSON cfg.environment.etc."peasy/system-profile.json".text;
  validTray =
    cfg:
    !(pkgs.lib.hasInfix "OnlyShowIn" (genericTray cfg))
    && !(pkgs.lib.hasInfix "NotShowIn" (genericTray cfg))
    && pkgs.lib.hasInfix "/bin/peasy-tray --ui" (genericTray cfg);
in
assert pkgs.lib.all validTray [
  gnome
  plasma
  hyprland
  generic
];
assert (profile plasma).configured_desktops == [ "kde_plasma" ];
assert (profile gnome).configured_desktops == [ "gnome" ];
assert (profile hyprland).configured_desktops == [ "hyprland" ];
assert !(plasma.environment.etc ? "xdg/autostart/peasy-panel.desktop");
assert !(hyprland.environment.etc ? "xdg/autostart/peasy-panel.desktop");
assert !(plasma.services.desktopManager.gnome.enable);
assert !(builtins.elem pkgs.gnomeExtensions.appindicator plasma.environment.systemPackages);
assert !(builtins.elem pkgs.gnomeExtensions.appindicator hyprland.environment.systemPackages);
assert builtins.elem pkgs.gnomeExtensions.appindicator gnome.environment.systemPackages;
pkgs.runCommand "peasy-desktop-configuration-check" { } "touch $out"
