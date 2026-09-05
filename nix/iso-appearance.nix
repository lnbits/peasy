{
  config,
  lib,
  pkgs,
  ...
}:
let
  wallpaper = ../assets/peasy_bg.png;
in
{
  # Imported by ISO profiles and ISO-installed systems, not manual installs.
  # This is build-time trusted branding, not an AI-selectable path or script.
  environment.etc."peasy/wallpaper.png".source = wallpaper;

  services.desktopManager.gnome.extraGSettingsOverrides = lib.mkIf config.services.desktopManager.gnome.enable ''
    [org.gnome.desktop.background]
    picture-uri='file://${wallpaper}'
    picture-uri-dark='file://${wallpaper}'
    picture-options='zoom'
    [org.gnome.desktop.interface]
    accent-color='green'
  '';

  # Plasma creates desktop containments at first login. Its fixed upstream CLI
  # applies our bundled image after plasmashell starts, without replacing the
  # normal panel/layout or exposing evaluateScript to Peasy's action interface.
  systemd.user.services.peasy-iso-appearance =
    lib.mkIf config.services.desktopManager.plasma6.enable
      {
        description = "Initial Peasy wallpaper and green accent";
        wantedBy = [ "plasma-workspace.target" ];
        after = [
          "plasma-plasmashell.service"
          "plasma-ksplash.service"
        ];
        unitConfig = {
          ConditionPathExists = "!%h/.local/state/peasy/iso-appearance-v1";
          StartLimitIntervalSec = 120;
          StartLimitBurst = 20;
        };
        serviceConfig = {
          Type = "oneshot";
          TimeoutStartSec = 20;
          Restart = "on-failure";
          RestartSec = 2;
          ExecStart = pkgs.writeShellScript "peasy-iso-appearance" ''
            set -eu
            # plasmashell can own its bus name before creating a desktop. The
            # wallpaper CLI reports success even when that desktop list is empty.
            desktops=$(${pkgs.kdePackages.qttools}/bin/qdbus org.kde.plasmashell /PlasmaShell org.kde.PlasmaShell.evaluateScript 'print(desktops().length)')
            [[ "$desktops" =~ ^[1-9][0-9]*$ ]] || exit 1
            ${pkgs.kdePackages.plasma-workspace}/bin/plasma-apply-wallpaperimage --fill-mode preserveAspectCrop ${wallpaper}
            ${pkgs.kdePackages.kconfig}/bin/kwriteconfig6 --file kdeglobals --group General --key AccentColor '58,148,74'
            ${pkgs.kdePackages.plasma-workspace}/bin/plasma-apply-colorscheme --accent-color '#3a944a'
            ${pkgs.coreutils}/bin/mkdir -p "$HOME/.local/state/peasy"
            ${pkgs.coreutils}/bin/touch "$HOME/.local/state/peasy/iso-appearance-v1"
          '';
        };
      };
}
