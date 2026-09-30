{
  config,
  lib,
  pkgs,
  ...
}:
let
  model = import ./ollama-model.nix { inherit pkgs; };
in
{
  # Shared by live media and its installed target; manual Peasy installs do
  # not import this module. No model is downloaded or loaded into RAM at boot.
  services.peasy.ollama.enable = lib.mkDefault true;
  services.ollama = {
    package = lib.mkDefault pkgs.ollama-cpu;
    host = lib.mkDefault "127.0.0.1";
    port = lib.mkDefault 11434;
    openFirewall = lib.mkDefault false;
    environmentVariables = {
      OLLAMA_NO_CLOUD = "1";
      OLLAMA_KEEP_ALIVE = "2m";
      OLLAMA_MAX_LOADED_MODELS = "1";
      OLLAMA_NUM_PARALLEL = "1";
    };
  };

  environment.sessionVariables.PEASY_DEFAULT_OLLAMA_MODEL = lib.mkDefault "qwen3:0.6b";

  systemd.services.ollama = lib.mkIf config.services.ollama.enable {
    serviceConfig.StateDirectory = [ "ollama/models" ];
    preStart = ''
      # Symlink immutable weights instead of copying 523 MB into the live
      # session's writable overlay. Keep a writable catalogue for user pulls.
      mkdir -p "$OLLAMA_MODELS/blobs" "$OLLAMA_MODELS/manifests/registry.ollama.ai/library/qwen3"
      for blob in ${model}/blobs/*; do
        target="$OLLAMA_MODELS/blobs/''${blob##*/}"
        if [ ! -e "$target" ]; then
          ln -s "$blob" "$target"
        fi
      done
      manifest="$OLLAMA_MODELS/manifests/registry.ollama.ai/library/qwen3/0.6b"
      if [ ! -e "$manifest" ]; then
        cp ${model}/manifests/registry.ollama.ai/library/qwen3/0.6b "$manifest"
      fi
    '';
  };
}
