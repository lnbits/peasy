{
  lib,
  stdenv,
  rustPlatform,
  pkg-config,
  gtk4,
  libadwaita,
  lld,
  makeWrapper,
  networkmanager,
  bluez,
  glib,
  coreutils,
  nix,
  polkit,
  cacert,
  withGui ? true,
}:

rustPlatform.buildRustPackage {
  pname = if withGui then "peasy" else "peasy-core";
  version = "0.1.0";

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions (
      [
        ../Cargo.lock
        ../Cargo.toml
        ../crates
        ../flake.lock
        ../flake.nix
        ../nix
        ../wit
      ]
      ++ lib.optionals withGui [ ../assets ]
    );
  };

  cargoLock.lockFile = ../Cargo.lock;

  # Offline provider tests still construct a TLS client, which requires roots.
  SSL_CERT_FILE = "${cacert}/etc/ssl/certs/ca-bundle.crt";

  nativeBuildInputs = [
    makeWrapper
    pkg-config
    lld
  ];
  buildInputs = lib.optionals withGui [
    gtk4
    libadwaita
  ];

  cargoBuildFlags = [
    "--workspace"
    "--exclude"
    "peasy-engine"
  ]
  ++ lib.optionals (!withGui) [
    "--exclude"
    "peasy-ui"
    "--exclude"
    "peasy-tray"
  ];
  cargoTestFlags = [
    "--workspace"
    "--exclude"
    "peasy-engine"
  ]
  ++ lib.optionals (!withGui) [
    "--exclude"
    "peasy-ui"
    "--exclude"
    "peasy-tray"
  ];

  preBuild = ''
    cargo build --release --locked -p peasy-engine --target wasm32-unknown-unknown
  '';

  postInstall = ''
    install -Dm755 target/${stdenv.hostPlatform.rust.rustcTarget}/release/peasy "$out/bin/peasy"
    install -Dm755 target/${stdenv.hostPlatform.rust.rustcTarget}/release/peasy-system "$out/libexec/peasy-system"
    install -Dm644 target/wasm32-unknown-unknown/release/peasy_engine.wasm \
      "$out/lib/peasy/peasy-engine.wasm"
    ${lib.optionalString withGui ''
        install -Dm755 target/${stdenv.hostPlatform.rust.rustcTarget}/release/peasy-ui "$out/bin/peasy-ui"
        install -Dm755 target/${stdenv.hostPlatform.rust.rustcTarget}/release/peasy-tray "$out/bin/peasy-tray"
      install -Dm644 assets/io.github.peasy.Peasy.desktop \
        "$out/share/applications/io.github.peasy.Peasy.desktop"
      install -Dm644 assets/io.github.peasy.Peasy-symbolic.svg \
        "$out/share/icons/hicolor/symbolic/apps/io.github.peasy.Peasy-symbolic.svg"
      install -Dm644 assets/io.github.peasy.Peasy.svg \
        "$out/share/icons/hicolor/scalable/apps/io.github.peasy.Peasy.svg"
      extension="$out/share/gnome-shell/extensions/peasy@peasy-nixos.github.io"
      install -Dm644 assets/gnome-shell-extension/metadata.json "$extension/metadata.json"
      install -Dm644 assets/gnome-shell-extension/extension.js "$extension/extension.js"
      install -Dm644 assets/gnome-shell-extension/stylesheet.css "$extension/stylesheet.css"
      mkdir -p "$out/share/peasy/source"
      cp -R Cargo.lock Cargo.toml flake.lock flake.nix crates nix wit assets \
        "$out/share/peasy/source/"
    ''}
    ${
      if withGui then
        ''
            wrapProgram "$out/bin/peasy" \
              --set-default PEASY_ENGINE "$out/lib/peasy/peasy-engine.wasm" \
              --set-default PEASY_PKTTYAGENT "${polkit}/bin/pkttyagent" \
              --set-default PEASY_NIX "${nix}/bin/nix" \
              --set-default PEASY_DATE "${coreutils}/bin/date" \
          --set-default PEASY_NMCLI "${networkmanager}/bin/nmcli" \
          --set-default PEASY_BLUETOOTHCTL "${bluez}/bin/bluetoothctl" \
          --set-default PEASY_GIO "${glib}/bin/gio" \
              --set-default PEASY_GSETTINGS "${glib}/bin/gsettings" \
              --set-default PEASY_VARIANT "desktop" \
              --set-default PEASY_NIX_SYSTEM "${stdenv.hostPlatform.system}"
        ''
      else
        ''
          wrapProgram "$out/bin/peasy" \
            --set-default PEASY_ENGINE "$out/lib/peasy/peasy-engine.wasm" \
            --set-default PEASY_PKTTYAGENT "${polkit}/bin/pkttyagent" \
            --set-default PEASY_NIX "${nix}/bin/nix" \
            --set-default PEASY_DATE "${coreutils}/bin/date" \
            --set-default PEASY_VARIANT "core" \
            --set-default PEASY_NIX_SYSTEM "${stdenv.hostPlatform.system}"
        ''
    }
    ${lib.optionalString withGui ''
      wrapProgram "$out/bin/peasy-ui" \
        --set-default PEASY_ENGINE "$out/lib/peasy/peasy-engine.wasm" \
        --set-default PEASY_NMCLI "${networkmanager}/bin/nmcli" \
        --set-default PEASY_BLUETOOTHCTL "${bluez}/bin/bluetoothctl" \
        --set-default PEASY_GIO "${glib}/bin/gio" \
        --set-default PEASY_GSETTINGS "${glib}/bin/gsettings" \
        --set-default PEASY_NIX "${nix}/bin/nix" \
        --set-default PEASY_DATE "${coreutils}/bin/date" \
        --set-default PEASY_VARIANT "desktop" \
        --set-default PEASY_NIX_SYSTEM "${stdenv.hostPlatform.system}"
    ''}
  '';

  doCheck = true;

  meta = {
    description =
      if withGui then
        "Typed natural-language assistant for NixOS, GNOME, and Hyprland"
      else
        "Headless typed natural-language assistant and service for NixOS";
    homepage = "https://github.com/lnbits/peasy";
    license = lib.licenses.mit;
    mainProgram = "peasy";
    platforms = lib.platforms.linux;
  };
}
