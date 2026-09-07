{ lib, ... }:
{
  # Installed by the Peasy ISO, not by the manual Peasy module.
  imports = [
    ./peasy/nix/module.nix
    ./peasy/nix/iso-appearance.nix
  ]
  ++ lib.optional (builtins.pathExists ./.peasy/peasy-managed.nix) ./.peasy/peasy-managed.nix;
  services.peasy.enable = true;
  # Reuse the locale archive bundled with the ISO for every locale the wizard
  # offers, instead of building a different archive for each user's selection.
  i18n.supportedLocales = lib.mkDefault [ "all" ];
}
