{
  nixpkgs,
  managed,
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
  installed = evaluate [ managed ];
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
true
