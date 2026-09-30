{ pkgs }:
let
  # The checked-in registry manifest pins weights, template, parameters and
  # license together. Fetch by digest, never by a mutable tag during a build.
  manifest = builtins.fromJSON (builtins.readFile ./qwen3-0.6b.json);
  blobs = map (
    entry:
    let
      hash = pkgs.lib.removePrefix "sha256:" entry.digest;
    in
    {
      name = "sha256-${hash}";
      source = pkgs.fetchurl {
        url = "https://registry.ollama.ai/v2/library/qwen3/blobs/${entry.digest}";
        sha256 = hash;
      };
    }
  ) ([ manifest.config ] ++ manifest.layers);
in
pkgs.runCommand "peasy-qwen3-0.6b" { } ''
  mkdir -p "$out/blobs" "$out/manifests/registry.ollama.ai/library/qwen3"
  cp ${./qwen3-0.6b.json} "$out/manifests/registry.ollama.ai/library/qwen3/0.6b"
  ${pkgs.lib.concatMapStringsSep "\n" (
    blob: ''ln -s ${blob.source} "$out/blobs/${blob.name}"''
  ) blobs}
''
