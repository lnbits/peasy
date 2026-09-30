{
  pkgs,
  module,
  package,
}:
let
  # Reconstruct pkgs.path as the daemon does when importing its saved source.
  # A flake's original path can hide the extra copy caused by interpolation.
  evaluate =
    source:
    (import (pkgs.path + "/nixos/lib/eval-config.nix") {
      system = pkgs.stdenv.hostPlatform.system;
      modules = [
        module
        {
          nixpkgs.pkgs = pkgs // {
            path = builtins.toPath source;
          };
          services.peasy = {
            enable = true;
            desktop.enable = false;
            inherit package;
          };
          system.stateVersion = "26.05";
        }
      ];
    }).config;
  original = toString pkgs.path;
  generation =
    source:
    let
      cfg = evaluate source;
      command = cfg.systemd.services.peasy-system.serviceConfig.ExecStart;
      identity = cfg.environment.etc."peasy/daemon-identity.json".source.text;
      # Reading the written JSON on the next boot produces context-free text.
      next = (builtins.fromJSON (builtins.unsafeDiscardStringContext identity)).nixpkgs;
    in
    assert pkgs.lib.hasInfix "--nixpkgs ${original} " command;
    assert (builtins.getContext command).${original}.path;
    assert (builtins.getContext identity).${original}.path;
    assert next == original;
    next;
  final = builtins.foldl' (source: _: generation source) original (pkgs.lib.range 1 3);
in
assert final == original;
pkgs.runCommand "peasy-nixpkgs-reference-check" { } "touch $out"
