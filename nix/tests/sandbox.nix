{
  pkgs,
  module,
  package,
}:

let
  fakeSwitch = pkgs.writeShellScript "peasy-test-switch-to-configuration" ''
    exit 0
  '';
  fakeSystem = pkgs.runCommand "peasy-test-nixos-system" { } ''
    mkdir -p $out/bin
    ln -s ${fakeSwitch} $out/bin/switch-to-configuration
  '';
  legacyHostConfiguration = pkgs.writeText "peasy-test-host-configuration.nix" ''
    { ... }:
    {
      boot.loader.grub.devices = [ "nodev" ];
      fileSystems."/" = {
        device = "none";
        fsType = "tmpfs";
      };
      system.stateVersion = "26.05";
    }
  '';
  legacyPackagesModule = pkgs.writeText "peasy-test-packages.nix" ''
    { lib, pkgs, ... }:
    {
      environment.systemPackages = map
        (attribute: lib.getAttrFromPath (lib.splitString "." attribute) pkgs)
        [ "hello" ];
      programs.dconf.enable = true;
      programs.dconf.profiles.user.databases = [
        {
          settings."org/gnome/desktop/interface" = {
            accent-color = "blue";
            color-scheme = "prefer-dark";
          };
          locks = [
            "/org/gnome/desktop/interface/accent-color"
            "/org/gnome/desktop/interface/color-scheme"
          ];
        }
      ];
    }
  '';
  legacySystemExpression = pkgs.writeText "peasy-test-system.nix" ''
    let
      nixpkgs = builtins.toPath "${pkgs.path}";
      evaluated = import (nixpkgs + "/nixos/lib/eval-config.nix") {
        system = "${pkgs.stdenv.hostPlatform.system}";
        modules = [
          (builtins.toPath "${legacyHostConfiguration}")
          (builtins.toPath "${legacyPackagesModule}")
        ];
      };
    in
    builtins.seq evaluated.config.system.build.toplevel.drvPath ${fakeSystem}
  '';
  generatedThemeSystemExpression = pkgs.writeText "peasy-generated-theme-system.nix" ''
    let
      nixpkgs = builtins.toPath "${pkgs.path}";
      evaluated = import (nixpkgs + "/nixos/lib/eval-config.nix") {
        system = "${pkgs.stdenv.hostPlatform.system}";
        modules = [
          (builtins.toPath "${legacyHostConfiguration}")
          (builtins.toPath "/tmp/peasy-generated-theme.nix")
        ];
      };
    in
    builtins.seq evaluated.config.system.build.toplevel.drvPath ${fakeSystem}
  '';
  generatedAppImageSystemExpression = pkgs.writeText "peasy-generated-appimage-system.nix" ''
    let
      nixpkgs = builtins.toPath "${pkgs.path}";
      evaluated = import (nixpkgs + "/nixos/lib/eval-config.nix") {
        system = "${pkgs.stdenv.hostPlatform.system}";
        modules = [
          (builtins.toPath "${legacyHostConfiguration}")
          (builtins.toPath "/tmp/peasy-generated-appimage.nix")
        ];
      };
    in
    builtins.seq evaluated.config.system.build.toplevel.drvPath ${fakeSystem}
  '';
in
pkgs.testers.runNixOSTest {
  name = "peasy-sandbox";
  nodes.machine =
    { lib, ... }:
    {
      imports = [ module ];
      services.peasy = {
        enable = true;
        desktop.enable = false;
        inherit package;
        configurationReadPaths = [ "/home/testuser/peasy-source" ];
      };
      users.users.testuser = {
        isNormalUser = true;
        password = "test";
      };
      systemd.tmpfiles.rules = [
        "d /home/testuser 0700 testuser users -"
        "f /home/testuser/private.txt 0600 testuser users - test-secret"
        "d /home/testuser/peasy-source 0755 testuser users -"
        "f /home/testuser/peasy-source/module.nix 0644 testuser users - allowed-module"
      ];
      systemd.services.peasy-sandbox-probe = {
        description = "Exercise Peasy's production filesystem sandbox";
        serviceConfig = {
          Type = "oneshot";
          ExecStart = lib.concatStringsSep " " [
            "${package}/libexec/peasy-system"
            "--nix ${pkgs.nix}/bin/nix"
            "--systemctl ${pkgs.systemd}/bin/systemctl"
            "--nixpkgs ${pkgs.path}"
            "--system ${pkgs.stdenv.hostPlatform.system}"
            "--self-test-sandbox"
          ];
          ProtectHome = "tmpfs";
          ProtectSystem = "strict";
          ReadWritePaths = [
            "/run/peasy"
            "/etc/nixos/.peasy"
          ];
          PrivateTmp = true;
          PrivateDevices = true;
          NoNewPrivileges = true;
          RestrictSUIDSGID = true;
          LockPersonality = true;
          RestrictAddressFamilies = [
            "AF_UNIX"
            "AF_NETLINK"
          ];
          IPAddressDeny = "any";
        };
      };
      environment.systemPackages = [ pkgs.socat ];
      nix.settings.experimental-features = [
        "nix-command"
        "flakes"
      ];
      environment.etc."nixos/configuration.nix".text = ''
        { ... }: { system.stateVersion = "26.05"; }
      '';
      # A cold `nix search --json nixpkgs` evaluates the full package set and
      # currently peaks above 4 GiB.  The Peasy daemon itself remains tiny.
      virtualisation.memorySize = 8192;
    };
  testScript = ''
    start_all()
    machine.wait_for_unit("peasy-system.service")
    machine.wait_for_file("/run/peasy/peasy.sock")
    machine.succeed("test -x ${package}/bin/peasy")
    machine.succeed("test -x ${package}/libexec/peasy-system")
    machine.fail("test -e ${package}/bin/peasy-ui")
    machine.fail("test -e ${package}/bin/peasy-tray")
    machine.fail("test -e /etc/xdg/autostart/peasy-panel.desktop")
    machine.succeed("test -f /etc/peasy/system-profile.json")
    machine.succeed("test -f /etc/peasy/host-configuration-path")
    machine.succeed("grep -qx '/etc/nixos/configuration.nix' /etc/peasy/host-configuration-path")
    machine.succeed("grep -q '\"peasy_variant\":\"headless\"' /etc/peasy/system-profile.json")
    machine.succeed("grep -q '\"installed_system_packages\"' /etc/peasy/system-profile.json")
    machine.succeed("grep -q 'socat' /etc/peasy/system-profile.json")
    response = machine.succeed(
      "printf '%s\\n' '{\"request\":\"status\"}' | socat -t 120 STDIO,ignoreeof UNIX-CONNECT:/run/peasy/peasy.sock"
    )
    assert '"ready":true' in response
    machine.succeed(
      "nix build --file ${legacySystemExpression} --out-link /tmp/peasy-legacy-result"
    )
    machine.succeed("test -x /tmp/peasy-legacy-result/bin/switch-to-configuration")
    # Generate this module with the production Rust renderer, then require a
    # real NixOS module evaluation to accept its GNOME theme settings.
    machine.succeed(
      "${package}/libexec/peasy-system --render-test-theme > /tmp/peasy-generated-theme.nix"
    )
    machine.succeed("grep 'accent-color = \"blue\"' /tmp/peasy-generated-theme.nix")
    machine.succeed("grep 'color-scheme = \"prefer-dark\"' /tmp/peasy-generated-theme.nix")
    machine.succeed("grep 'environment.etc.\"peasy/theme.json\"' /tmp/peasy-generated-theme.nix")
    machine.fail("grep 'locks =' /tmp/peasy-generated-theme.nix")
    machine.succeed(
      "nix build --file ${generatedThemeSystemExpression} --out-link /tmp/peasy-theme-result"
    )
    machine.succeed("test -x /tmp/peasy-theme-result/bin/switch-to-configuration")
    # External releases must remain inert pinned Nix data during evaluation;
    # this deliberately fake URL is never fetched by the test.
    machine.succeed(
      "${package}/libexec/peasy-system --render-test-appimage > /tmp/peasy-generated-appimage.nix"
    )
    machine.succeed("grep 'pkgs.appimageTools.wrapType2' /tmp/peasy-generated-appimage.nix")
    machine.succeed("grep 'sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=' /tmp/peasy-generated-appimage.nix")
    machine.succeed(
      "nix build --file ${generatedAppImageSystemExpression} --out-link /tmp/peasy-appimage-result"
    )
    machine.succeed("test -x /tmp/peasy-appimage-result/bin/switch-to-configuration")
    unit = machine.succeed("systemctl cat peasy-system.service")
    assert "--host-configuration /etc/nixos/configuration.nix" in unit
    assert "--host-flake" not in unit
    assert "X-RestartIfChanged=false" in unit
    assert "X-StopIfChanged=false" in unit
    activate_unit = machine.succeed("systemctl cat peasy-activate.service")
    assert "ProtectHome=" not in activate_unit
    assert "PrivateDevices=" not in activate_unit
    assert "ProtectKernelTunables=" not in activate_unit
    machine.fail("test -e /etc/systemd/user/peasy-theme-sync.service")
    machine.fail("test -e /etc/systemd/user/peasy-theme-sync.path")
    machine.succeed(
      "pid=$(systemctl show -p MainPID --value peasy-system.service); nsenter -t $pid -m -- cat /home/testuser/peasy-source/module.nix | grep allowed-module"
    )
    machine.fail(
      "pid=$(systemctl show -p MainPID --value peasy-system.service); nsenter -t $pid -m -- cat /home/testuser/private.txt"
    )
    search = machine.succeed(
      "printf '%s\\n' '{\"request\":\"search_packages\",\"query\":\"hello\"}' | socat -t 120 STDIO,ignoreeof UNIX-CONNECT:/run/peasy/peasy.sock"
    )
    print(search)
    assert '"search_results"' in search
    assert 'hello' in search
    machine.succeed("systemctl start peasy-sandbox-probe.service")
    machine.succeed("journalctl -u peasy-sandbox-probe.service | grep 'home-read=denied etc-write=denied'")
    machine.fail("test -e /etc/peasy-security-test")
  '';
}
