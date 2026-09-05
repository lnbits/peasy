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
  expression = pkgs.writeText "peasy-sandbox-fixture-check.nix" ''
    let
      system = "${pkgs.stdenv.hostPlatform.system}";
      pkgs = import ${pkgs.path} { inherit system; };
      evaluated = import ${pkgs.path}/nixos/lib/eval-config.nix {
        inherit system;
        modules = [ ${configuration} ];
      };
      expected = import ${./sandbox-system.nix} { inherit pkgs; };
    in
    assert evaluated.config.system.build.toplevel.drvPath == expected.drvPath;
    true
  '';
in
pkgs.runCommand "peasy-sandbox-fixture-check" { nativeBuildInputs = [ pkgs.nix ]; } ''
  # Read the generated file only at build time, when it exists. A dummy store
  # prevents a populated host store from hiding missing .drv dependencies.
  export XDG_CACHE_HOME="$TMPDIR/cache"
  nix-instantiate --eval --strict --readonly-mode --store dummy:// ${expression} | grep -qx true
  touch $out
''
