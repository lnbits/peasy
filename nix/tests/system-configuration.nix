{ pkgs, package }:
pkgs.runCommand "peasy-system-configuration-check" { nativeBuildInputs = [ pkgs.nix ]; } ''
  ${package}/libexec/peasy-system --render-test-setup > managed.nix
  ${package}/libexec/peasy-system --render-test-postgresql > postgresql.nix
  ${package}/libexec/peasy-system --render-test-capabilities > capabilities.nix
  export XDG_CACHE_HOME="$TMPDIR/cache"
  nix-instantiate --eval --strict --readonly-mode --store dummy:// \
    ${./system-configuration-eval.nix} \
    --arg nixpkgs ${pkgs.path} \
    --arg managed "$PWD/managed.nix" \
    --arg postgresql "$PWD/postgresql.nix" \
    --arg capabilities "$PWD/capabilities.nix" \
    --argstr system ${pkgs.stdenv.hostPlatform.system} | grep -qx true
  touch $out
''
