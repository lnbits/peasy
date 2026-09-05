{ lib, ... }:
{
  # Installed by the Peasy ISO, not by the manual Peasy module.
  imports = [
    ./peasy/nix/module.nix
    ./peasy/nix/iso-appearance.nix
  ]
  ++ lib.optional (builtins.pathExists ./.peasy/peasy-managed.nix) ./.peasy/peasy-managed.nix;
  services.peasy.enable = true;
}
