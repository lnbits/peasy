{ pkgs }:
let
  package = pkgs.callPackage ../package.nix { };
  # A channel contains the same package definitions at a different source path.
  # Neither the desktop nor core package may embed that path just for a test:
  # the offline installer must reuse the package already cached on the ISO.
  relocated = pkgs.extend (_final: _previous: { path = /peasy-test-relocated-nixpkgs; });
in
assert package.drvPath == (relocated.callPackage ../package.nix { }).drvPath;
assert
  (pkgs.callPackage ../package-core.nix { }).drvPath
  == (relocated.callPackage ../package-core.nix { }).drvPath;
(pkgs.callPackage ../package-core.nix { }).overrideAttrs (old: {
  pname = "peasy-export-check";
  # Compile the production exporter in a small test harness. It needs no GTK
  # initialization, Wasm build or second build of the whole application.
  postPatch = (old.postPatch or "") + ''
    mkdir -p crates/peasy-core/tests
    cat > crates/peasy-core/tests/configuration_export.rs <<'EOF'
    #[allow(dead_code)]
    mod export {
        use nix::libc;
        include!("../../peasy-ui/src/export.rs");
    }
    EOF
  '';
  cargoBuildFlags = [
    "-p"
    "peasy-core"
  ];
  cargoTestFlags = [
    "-p"
    "peasy-core"
    "--test"
    "configuration_export"
  ];
  checkFlags = [
    "--test-threads=1"
    "--include-ignored"
  ];
  preBuild = "";
  preCheck = ''
    export PEASY_TEST_NIX="${pkgs.nix}/bin/nix-instantiate"
    export PEASY_TEST_NIXPKGS="${pkgs.path}"
    export PEASY_TEST_SYSTEM="${pkgs.stdenv.hostPlatform.system}"
  '';
  installPhase = ''
    mkdir -p $out
    touch $out/passed
  '';
  postInstall = "";
})
