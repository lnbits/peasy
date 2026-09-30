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
  validOllama =
    cfg:
    cfg.services.ollama.enable
    && cfg.services.ollama.package == isoPkgs.ollama-cpu
    && cfg.services.ollama.host == "127.0.0.1"
    && cfg.services.ollama.port == 11434
    && !cfg.services.ollama.openFirewall
    && cfg.services.ollama.loadModels == [ ]
    && cfg.environment.sessionVariables.PEASY_DEFAULT_OLLAMA_MODEL == "qwen3:0.6b"
    && cfg.services.ollama.environmentVariables.OLLAMA_NO_CLOUD == "1"
    && pkgs.lib.hasInfix "/manifests/registry.ollama.ai/library/qwen3/0.6b" cfg.systemd.services.ollama.preStart;
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
    && cached.configuration.networking.networkmanager.enable
    && cached.configuration.hardware.bluetooth.enable
    && validOllama cached.configuration
    && cached.configuration.boot.loader.grub.theme == null
    && cached.configuration.system.nixos.version == system.config.system.nixos.version
    && cached.configuration.services.speechd.package == pkgs.speechd
    && cached.configuration.documentation.nixos.enable
    && pkgs.lib.hasInfix ''--generator "nixos-render-docs ${system.config.system.nixos.version}"'' cached.configuration.system.build.manual.manualHTML.buildCommand;
  valid =
    cfg:
    cfg.services.peasy.enable
    && validOllama cfg
    && cfg.networking.networkmanager.enable
    && cfg.hardware.bluetooth.enable
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
