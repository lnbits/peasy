{ package }:
# A test-only binary linked to the real resource adapters. It is deliberately
# absent from the installed application and privileged IPC surface.
package.overrideAttrs (_: {
  pname = "peasy-resource-test-tools";
  cargoBuildFlags = [
    "-p"
    "peasy-core"
    "--example"
    "resource_session"
  ];
  preBuild = "";
  preCheck = "";
  doCheck = false;
  installPhase = ''
    mkdir -p $out/bin
    find target -path '*/release/examples/resource_session' -type f -exec cp {} $out/bin/ \;
  '';
  postInstall = "";
  preFixup = "";
})
