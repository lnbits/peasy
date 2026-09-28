# A release module has a distinct key so the original host import can be disabled
# without editing the administrator's traditional configuration or flake input.
{
  config,
  lib,
  pkgs,
  ...
}@args:
(import ./module.nix args) // { key = "peasy-active-release"; }
