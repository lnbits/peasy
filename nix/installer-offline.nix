{
  pkgs,
  package,
  desktop,
  lib ? pkgs.lib,
}:
let
  gnome = desktop == "gnome";
  # Cache a representative installed desktop, not the live system's special
  # accounts/permissions. This generation is never activated or copied as the
  # user's configuration: Calamares still generates that from their choices.
  configuration =
    (import (pkgs.path + "/nixos/lib/eval-config.nix") {
      inherit lib;
      # The installed channel embeds the ISO's release metadata. NixOS's HTML
      # manual includes pkgs.lib.version in its derivation, so using pre-git
      # here would miss the installed manual and trigger its compiler closure.
      system = pkgs.stdenv.hostPlatform.system;
      modules = [
        ./module.nix
        ./iso-appearance.nix
        {
          # Start with the installed system's normal package set. Reusing the
          # live ISO's pkgs would inherit installation-device's reduced speech
          # voices and miss the full installed accessibility packages.
          nixpkgs.overlays = [ (_final: _previous: { inherit lib; }) ];
          services.peasy.enable = true;
          services.peasy.package = lib.mkForce package;
          networking.hostName = "peasy-offline-seed";
          networking.networkmanager.enable = true;
          services.displayManager.gdm.enable = gnome;
          services.displayManager.sddm.enable = !gnome;
          services.desktopManager.gnome.enable = gnome;
          services.desktopManager.plasma6.enable = !gnome;
          services.xserver.enable = !gnome;
          services.printing.enable = true;
          services.pulseaudio.enable = false;
          security.rtkit.enable = true;
          services.pipewire = {
            enable = true;
            alsa.enable = true;
            alsa.support32Bit = true;
            pulse.enable = true;
          };
          programs.firefox.enable = true;
          i18n.supportedLocales = [ "all" ];
          users.users.peasyseed = {
            isNormalUser = true;
            hashedPassword = "!";
            extraGroups = [
              "networkmanager"
              "wheel"
            ];
            packages = lib.optional (!gnome) pkgs.kdePackages.kate;
          };
          boot.loader.systemd-boot.enable = true;
          boot.loader.efi.canTouchEfiVariables = false;
          fileSystems."/" = {
            device = "/dev/disk/by-label/nixos";
            fsType = "ext4";
          };
          fileSystems."/boot" = {
            device = "/dev/disk/by-label/ESP";
            fsType = "vfat";
          };
          system.stateVersion = lib.versions.majorMinor lib.version;
        }
      ];
    }).config;
  seed = configuration.system.build.toplevel;
  # NixOS still assembles account, locale, boot and service configuration for
  # each machine. Include those builders, but not the recursive source/build
  # closure of every native desktop package (which would greatly inflate ISOs).
  builders =
    with pkgs;
    [
      stdenv
      bintools
      brotli
      brotli.dev
      brotli.lib
      desktop-file-utils
      docbook5
      docbook_xsl_ns
      hello
      kbd.dev
      kmod.dev
      libarchive.dev
      libcap-text-verifier
      libxml2.bin
      libxslt.bin
      nixos-rebuild-ng
      perlPackages.ConfigIniFiles
      perlPackages.FileSlurp
      perlPackages.JSON
      perlPackages.ListCompare
      perlPackages.XMLLibXML
      perlPackages.FileCopyRecursive
      perlPackages.XMLSAX
      perlPackages.XMLSAXBase
      shared-mime-info
      sudo
      switch-to-configuration-ng
      texinfo
      unionfs-fuse
      lndir
      shellcheck-minimal
      systemdMinimal.out
      grub2
      grub2_efi
      nixos-artwork.wallpapers.simple-dark-gray-bootloader
      zstd.bin
      mypy
    ]
    ++ lib.concatMap (p: map (output: p.${output}) p.outputs) [
      pkgs.gtk3
      pkgs.ghostscript
      pkgs.ibus
      pkgs.libxkbcommon
    ];
in
{
  inherit seed builders configuration;
  storeContents = [ seed ] ++ builders;
}
