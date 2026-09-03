# Installing Peasy

Peasy is a NixOS module. It works with a traditional `configuration.nix` or a
flake-based host and supports x86-64 and AArch64 Linux.

## Standard NixOS configuration

Clone Peasy under `/etc/nixos`:

```console
sudo git clone https://github.com/lnbits/peasy /etc/nixos/peasy
```

Update `/etc/nixos/configuration.nix`. Add `lib` to the function arguments and
extend the existing `imports` list as shown:

```nix
{ config, pkgs, lib, ... }:

{
  imports = [
    ./hardware-configuration.nix
    ./peasy/nix/module.nix
  ] ++ lib.optional
    (builtins.pathExists ./.peasy/peasy-managed.nix)
    ./.peasy/peasy-managed.nix;

  services.peasy.enable = true;
}
```

Keep any other imports and options already present in the file. Then rebuild:

```console
sudo nixos-rebuild switch --no-flake
sudo systemctl restart peasy-system
```

The optional import lets the first rebuild succeed before the managed file
exists. Peasy creates `.peasy/peasy-managed.nix` and uses it as its only durable
state.

On GNOME, log out and back in after installing or upgrading so GNOME Shell
loads the bundled panel extension. On Hyprland, ensure the bar has a
StatusNotifier tray, such as Waybar's `tray` module.

## Development checkout in a home directory

When importing Peasy from a protected home directory, permit the system service
to read that checkout:

```nix
{ config, pkgs, lib, ... }:

{
  imports = [
    ./hardware-configuration.nix
    /home/alice/src/peasy/nix/module.nix
  ] ++ lib.optional
    (builtins.pathExists ./.peasy/peasy-managed.nix)
    ./.peasy/peasy-managed.nix;

  services.peasy = {
    enable = true;
    configurationReadPaths = [ "/home/alice/src/peasy" ];
  };
}
```

Replace the example user and path with the actual checkout location.

## Flake-based host

Add Peasy as an input and include its module in the host:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    peasy.url = "github:lnbits/peasy";
  };

  outputs = { nixpkgs, peasy, ... }: {
    nixosConfigurations.my-host = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        peasy.nixosModules.default
        ./configuration.nix
        {
          services.peasy = {
            enable = true;
            hostFlake = "/etc/nixos#my-host";
          };
        }
      ];
    };
  };
}
```

The host's `configuration.nix` must contain the same optional managed-file
import used in the standard example:

```nix
imports = [
  ./hardware-configuration.nix
] ++ lib.optional
  (builtins.pathExists ./.peasy/peasy-managed.nix)
  ./.peasy/peasy-managed.nix;
```

Build the local path explicitly so a newly created, untracked managed file is
included:

```console
sudo nixos-rebuild switch --flake path:/etc/nixos#my-host
sudo systemctl restart peasy-system
```

## Headless installation

Disable desktop integration on servers or minimal systems:

```nix
services.peasy = {
  enable = true;
  desktop.enable = false;
};
```

This installs the CLI, policy engine, and system service without GTK, GNOME, or
Hyprland components.

## Choose an AI provider

The first launch opens provider setup. OpenAI requires an API key. The key is
stored for the current user in `~/.config/peasy/openai-key` with mode `0600`.

For a local provider, enable Ollama:

```nix
services.peasy = {
  enable = true;
  ollama.enable = true;
};
```

After rebuilding, install a model and select it from Peasy's settings:

```console
ollama pull qwen3:8b
```

Provider setup is also available in the terminal:

```console
peasy --setup-provider
```

## Upgrade

Update the Peasy source or flake lock, rebuild the host, and restart the daemon:

```console
sudo nixos-rebuild switch --no-flake
sudo systemctl restart peasy-system
```

Use the flake rebuild command from the flake section when applicable. Restarting
after an administrator-initiated upgrade ensures the running daemon and UI use
the same version. Peasy deliberately remains running during a change it applies
itself so it can report the completed result.
