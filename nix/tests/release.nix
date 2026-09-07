{ pkgs, releaseTools }:
pkgs.runCommand "peasy-release-checks"
  {
    nativeBuildInputs = [
      releaseTools
      pkgs.nodejs
      pkgs.actionlint
    ];
    # Missing SDK dependencies must fail here, not silently skip coverage.
    PEASY_REQUIRE_R2_SDK = "1";
  }
  ''
    mkdir -p assets .github
    cp -r ${../../scripts} scripts
    cp -r ${../../.github/workflows} .github/workflows
    cp ${../../assets/downloads.js} assets/downloads.js
    cp ${../../index.html} index.html
    # These tests use fake credentials, stubbed SDK requests and temporary
    # fixtures. The Nix sandbox prevents accidental access to cloud services.
    python3 -B -m unittest discover -s scripts/tests -v --buffer
    node --test scripts/tests/downloads.test.cjs
    actionlint .github/workflows/*.yml
    touch $out
  ''
