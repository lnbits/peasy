{
  pkgs,
  configurations,
  releaseStatus,
}:
let
  gnome = configurations.peasy-iso-gnome.config;
  plasma = configurations.peasy-iso-plasma.config;
  isoPkgs = configurations.peasy-iso-plasma.pkgs;
  valid =
    cfg:
    cfg.services.peasy.enable
    && builtins.elem isoPkgs.calamares-nixos cfg.environment.systemPackages
    && cfg.environment.etc."peasy/wallpaper.png".source == ../.. + "/assets/peasy_bg.png"
    && (builtins.fromJSON cfg.environment.etc."peasy/iso-status.json".text) == releaseStatus
    && pkgs.lib.hasSuffix "x86_64" cfg.image.baseName
    && pkgs.lib.hasInfix "io.github.peasy.apply\") return polkit.Result.NO" cfg.security.polkit.extraConfig;
in
assert valid gnome && valid plasma;
assert gnome.isoImage.edition == "gnome";
assert plasma.isoImage.edition == "plasma6";
assert gnome.services.desktopManager.gnome.enable;
assert plasma.services.desktopManager.plasma6.enable;
assert !(plasma.services.desktopManager.gnome.enable);
assert !(builtins.elem isoPkgs.gnomeExtensions.appindicator plasma.environment.systemPackages);
assert pkgs.lib.hasInfix "accent-color='green'"
  gnome.services.desktopManager.gnome.extraGSettingsOverrides;
assert plasma.systemd.user.services ? peasy-iso-appearance;
assert
  releaseStatus.releaseReady
  && releaseStatus.installedTargetHasPeasy
  && releaseStatus.installedBootVerified;
pkgs.runCommand "peasy-iso-configuration-check" { } "touch $out"
