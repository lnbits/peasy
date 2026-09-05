{
  pkgs,
  package,
  desktop,
}:
let
  lib = pkgs.lib;
  gnome = desktop == "gnome";
  integration = import ../installer.nix {
    inherit pkgs;
    source = package.src;
  };
  # Offline NixOS glue-build inputs, following nixpkgs' installer test.
  buildTools =
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
      perlPackages.FileCopyRecursive
      perlPackages.XMLSAX
      perlPackages.XMLSAXBase
      zstd.bin
      mypy
    ]
    ++ lib.concatMap (package: map (output: package.${output}) package.outputs) [
      # Desktop glue builds need outputs not retained by the runtime closure
      # (for example validators and development data used to assemble wrappers).
      pkgs.gtk3
      pkgs.ghostscript
      pkgs.ibus
      pkgs.libxkbcommon
    ];
  testData = {
    inherit gnome;
    buildTools = map toString buildTools;
  };
  instrumentation = ./installed-instrumentation.nix;
  testDataFile = pkgs.writeText "peasy-installed-test-data.json" (builtins.toJSON testData);
  seed =
    (import (pkgs.path + "/nixos/lib/eval-config.nix") {
      system = pkgs.stdenv.hostPlatform.system;
      specialArgs.peasyTestData = testData // {
        buildTools = [ ];
      };
      modules = [
        ../module.nix
        ../iso-appearance.nix
        instrumentation
        {
          networking.hostName = "peasy-installed";
          networking.networkmanager.enable = true;
          services.peasy.enable = true;
          services.peasy.package = lib.mkForce package;
          system.extraDependencies = buildTools;
          services.displayManager.gdm.enable = gnome;
          services.displayManager.sddm.enable = !gnome;
          services.desktopManager.gnome.enable = gnome;
          services.desktopManager.plasma6.enable = !gnome;
          services.xserver.enable = !gnome;
          services.displayManager.autoLogin = {
            enable = true;
            user = "peasytest";
          };
          users.users.peasytest = {
            isNormalUser = true;
            description = "Peasy Test";
            extraGroups = [
              "networkmanager"
              "wheel"
            ];
            packages = lib.optional (!gnome) pkgs.kdePackages.kate;
          };
          programs.firefox.enable = true;
          services.printing.enable = true;
          security.rtkit.enable = true;
          services.pipewire = {
            enable = true;
            alsa.enable = true;
            alsa.support32Bit = true;
            pulse.enable = true;
          };
          boot.loader.grub = lib.mkIf gnome {
            enable = true;
            device = "/dev/vda";
            useOSProber = true;
          };
          boot.loader.systemd-boot.enable = !gnome;
          fileSystems."/" = {
            device = "/dev/disk/by-label/nixos";
            fsType = "ext4";
          };
          fileSystems."/boot" = lib.mkIf (!gnome) {
            device = "/dev/disk/by-label/ESP";
            fsType = "vfat";
          };
        }
      ];
    }).config.system.build.toplevel;
in
pkgs.testers.runNixOSTest {
  name = "peasy-installed-${desktop}";
  node.pkgsReadOnly = false;
  globalTimeout = 2400;
  nodes = {
    installer = { lib, modulesPath, ... }: {
      imports = [
        (modulesPath + "/profiles/installation-device.nix")
        (pkgs.path + "/nixos/tests/common/auto-format-root-device.nix")
      ];
      virtualisation = {
        memorySize = 6144;
        diskSize = 24576;
        diskImage = "./target.qcow2";
        rootDevice = "/dev/vdb";
        emptyDiskImages = [ 1024 ];
        fileSystems."/".autoFormat = true;
        useEFIBoot = !gnome;
      };
      hardware.enableAllFirmware = lib.mkForce false;
      security.polkit.enablePkexecWrapper = true;
      nix.settings = {
        substituters = lib.mkForce [ ];
        connect-timeout = 1;
      };
      environment.systemPackages = [
        pkgs.python3
        pkgs.parted
        pkgs.dosfstools
      ];
      system.extraDependencies = [
        seed
        integration.extensions
        integration.source
      ]
      ++ buildTools;
    };
    target = {
      virtualisation = {
        memorySize = 6144;
        diskSize = 24576;
        diskImage = "./target.qcow2";
        useBootLoader = true;
        useEFIBoot = !gnome;
        useDefaultFilesystems = false;
        efi.keepVariables = false;
        fileSystems."/" = {
          device = "/dev/disk/by-label/not-used";
          fsType = "ext4";
        };
      };
    };
  };
  testScript = ''
    import datetime as dt

    installer.start()
    installer.wait_for_unit("multi-user.target")
    installer.succeed("udevadm settle")
    # /dev/vda is the disposable test disk; the installer itself uses /dev/vdb.
    installer.succeed("findmnt -n -o SOURCE / | grep -q '^/dev/vdb'")
    installer.succeed("parted --script /dev/vda mklabel gpt")
    ${
      if gnome then
        ''
          installer.succeed("parted --script /dev/vda mkpart bios 1MiB 3MiB set 1 bios_grub on mkpart root 3MiB 100%")
        ''
      else
        ''
          installer.succeed("parted --script /dev/vda mkpart ESP fat32 1MiB 257MiB set 1 esp on mkpart root 257MiB 100%")
          installer.succeed("udevadm settle; mkfs.vfat -n ESP /dev/vda1")
        ''
    }
    installer.succeed("udevadm settle; mkfs.ext4 -L nixos /dev/vda2")
    installer.succeed("mount -t ext4 /dev/vda2 /mnt; mkdir -p /mnt/etc/nixos /mnt/boot")
    ${lib.optionalString (!gnome) ''installer.succeed("mount -t vfat /dev/vda1 /mnt/boot")''}
    installer.copy_from_host("${instrumentation}", "/mnt/etc/nixos/test-instrumentation.nix")
    installer.copy_from_host("${testDataFile}", "/mnt/etc/nixos/test-data.json")
    installer.copy_from_host("${./installer-package.py}", "/mnt/etc/nixos/test-package.py")
    installer.succeed("python ${./installer-run.py} ${
      if gnome then "gnome bios" else "plasma6 efi"
    } ${integration.extensions}/lib/calamares/modules/nixos/main.py", timeout=dt.timedelta(minutes=25))
    installer.succeed("grep -q './peasy.nix' /mnt/etc/nixos/configuration.nix")
    installer.succeed("test -f /mnt/etc/nixos/peasy/nix/module.nix; test -f /mnt/etc/nixos/peasy/assets/peasy_bg.png")
    installer.succeed("umount -R /mnt; sync")
    installer.shutdown()
    target.state_dir = installer.state_dir
    target.start()
    target.wait_for_unit("graphical.target")
    target.wait_for_unit("peasy-system.service")
    target.wait_until_succeeds("pgrep -u peasytest -x peasy-tray", timeout=dt.timedelta(minutes=3))
    target.succeed("test -x /run/current-system/sw/bin/peasy-ui; test -f /etc/nixos/peasy.nix")
    target.succeed("pkaction --action-id io.github.peasy.apply --verbose | grep auth_admin")
    # Without an authentication agent or prior approval, a normal administrator
    # must not inherit the installer's passwordless wheel authorization.
    target.fail("su - peasytest -c 'pkcheck --action-id io.github.peasy.apply --process $$'")
    target.fail("id nixos")
    target.fail("test -e /etc/peasy/ISO-README.txt")
    target.fail("test -e /home/peasytest/.config/peasy/openai-api-key")
    target.succeed("python /etc/nixos/test-package.py install", timeout=dt.timedelta(minutes=10))
    target.succeed("/run/current-system/sw/bin/hello")
    target.succeed("test -f /etc/nixos/.peasy/peasy-managed.nix")
    target.succeed("nixos-rebuild build --no-flake", timeout=dt.timedelta(minutes=10))
    target.succeed("test -x result/sw/bin/hello")
    target.succeed("python /etc/nixos/test-package.py remove", timeout=dt.timedelta(minutes=10))
    target.fail("test -e /run/current-system/sw/bin/hello")
    target.screenshot("peasy-installed-${desktop}")
  '';
}
