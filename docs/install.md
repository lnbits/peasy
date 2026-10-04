# Install Peasy

For a new machine, use the [installer ISO](iso.md). For an existing NixOS host,
use either configuration method below. Your user needs `wheel` membership and a
Polkit authentication agent for system changes. Run Peasy as your normal user.

## Standard NixOS configuration

```sh
sudo git clone https://github.com/lnbits/peasy /etc/nixos/peasy
```

Add these imports and option to `/etc/nixos/configuration.nix`, retaining your
other settings. Include `lib` in the function arguments:

```nix
{ lib, ... }:
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

```sh
sudo nixos-rebuild switch --no-flake
sudo systemctl restart peasy-system
```

Log out and back in, then open Peasy from the application menu or tray.

## Flake-based host

Add the Peasy input and module to your existing flake:

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
          services.peasy.enable = true;
          services.peasy.hostFlake = "/etc/nixos#my-host";
        }
      ];
    };
  };
}
```

Keep the optional `.peasy/peasy-managed.nix` import from the standard example in
`configuration.nix`; omit `./peasy/nix/module.nix` because the flake supplies it.
Replace `my-host` and architecture as appropriate. Use `path:` so untracked
managed configuration is included:

```sh
sudo nixos-rebuild switch --flake path:/etc/nixos#my-host
sudo systemctl restart peasy-system
```

## Choose an AI provider

Open **Settings → AI provider**, or run `peasy --setup-provider`.

- **Ollama:** Peasy detects and tries to start an existing installation. If absent,
  choose **Install and enable Ollama** and review the system change. Select a model
  to download, then **Save provider**. Downloads show approximate sizes; inference
  needs additional RAM. Ollama 0.30.6 or newer is required.
- **OpenAI:** enter your API key and select a model. The key is stored privately
  for your user; API usage is billed by the provider.

Installed Ollama models remain selectable. **Remove model** requires confirmation;
choose and save a replacement before removing the active model. The optional model
catalogue refreshes automatically, but weights and your selection do not.
The ISO's bundled Qwen3 files remain in the Nix store and are restored on service restart.

## Optional configuration

```nix
# CLI-only installation:
services.peasy.desktop.enable = false;

# Start local Ollama at boot:
services.peasy.ollama.enable = true;
```

Manual installations do not automatically enable NetworkManager or Bluetooth.
Enable `networking.networkmanager.enable` and `hardware.bluetooth.enable` only
when wanted. For a checkout under a protected home directory, add its absolute
path to `services.peasy.configurationReadPaths`.

GNOME and Plasma supply authentication dialogs. Hyprland's Peasy agent can be
disabled with `services.peasy.hyprland.authenticationAgent.enable = false` if you
already run one. Other sessions need an agent or the interactive CLI.

## Local development

```sh
nix build
./result/bin/peasy-ui
```

Stop any older Peasy UI first. A local UI build does not update the installed
system service; rebuild the host when changing daemon or module behaviour.
New source files must be added to Git for Git-based Nix builds.

## Troubleshooting

- **Old version running:** finish active work, restart `peasy-system`, then reopen Peasy.
- **Interrupted change:** use **System status and recovery**, `peasy --status` or
  `peasy --recover`. Recovery can switch the whole system generation.
- **Slow search/build:** cold searches can use several GiB. The service defaults
  to a 6 GiB ceiling; adjust `services.peasy.resourceLimits.memoryMax` if needed.
- **Ollama:** run `ollama ps`; launch Peasy with `PEASY_OLLAMA_DIAGNOSTICS=1` for
  token/timing diagnostics without prompt contents.
- **Offline:** “Sorry, no internet connection” appears only when the OS confirms
  no usable addressed interface. DNS, authentication and server failures retain
  their own errors.

[Desktop/tray help](desktop-compatibility.md) · [Updates](updates.md) · [Backups](backups.md)
