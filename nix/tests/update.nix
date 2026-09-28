{ pkgs, package }:
pkgs.runCommand "peasy-update-checks" { nativeBuildInputs = [ pkgs.nix ]; } ''
  ${package}/libexec/peasy-system --render-test-update > managed.nix
  # Only the network fetch is substituted: evaluate the real renderer, module,
  # package provenance assertion and host settings against this checkout offline.
  substituteInPlace managed.nix --replace-fail \
    'builtins.fetchTarball { url = "https://codeload.github.com/lnbits/peasy/tar.gz/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"; sha256 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"; }' \
    '${../..}'
  export XDG_CACHE_HOME="$TMPDIR/cache"
  nix-instantiate --eval --strict --readonly-mode --store dummy:// \
    ${./update-eval.nix} --arg nixpkgs ${pkgs.path} --arg source ${../..} \
    --arg managed "$PWD/managed.nix" --argstr system ${pkgs.stdenv.hostPlatform.system} | grep -qx true
  touch $out
''
