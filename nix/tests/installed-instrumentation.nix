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
  nix.settings.substituters = lib.mkForce [ ];
  system.extraDependencies = map builtins.storePath data.buildTools;
  services.desktopManager.gnome.extraGSettingsOverrides = lib.mkIf data.gnome ''
    [org.gnome.shell]
    welcome-dialog-last-shown-version='9999999999'
  '';
}
