{
  pkgs,
  # Pass the original source from the override to avoid overlay recursion.
  installerSource ? pkgs.path + "/pkgs/by-name/ca/calamares-nixos-extensions/src",
}:
pkgs.runCommand "peasy-iso-branding"
  {
    nativeBuildInputs = [
      pkgs.librsvg
    ];
    FONTCONFIG_FILE = pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; };
  }
  ''
    mkdir -p $out/grub
    # Render trusted vector artwork with pinned fonts at build time. Neither
    # firmware nor the installer needs a font fallback or network access.
    cp ${installerSource}/branding/nixos/nixos-logomark-default-flat-none.svg nixos.svg
    cp ${./installer-welcome.svg} installer-welcome.svg
    rsvg-convert installer-welcome.svg -o $out/installer-welcome.png
    cp -r ${pkgs.nixos-grub2-theme}/. $out/grub/
    chmod u+w $out/grub/{theme.txt,background.png,select_c.png}
    # Preserve the upstream logo, fonts, icons, panels and menu geometry.
    # Only the code-native colour swatches and progress colours change.
    rsvg-convert ${./grub-background.svg} -o $out/grub/background.png
    rsvg-convert ${./grub-selection.svg} -o $out/grub/select_c.png
    substituteInPlace $out/grub/theme.txt \
      --replace-fail '#5579C4' '#246347' \
      --replace-fail '#7EBAE4' '#5cd698'
    cat ${./grub-credit.txt} >> $out/grub/theme.txt
  ''
