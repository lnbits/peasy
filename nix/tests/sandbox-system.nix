{ pkgs }:

# Shared by the test builder and the configuration evaluated inside its VM.
# Keep this as source: embedding a .drv path in generated text depends on that
# derivation already existing during read-only `nix flake check --no-build`.
let
  fakeSwitch = pkgs.writeShellScript "peasy-test-switch-to-configuration" ''
    exit 0
  '';
in
pkgs.runCommand "peasy-test-nixos-system" { } ''
  mkdir -p $out/bin
  ln -s ${fakeSwitch} $out/bin/switch-to-configuration
  mkdir -p $out/etc/peasy
  echo '{"packages":["hello"],"appimages":[],"theme":{"accent_color":null,"color_scheme":null}}' > $out/etc/peasy/state.json
''
