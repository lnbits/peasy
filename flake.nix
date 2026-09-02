{
  description = "Peasy — Tell your computer what you want.";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs =
    { self, nixpkgs }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.callPackage ./nix/package.nix { };
          peasy = pkgs.callPackage ./nix/package.nix { };
          peasy-core = pkgs.callPackage ./nix/package-core.nix { };
        }
      );

      nixosModules.default = import ./nix/module.nix;

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.default ];
            packages = with pkgs; [
              cargo
              clippy
              nixfmt
              pkg-config
              rustc
              rustfmt
              wasm-tools
            ];
          };
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          corePackage = self.packages.${system}.peasy-core;
          coreClosure = pkgs.closureInfo {
            rootPaths = [ corePackage ];
          };
        in
        {
          package = self.packages.${system}.default;
          core-package =
            pkgs.runCommand "peasy-core-package-check" { nativeBuildInputs = [ pkgs.gnugrep ]; }
              ''
                test -x ${corePackage}/bin/peasy
                test -x ${corePackage}/libexec/peasy-system
                test -f ${corePackage}/lib/peasy/peasy-engine.wasm
                test ! -e ${corePackage}/bin/peasy-ui
                test ! -e ${corePackage}/bin/peasy-tray
                test ! -e ${corePackage}/share/applications/io.github.peasy.Peasy.desktop
                if grep -E '/(gtk4|libadwaita|gnome-shell)-' ${coreClosure}/store-paths; then
                  echo "peasy-core unexpectedly contains a graphical dependency" >&2
                  exit 1
                fi
                touch $out
              '';
          formatting = pkgs.runCommand "peasy-nix-format" { nativeBuildInputs = [ pkgs.nixfmt ]; } ''
                nixfmt --check \
                  ${./flake.nix} \
                  ${./nix/module.nix} \
                  ${./nix/package.nix} \
                  ${./nix/package-core.nix} \
                  ${./nix/tests/gnome-tray.nix} \
                  ${./nix/tests/sandbox.nix}
            touch $out
          '';
          wasm-imports = pkgs.runCommand "peasy-wasm-imports" { nativeBuildInputs = [ pkgs.wasm-tools ]; } ''
            wasm-tools print ${self.packages.${system}.default}/lib/peasy/peasy-engine.wasm > engine.wat
            if grep -q '^[[:space:]]*(import ' engine.wat; then
              echo "peasy-engine.wasm unexpectedly imports a host capability" >&2
              exit 1
            fi
            touch $out
          '';
          sandbox = import ./nix/tests/sandbox.nix {
            inherit pkgs;
            module = self.nixosModules.default;
            package = corePackage;
          };
          gnome-tray = import ./nix/tests/gnome-tray.nix {
            inherit pkgs;
            module = self.nixosModules.default;
            package = self.packages.${system}.default;
          };
        }
      );
    };
}
