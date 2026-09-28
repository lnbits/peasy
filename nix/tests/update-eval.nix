{
  nixpkgs,
  source,
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
    networking.hostName = "destination";
    services.peasy = {
      enable = true;
      desktop.enable = false;
    };
  };
  evaluate =
    modules:
    (import (nixpkgs + "/nixos/lib/eval-config.nix") {
      inherit system;
      modules = [ base ] ++ modules;
    }).config;
  check =
    original: extra:
    let
      configured = evaluate (
        [
          original
          managed
        ]
        ++ extra
      );
    in
    assert configured.networking.hostName == "destination";
    assert configured.fileSystems."/".device == "none";
    assert configured.virtualisation.libvirtd.enable;
    assert builtins.elem "wheel" configured.users.users.peasytest.extraGroups;
    assert builtins.elem "libvirtd" configured.users.users.peasytest.extraGroups;
    assert builtins.all (item: item.assertion) configured.assertions;
    assert configured.services.peasy.package.peasySource == toString source;
    assert configured.systemd.services.peasy-update-check.serviceConfig.DynamicUser;
    assert builtins.all (
      family:
      builtins.elem family [
        "AF_UNIX"
        "AF_NETLINK"
      ]
    ) configured.systemd.services.peasy-system.serviceConfig.RestrictAddressFamilies;
    assert
      (builtins.fromJSON configured.environment.etc."peasy/update-policy.json".text).version
      == configured.services.peasy.package.version;
    true;
  # Path imports, imported functions (flake output), repeated release module
  # imports and rollback all need to work with the stable module keys.
  traditional = check (source + "/nix/module.nix") [ ];
  flake = check (import (source + "/nix/module.nix")) [
    { services.peasy.hostFlake = "/etc/nixos#host"; }
  ];
  subsequent = check (source + "/nix/update-module.nix") [ ];
  rollback = evaluate [ (source + "/nix/module.nix") ];
  custom = evaluate [
    (source + "/nix/module.nix")
    managed
    { services.peasy.package = pkgs.hello; }
  ];
  disabled = evaluate [
    (source + "/nix/module.nix")
    { services.peasy.updates.enable = false; }
  ];
in
assert traditional && flake && subsequent;
assert !rollback.virtualisation.libvirtd.enable;
assert !(builtins.all (item: item.assertion) custom.assertions);
assert !(builtins.fromJSON disabled.environment.etc."peasy/update-policy.json".text).enabled;
true
