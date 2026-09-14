{
  nixpkgs,
  managed,
  system ? builtins.currentSystem,
}:
let
  pkgs = import nixpkgs { inherit system; };
  evaluate =
    modules:
    (import (nixpkgs + "/nixos/lib/eval-config.nix") {
      inherit system;
      modules = [
        {
          boot.loader.grub.devices = [ "nodev" ];
          fileSystems."/" = {
            device = "none";
            fsType = "tmpfs";
          };
          system.stateVersion = "26.05";
        }
      ]
      ++ modules;
    }).config;
  installed = evaluate [ managed ];
  removed = evaluate [ ];
  connection =
    installed.environment.etc."NetworkManager/system-connections/peasy-wireless.nmconnection";
  conflict =
    builtins.tryEval
      (evaluate [
        managed
        { networking.networkmanager.enable = false; }
      ]).networking.networkmanager.enable;
in
assert builtins.all (item: item.assertion) installed.assertions;
assert installed.networking.networkmanager.enable;
assert connection.mode == "0600";
assert pkgs.lib.hasInfix "ssid=Local \${notNix} Network" connection.text;
assert pkgs.lib.hasInfix "psk-flags=2" connection.text;
assert !(pkgs.lib.hasInfix "psk=" connection.text);
assert
  installed.networking.firewall.interfaces.wlan0.allowedUDPPorts == [
    53
    67
  ];
assert !(removed.environment.etc ? "NetworkManager/system-connections/peasy-wireless.nmconnection");
assert !(removed.networking.firewall.interfaces ? wlan0);
assert !conflict.success;
true
