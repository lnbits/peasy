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
    networking.firewall.allowedTCPPorts = [ 9090 ];
  };
  evaluate =
    modules:
    (import (nixpkgs + "/nixos/lib/eval-config.nix") {
      inherit system;
      modules = [ base ] ++ modules;
    }).config;
  configured = evaluate [ managed ];
  removed = evaluate [ ];
  changed = evaluate [
    managed
    { users.users.peasytest.uid = pkgs.lib.mkForce 1099; }
  ];
in
assert configured.services.printing.enable;
assert configured.virtualisation.libvirtd.enable;
assert builtins.all (group: builtins.elem group configured.users.users.peasytest.extraGroups) [
  "wheel"
  "libvirtd"
  "video"
  "audio"
];
assert configured.fileSystems."/mnt/peasy-usb".device == "/dev/disk/by-uuid/abcd-1234";
assert builtins.elem "nofail" configured.fileSystems."/mnt/peasy-usb".options;
assert builtins.elem 8080 configured.networking.firewall.allowedTCPPorts;
assert builtins.elem 9090 configured.networking.firewall.allowedTCPPorts;
assert configured.users.users.guestuser.uid == 1001;
assert configured.users.users.guestuser.hashedPassword == "!";
assert builtins.all (name: builtins.elem name configured.services.openssh.settings.DenyUsers) [
  "guestuser"
  "secondguest"
];
assert configured.services.logind.settings.Login.HandleLidSwitch == "suspend";
assert builtins.all (item: item.assertion) configured.assertions;
assert !(builtins.all (item: item.assertion) changed.assertions);
assert removed.networking.firewall.allowedTCPPorts == [ 9090 ];
assert !(builtins.hasAttr "/mnt/peasy-usb" removed.fileSystems);
assert !(builtins.hasAttr "guestuser" removed.users.users);
true
