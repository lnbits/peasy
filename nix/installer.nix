{
  pkgs,
  source ? (pkgs.callPackage ./package.nix { }).src,
}:
let
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
