{
  pkgs,
  sandboxTest,
}:
let
  # Evaluate exactly the configuration used by the daemon inside the sandbox
  # VM. This must recreate the same fixture as the host without importing an
  # already-existing .drv file. No VM boot or native package build is needed.
  configuration =
    pkgs.writeText "peasy-sandbox-host-configuration.nix"
      sandboxTest.config.nodes.machine.environment.etc."nixos/configuration.nix".text;
  states =
    map
      (accent_color: {
        packages = [ "hello" ];
        appimages = [ ];
        theme = {
          inherit accent_color;
          color_scheme = null;
        };
      })
      [
        null
        "blue"
        "green"
      ];
  expression = pkgs.writeText "peasy-sandbox-fixture-check.nix" ''
    let
      system = "${pkgs.stdenv.hostPlatform.system}";
      pkgs = import ${pkgs.path} { inherit system; };
      evaluated = import ${pkgs.path}/nixos/lib/eval-config.nix {
        inherit system;
        modules = [ (builtins.toPath (builtins.getEnv "PEASY_TEST_CONFIGURATION")) ];
      };
      state = builtins.fromJSON (builtins.getEnv "PEASY_TEST_STATE");
      expected = import ${./sandbox-system.nix} { inherit pkgs state; };
    in
    assert evaluated.config.system.build.toplevel.drvPath == expected.drvPath;
    true
  '';
in
pkgs.runCommand "peasy-sandbox-fixture-check" { nativeBuildInputs = [ pkgs.nix ]; } ''
  # Read the generated file only at build time, when it exists. A dummy store
  # prevents a populated host store from hiding missing .drv dependencies.
  export XDG_CACHE_HOME="$TMPDIR/cache"
  mkdir -p "$TMPDIR/host/.peasy"
  cp ${configuration} "$TMPDIR/host/configuration.nix"
  export PEASY_TEST_CONFIGURATION="$TMPDIR/host/configuration.nix"
  export PEASY_TEST_STATE='{}'
  nix-instantiate --eval --strict --readonly-mode --store dummy:// ${expression} | grep -qx true
  ${pkgs.lib.concatMapStringsSep "\n" (state: ''
    # Use the same relative managed-module import as a real host. Each colour
    # must produce a distinct generation instead of the old hard-coded state.
    install -m 0644 ${pkgs.writeText "peasy-sandbox-managed.nix" ''
      { ... }: {
        environment.etc."peasy/state.json".text = ${builtins.toJSON (builtins.toJSON state)};
      }
    ''} "$TMPDIR/host/.peasy/peasy-managed.nix"
    export PEASY_TEST_STATE=${pkgs.lib.escapeShellArg (builtins.toJSON state)}
    nix-instantiate --eval --strict --readonly-mode --store dummy:// ${expression} | grep -qx true
  '') states}
  touch $out
''
