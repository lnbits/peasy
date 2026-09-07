{
  config,
  lib,
  pkgs,
  ...
}:
{
  imports = [
    ./module.nix
    ./iso-appearance.nix
    ./iso-boot-branding.nix
  ];
  services.peasy.enable = true;
  # nixosSystem is evaluated from a flake, but Calamares invokes traditional
  # nixos-install. Resolve <nixpkgs> through the bundled channel, not a flake
  # lookup that requires experimental features in the live environment.
  nixpkgs.flake.setNixPath = false;

  # The actual release medium must provide the installed desktop and its
  # configuration builders, not merely a runnable live desktop.
  isoImage.storeContents =
    (import ./installer-offline.nix {
      inherit pkgs lib;
      package = config.services.peasy.package;
      desktop = if config.services.desktopManager.gnome.enable then "gnome" else "plasma";
    }).storeContents;

  # A pinned, fail-closed addition to upstream's configuration generator.
  # The installer UI, storage, accounts and single nixos-install flow stay intact.
  nixpkgs.overlays = [
    (_final: previous: {
      calamares-nixos-extensions = (import ./installer.nix { pkgs = previous; }).extensions;
    })
  ];

  image.baseName = lib.mkForce "peasy-nixos-${config.system.nixos.release}-${config.isoImage.edition}-x86_64";
  environment.etc."peasy/iso-status.json".text = builtins.toJSON {
    releaseReady = true;
    installedTargetHasPeasy = true;
    installedBootVerified = true;
    reason = "Installed-disk boot verified. Tag releases publish after CI checks and whole-image R2 verification.";
  };
  environment.etc."peasy/ISO-README.txt".text = ''
    PEASY NIXOS INSTALLER
    The upstream NixOS graphical installer has a narrow Peasy integration:
    it adds /etc/nixos/peasy.nix and bundled Peasy source to the target system.
    Partitioning, accounts and the normal nixos-install flow stay upstream's.
    GNOME is selected by default. Optional XFCE installation needs Internet
    access; this image caches its live desktop's installed-system packages.
    See https://github.com/lnbits/peasy/blob/main/docs/iso.md for validation status.
    Peasy system Apply is disabled in this live session; search and supported
    local desktop actions remain available. Do not put production API keys on
    a shared live session. Live-session settings and keys are not copied into
    the installed system. Configure your own AI provider after installation.
  '';

  # The upstream live image grants wheel passwordless Polkit access for its
  # installer. Do not inherit that bypass for Peasy's privileged Apply action.
  # This earlier, narrowly scoped rule does not change Calamares permissions.
  security.polkit.extraConfig = lib.mkBefore ''
    polkit.addRule(function(action, subject) {
      if (action.id == "io.github.peasy.apply") return polkit.Result.NO;
    });
  '';
}
