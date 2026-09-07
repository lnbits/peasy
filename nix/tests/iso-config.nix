{
  pkgs,
  configurations,
  releaseStatus,
}:
let
  gnome = configurations.peasy-iso-gnome.config;
  plasma = configurations.peasy-iso-plasma.config;
  isoPkgs = configurations.peasy-iso-plasma.pkgs;
  artwork = import ../iso-branding { pkgs = isoPkgs; };
  validOffline =
    desktop: system:
    let
      cached = import ../installer-offline.nix {
        inherit desktop;
        pkgs = system.pkgs;
        lib = pkgs.lib;
        package = system.config.services.peasy.package;
      };
    in
    builtins.elem cached.seed system.config.isoImage.storeContents
    && !(cached.configuration ? isoImage)
    && cached.configuration.boot.loader.grub.theme == null
    && cached.configuration.system.nixos.version == system.config.system.nixos.version
    && cached.configuration.services.speechd.package == pkgs.speechd
    && cached.configuration.documentation.nixos.enable
    && pkgs.lib.hasInfix ''--generator "nixos-render-docs ${system.config.system.nixos.version}"'' cached.configuration.system.build.manual.manualHTML.buildCommand;
  valid =
    cfg:
    cfg.services.peasy.enable
    && !(builtins.elem "nixpkgs=flake:nixpkgs" cfg.nix.nixPath)
    && builtins.elem isoPkgs.calamares-nixos cfg.environment.systemPackages
    && cfg.environment.etc."peasy/wallpaper.png".source == ../.. + "/assets/peasy_bg.png"
    && cfg.isoImage.grubTheme == "${artwork}/grub"
    && cfg.isoImage.appendToMenuLabel == " Installer - Includes Peasy"
    && (builtins.fromJSON cfg.environment.etc."peasy/iso-status.json".text) == releaseStatus
    && pkgs.lib.hasSuffix "x86_64" cfg.image.baseName
    && pkgs.lib.hasInfix "io.github.peasy.apply\") return polkit.Result.NO" cfg.security.polkit.extraConfig;
in
assert valid gnome && valid plasma;
assert validOffline "gnome" configurations.peasy-iso-gnome;
assert validOffline "plasma" configurations.peasy-iso-plasma;
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
