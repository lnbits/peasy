{
  pkgs,
  source ? (pkgs.callPackage ./package.nix { }).src,
}:
let
  artwork = import ./iso-branding { inherit pkgs; };
  script = pkgs.replaceVars ./installer-target.py {
    inherit source;
    module = "${./installer-target.nix}";
  };
  helper = pkgs.writeShellScript "peasy-install-target" ''
    exec ${pkgs.python3}/bin/python3 ${script} "$@"
  '';
  extensions = pkgs.calamares-nixos-extensions.overrideAttrs (old: {
    patchFlags = [
      "-p1"
      "--fuzz=0"
    ];
    patches = (old.patches or [ ]) ++ [ ./calamares-peasy.patch ];
    postPatch = (old.postPatch or "") + ''
      substituteInPlace modules/nixos/main.py \
        --replace-fail '@peasyTargetInstaller@' '${helper}'
      # Retain upstream's modules, slides, translations and desktop previews.
      # Branding is separate from the target system configuration generator.
      cp -r branding/nixos branding/peasy
      cp ${artwork}/installer-welcome.png branding/peasy/peasy-welcome.png
      substituteInPlace config/settings.conf \
        --replace-fail 'branding: nixos' 'branding: peasy'
      substituteInPlace branding/peasy/branding.desc \
        --replace-fail 'componentName:  nixos' 'componentName:  peasy' \
        --replace-fail 'productWelcome:      "nixos-logomark-default-flat-none.svg"' 'productWelcome:      "peasy-welcome.png"' \
        --replace-fail '"#5277C3"' '"#246347"' \
        --replace-fail '"#292F34"' '"#101715"' \
        --replace-fail '"#7EBAE4"' '"#5cd698"'
    '';
  });
in
{
  inherit
    source
    script
    helper
    extensions
    ;
}
