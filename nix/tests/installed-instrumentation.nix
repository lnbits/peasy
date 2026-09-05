{
  lib,
  pkgs,
  modulesPath,
  ...
}@args:
let
  data = args.peasyTestData or (builtins.fromJSON (builtins.readFile ./test-data.json));
in
{
  # Test fixtures only. This module is never imported by a shipped system.
  imports = [ (modulesPath + "/testing/test-instrumentation.nix") ];
  users.users.peasytest.initialPassword = "test";
  boot.loader.efi.canTouchEfiVariables = lib.mkForce false;
  documentation.enable = lib.mkForce false;
  system.stateVersion = lib.mkForce "26.05";
  environment.systemPackages = [
    pkgs.glib
    pkgs.python3
  ];
  environment.sessionVariables.GSK_RENDERER = "cairo";
  # QEMU provides eth0 with NAT/DHCP and eth1 on the test-only VLAN without a
  # DHCP server. The installed disk does not inherit qemu-vm.nix's static VLAN
  # configuration. Keep NetworkManager on eth0, but do not let its automatic
  # eth1 profile time out and fail an otherwise successful system activation.
  networking.networkmanager.unmanaged = [ "interface-name:eth1" ];
  nix.settings.substituters = lib.mkForce [ ];
  system.extraDependencies = map builtins.storePath data.buildTools;
  services.desktopManager.gnome.extraGSettingsOverrides = lib.mkIf data.gnome ''
    [org.gnome.shell]
    welcome-dialog-last-shown-version='9999999999'
  '';
}
