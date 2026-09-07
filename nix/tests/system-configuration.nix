{ pkgs, package }:
pkgs.runCommand "peasy-system-configuration-check" { nativeBuildInputs = [ pkgs.nix ]; } ''
  ${package}/libexec/peasy-system --render-test-setup > managed.nix
  export XDG_CACHE_HOME="$TMPDIR/cache"
  nix-instantiate --eval --strict --readonly-mode --store dummy:// \
    ${./system-configuration-eval.nix} \
    --arg nixpkgs ${pkgs.path} \
    --arg managed "$PWD/managed.nix" \
    --argstr system ${pkgs.stdenv.hostPlatform.system} | grep -qx true
  touch $out
''
