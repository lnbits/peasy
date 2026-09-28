{
  nixpkgs,
  managed,
  postgresql,
  capabilities,
  system ? builtins.currentSystem,
}:
let
  pkgs = import nixpkgs { inherit system; };
  base = {
    boot.loader.grub.devices = [ "nodev" ];
    fileSystems."/" = {
      device = "none";
      fsType = "tmpfs";
    };
    system.stateVersion = "26.05";
    users.users.peasytest = {
      isNormalUser = true;
      uid = 1000;
      extraGroups = [ "wheel" ];
    };
  };
  evaluate =
    modules:
    (import (nixpkgs + "/nixos/lib/eval-config.nix") {
      inherit system;
      modules = [ base ] ++ modules;
    }).config;
  capabilityCases = import capabilities;
  checkCapability =
    case:
    let
      configured = evaluate [ case.module ];
      option =
        if case.option == null then
          true
        else
          pkgs.lib.getAttrFromPath (pkgs.lib.splitString "." case.option) configured;
    in
    assert option;
    assert builtins.all (
      group:
      builtins.elem group configured.users.users.peasytest.extraGroups
      && builtins.hasAttr group configured.users.groups
    ) case.groups;
    assert builtins.all (item: item.assertion) configured.assertions;
    # Flatpak has a usable fallback and retains host desktop-specific choices.
    assert
      case.option != "services.flatpak.enable"
      || (
        configured.xdg.portal.enable
        && builtins.elem pkgs.xdg-desktop-portal-gtk configured.xdg.portal.extraPortals
      );
    true;
  installed = evaluate [ managed ];
  database = evaluate [ postgresql ];
  remoteDatabase = evaluate [
    postgresql
    { services.postgresql.settings.listen_addresses = pkgs.lib.mkForce "*"; }
  ];
  versionConflict =
    builtins.tryEval
      (evaluate [
        postgresql
        { services.postgresql.package = pkgs.postgresql_16; }
      ]).services.postgresql.package;
  changedAccount = evaluate [
    managed
    { users.users.peasytest.uid = pkgs.lib.mkForce 1001; }
  ];
  removed = evaluate [ ];
  external = evaluate [
    {
      virtualisation.libvirtd.enable = true;
      users.users.peasytest.extraGroups = [ "libvirtd" ];
    }
  ];
  conflict =
    builtins.tryEval
      (evaluate [
        managed
        { virtualisation.libvirtd.enable = false; }
      ]).virtualisation.libvirtd.enable;
in
assert builtins.all checkCapability capabilityCases;
assert installed.virtualisation.libvirtd.enable;
assert installed.programs.virt-manager.enable;
assert builtins.elem "libvirtd" installed.users.users.peasytest.extraGroups;
assert builtins.elem "wheel" installed.users.users.peasytest.extraGroups;
assert builtins.elem "virt-manager" (map pkgs.lib.getName installed.environment.systemPackages);
assert builtins.all (item: item.assertion) installed.assertions;
assert !(builtins.all (item: item.assertion) changedAccount.assertions);
assert !removed.virtualisation.libvirtd.enable;
assert !removed.programs.virt-manager.enable;
assert removed.users.users.peasytest.extraGroups == [ "wheel" ];
assert external.virtualisation.libvirtd.enable;
assert builtins.elem "libvirtd" external.users.users.peasytest.extraGroups;
assert !conflict.success;
assert database.services.postgresql.enable;
assert database.services.postgresql.package.version == pkgs.postgresql_17.version;
assert database.services.postgresql.settings.listen_addresses == "localhost";
assert database.services.postgresql.ensureDatabases == [ "peasytest" ];
assert (builtins.head database.services.postgresql.ensureUsers).ensureDBOwnership;
assert !(builtins.elem 5432 database.networking.firewall.allowedTCPPorts);
assert builtins.all (item: item.assertion) database.assertions;
assert !(builtins.all (item: item.assertion) remoteDatabase.assertions);
assert !versionConflict.success;
assert !removed.services.postgresql.enable;
true
