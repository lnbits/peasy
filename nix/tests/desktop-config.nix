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
  xfce = evaluate { services.xserver.desktopManager.xfce.enable = true; };
  generic = evaluate { };
  headless = evaluate { services.peasy.desktop.enable = false; };
  headlessXfce = evaluate {
    services.peasy.desktop.enable = false;
    services.xserver.desktopManager.xfce.enable = true;
  };
  genericTray = cfg: cfg.environment.etc."xdg/autostart/peasy-tray.desktop".text;
  profile = cfg: builtins.fromJSON cfg.environment.etc."peasy/system-profile.json".text;
  validTray =
    cfg:
    !(pkgs.lib.hasInfix "OnlyShowIn" (genericTray cfg))
    && !(pkgs.lib.hasInfix "NotShowIn" (genericTray cfg))
    && pkgs.lib.hasInfix "/bin/peasy-tray --ui" (genericTray cfg);
  appRefresh = cfg: cfg.systemd.services.peasy-applications-refresh.serviceConfig;
in
assert pkgs.lib.all validTray [
  gnome
  plasma
  hyprland
  xfce
  generic
];
assert (profile plasma).configured_desktops == [ "kde_plasma" ];
assert (profile gnome).configured_desktops == [ "gnome" ];
assert (profile hyprland).configured_desktops == [ "hyprland" ];
assert (profile xfce).configured_desktops == [ "xfce" ];
assert !(xfce.services.desktopManager.gnome.enable);
assert !(xfce.services.desktopManager.plasma6.enable);
assert !(xfce.environment.etc ? "xdg/autostart/peasy-panel.desktop");
assert builtins.elem "/share/peasy" xfce.environment.pathsToLink;
assert !(plasma.environment.etc ? "xdg/autostart/peasy-panel.desktop");
assert !(hyprland.environment.etc ? "xdg/autostart/peasy-panel.desktop");
assert !(plasma.services.desktopManager.gnome.enable);
assert !(builtins.elem pkgs.gnomeExtensions.appindicator plasma.environment.systemPackages);
assert !(builtins.elem pkgs.gnomeExtensions.appindicator hyprland.environment.systemPackages);
assert builtins.elem pkgs.gnomeExtensions.appindicator gnome.environment.systemPackages;
assert builtins.elem pkgs.gjs gnome.environment.systemPackages;
assert !(headless.systemd.services ? peasy-applications-refresh);
assert !(headless.systemd.paths ? peasy-applications-refresh);
assert builtins.length xfce.nixpkgs.overlays == 1;
assert pkgs.lib.all (cfg: cfg.nixpkgs.overlays == [ ]) [
  gnome
  plasma
  generic
  headlessXfce
];
assert pkgs.lib.all
  (
    cfg:
    (appRefresh cfg).ExecStart
    == "${pkgs.coreutils}/bin/touch --no-create --no-dereference /nix/var/nix/profiles/system"
    && (appRefresh cfg).ProtectSystem == "strict"
    && (appRefresh cfg).ReadWritePaths == [ "/nix/var/nix/profiles" ]
    && cfg.systemd.services.peasy-applications-refresh.wantedBy == [ "multi-user.target" ]
    && cfg.systemd.paths.peasy-applications-refresh.pathConfig.PathChanged == "/run"
  )
  [
    gnome
    plasma
    xfce
    generic
  ];
pkgs.runCommand "peasy-desktop-configuration-check" { } "touch $out"
