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
        // nixpkgs.lib.optionalAttrs (system == "x86_64-linux") {
          iso-gnome = self.nixosConfigurations.peasy-iso-gnome.config.system.build.isoImage;
          iso-plasma = self.nixosConfigurations.peasy-iso-plasma.config.system.build.isoImage;
        }
      );

      nixosModules.default = import ./nix/module.nix;

      nixosConfigurations = {
        peasy-iso-gnome = nixpkgs.lib.nixosSystem {
          system = "x86_64-linux";
          modules = [ ./nix/iso-gnome.nix ];
        };
        peasy-iso-plasma = nixpkgs.lib.nixosSystem {
          system = "x86_64-linux";
          modules = [ ./nix/iso-plasma.nix ];
        };
      };

      # CI requires these flags plus successful runtime tests and verified
      # uploads before publishing. Oversized ISOs use lossless split assets.
      lib.isoReleaseStatus = {
        releaseReady = true;
        installedTargetHasPeasy = true;
        installedBootVerified = true;
        reason = "Installed-disk boot verified. Tag releases publish after CI checks; oversized ISOs are distributed as verified lossless parts.";
      };

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
                  ${./nix/iso-common.nix} \
                  ${./nix/iso-appearance.nix} \
                  ${./nix/iso-gnome.nix} \
                  ${./nix/iso-plasma.nix} \
                  ${./nix/installer.nix} \
                  ${./nix/installer-target.nix} \
                  ${./nix/tests/desktop-config.nix} \
                  ${./nix/tests/iso-config.nix} \
                  ${./nix/tests/gnome-tray.nix} \
                  ${./nix/tests/plasma-tray.nix} \
                  ${./nix/tests/desktop-session.nix} \
                  ${./nix/tests/installer-boot.nix} \
                  ${./nix/tests/installed-instrumentation.nix} \
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
          desktop-config = import ./nix/tests/desktop-config.nix {
            inherit pkgs;
            module = self.nixosModules.default;
            package = self.packages.${system}.default;
          };
          plasma-tray = import ./nix/tests/plasma-tray.nix {
            inherit pkgs;
            module = self.nixosModules.default;
            package = self.packages.${system}.default;
          };
          installer-target =
            let
              installer = import ./nix/installer.nix { inherit pkgs; };
            in
            pkgs.runCommand "peasy-installer-target-check"
              {
                nativeBuildInputs = [ pkgs.python3 ];
              }
              ''
                mkdir -p $out
                cd $out
                python ${./nix/tests/installer-helper.py} ${installer.script}
                python ${./nix/tests/installer-target.py} \
                  ${pkgs.calamares-nixos-extensions.src}/modules/nixos/main.py \
                  ${installer.extensions}/lib/calamares/modules/nixos/main.py \
                  ${installer.helper} > status.json
              '';
          iso-config = import ./nix/tests/iso-config.nix {
            inherit pkgs;
            configurations = self.nixosConfigurations;
            releaseStatus = self.lib.isoReleaseStatus;
          };
        }
        // pkgs.lib.optionalAttrs (system == "x86_64-linux") {
          installed-gnome = import ./nix/tests/installer-boot.nix {
            inherit pkgs;
            package = self.packages.${system}.default;
            desktop = "gnome";
          };
          installed-plasma = import ./nix/tests/installer-boot.nix {
            inherit pkgs;
            package = self.packages.${system}.default;
            desktop = "plasma";
          };
        }
      );
    };
}
