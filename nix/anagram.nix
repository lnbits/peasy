{
  lib,
  appimageTools,
  fetchurl,
  desktop-file-utils,
}:
let
  pname = "anagram";
  version = "0.8.2";
  # Latest stable AppImage verified on 2026-10-04. Update URL and hash together.
  src = fetchurl {
    url = "https://github.com/lnbits/anagram/releases/download/v${version}/anagram-linux.AppImage";
    hash = "sha256-+UlZH8tk6moN8zczBrAuKedz8t9z9W2eL/C8BOIT+dE=";
  };
  contents = appimageTools.extract { inherit pname version src; };
in
appimageTools.wrapType2 {
  inherit pname version src;
  extraInstallCommands = ''
    install -Dm644 ${contents}/anagram.desktop $out/share/applications/anagram.desktop
    substituteInPlace $out/share/applications/anagram.desktop \
      --replace-fail 'Exec=AppRun ' "Exec=$out/bin/anagram "
    install -Dm644 ${contents}/usr/share/icons/hicolor/512x512/apps/anagram.png \
      $out/share/icons/hicolor/512x512/apps/anagram.png
    ${desktop-file-utils}/bin/desktop-file-validate $out/share/applications/anagram.desktop
    test -x $out/bin/anagram
  '';
  meta = {
    description = "Private messaging client for Nostr";
    homepage = "https://github.com/lnbits/anagram";
    license = lib.licenses.mit;
    platforms = [ "x86_64-linux" ];
    mainProgram = "anagram";
  };
}
