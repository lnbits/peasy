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
exists. Peasy creates `.peasy/peasy-managed.nix` and uses it as its desired
state.

Peasy desktop does not enable NetworkManager or Bluetooth. If you want those
services, configure them explicitly in your host module:

```nix
networking.networkmanager.enable = true;
hardware.bluetooth.enable = true;
```

Keep your existing network manager unless you intend to change it. Peasy's Wi-Fi
and networking actions require NetworkManager to manage the relevant interface;
Bluetooth actions require BlueZ. A reviewed persistent Peasy network profile
declares NetworkManager as a dependency. The Peasy live ISO and installed target
default both services on independently of the desktop option.

When upgrading a manual installation that relied on Peasy's former desktop
defaults, declare any required services explicitly before rebuilding.

Log out and back in so the generic XDG autostart entry starts Peasy's single
StatusNotifierItem tray. GNOME gets the AppIndicator compatibility extension
only when GNOME is configured; the old Peasy panel extension is disabled on
upgrade to avoid duplicate launchers. Plasma uses its built-in tray. On Hyprland,
ensure the bar has a StatusNotifier tray, such as Waybar's `tray` module, and that
the session runs XDG autostart entries. Peasy remains available in the application
menu when there is no tray host. See [desktop compatibility](desktop-compatibility.md).

The user must belong to `wheel`. Every system change now requires administrator
authentication through Polkit after the diff is accepted. GNOME and Plasma provide
their own authentication dialogs. For NixOS-configured Hyprland, Peasy enables an agent
in the systemd graphical session. If you already run one, set
`services.peasy.hyprland.authenticationAgent.enable = false`. Sessions without
a systemd graphical target must start their own agent. Without one, use `peasy`
in an interactive terminal, where Peasy starts a terminal agent. Never run the
AI-facing UI or CLI with `sudo`, and do not add passwordless Peasy Polkit rules.

Peasy now finishes existing requests and restarts automatically when the active
generation changes its daemon executable, package source, or service settings.
When upgrading from a version without this handoff, wait for any operation to
finish and perform the explicit restart above once. System status and recovery
shows the running executable, protocol and Nixpkgs source.

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

## External AppImage review

Nixpkgs remains the default source. External AppImages are executable code from
outside Nixpkgs. By default, Peasy shows the GitHub repository and release before
download, then the download URL, hash and configuration diff before installation.
Review the source and accept the change; installation still requires administrator
authentication. No manual hash configuration is required. A pinned hash ensures
the same bytes are used later, not that the publisher or application is safe.

For stricter deployments, an administrator can restrict installations to exact
repository/hash pairs after independently verifying their publisher and digest:

```nix
services.peasy.appImages.trustedHashes = {
  "owner/project" = [ "sha256-REPLACE_WITH_VERIFIED_BASE64_DIGEST" ];
};
```

Use lowercase `owner/project`, replace the placeholder, then rebuild normally.
A hash calculated from the same untrusted download is not publisher verification.
With this optional allowlist, new versions need new approval; existing AppImages
can still be removed after their approval is withdrawn. Set `trustedHashes = { };`
to disable new AppImage installs, or `trustedHashes = null;` (the default) to use
source review and administrator authentication without preapproval.

## Resource limits

Cold Nixpkgs searches can consume several GiB. Peasy serializes heavy Nix
operations, bounds IPC and command output, and defaults to a 6 GiB service
memory ceiling. For larger trusted host configurations, adjust
`services.peasy.resourceLimits.memoryMax` (for example `"8G"`). The separately
managed Nix daemon and its build workers have their own resource policy.

## Choose an AI provider

Enter Wi-Fi passwords only in the separate local confirmation field, never in
the natural-language request. Credential-looking requests are refused before
contacting the model, but arbitrary pasted secrets cannot be reliably detected.

On a manual installation, the first launch opens provider setup with **Ollama
(local)** selected by default.
An existing provider choice (including older OpenAI key-only setups) is preserved.
Selecting **Ollama (local)** checks its API first. If it is stopped, Peasy detects
an existing system service, user service, or executable in your PATH or Nix profile
and tries to start it. A standalone executable runs as your desktop user in the
`peasy-ollama.service` user unit, bound to `127.0.0.1:11434`, without enabling it
at boot. It stays available after settings closes and unloads idle models after
two minutes. Peasy does not start Ollama merely because you open OpenAI settings.

If starting a system service needs administrator approval, **Start Ollama** opens
the normal review and authorization flow. If neither a service nor an executable
is found, **Install and enable Ollama** offers a reviewed NixOS installation.
No AI provider is required. After applying, **Done** returns to Ollama settings
and refreshes the models. Installation enables the local endpoint at boot,
without downloading a model or opening a firewall port. Administrator settings
are not forcibly overridden. Installation requires the updated Peasy system service.

You can also enable Ollama using the configuration below, then select a model to
download, or select OpenAI instead. OpenAI requires an API key, stored for the
current user in `~/.config/peasy/openai-key` with mode `0600`.

The [Peasy ISO](iso.md) bundles Ollama and Qwen3 0.6B and supplies
`PEASY_DEFAULT_OLLAMA_MODEL=qwen3:0.6b` in the live and installed desktop sessions.
This fallback does not create or overwrite per-user provider settings and does
not override a legacy OpenAI key. The manual Peasy module adds no such default.

For a local provider, enable Ollama:

```nix
services.peasy = {
  enable = true;
  ollama.enable = true;
};
```

After rebuilding, open **Settings → AI provider → Ollama (local)**. The model selector shows
installed models and a short recommendation list. Its initial download choices are:

| Model | Approximate download |
| --- | --- |
| Qwen3 0.6B (bundled on the ISO) | 523 MB |
| Qwen3 1.7B | 1.4 GB |
| Qwen3 4B | 2.5 GB |
| Qwen3.5 2B | 2.7 GB |
| Qwen3.5 4B | 3.4 GB |
| Qwen3 8B | 5.2 GB |

Sizes are from the [Qwen3](https://ollama.com/library/qwen3) and
[Qwen3.5](https://ollama.com/library/qwen3.5) registries (October 2026), and may
change. Running a model also requires memory for inference and context. These
choices balance size, multilingual coverage and agent support; they have not
been benchmarked across Peasy’s tasks.

Selecting an uninstalled choice starts its download through the local Ollama
service. Progress is per file. **Cancel download** or closing settings cancels
the active request; Ollama manages any cached partial files. Peasy confirms the
model appears in the installed list before making it selectable for use. Choose
**Save provider** to activate it. Opening or refreshing settings never downloads
model weights, and a failed download preserves the saved provider.

Other models already installed in Ollama remain selectable. To add one outside
the download catalogue, use `ollama pull MODEL`, then **Refresh models**.
Only Qwen3 0.6B remains bundled in the ISO; the five additional choices are
optional downloads. Peasy does not start model inference while downloading.

Settings automatically checks Peasy’s published recommendation list on GitHub
when local Ollama is available. Successful lists are cached for six hours within
the running app; **Refresh models** checks again immediately. Offline,
unavailable or invalid lists fall back to the last
valid list in memory, or the list included with Peasy. Installed models remain
listed even when recommendations change. Model weights and the saved provider
are never updated automatically.

To publish new recommendations, edit
[`ollama-catalogue.json`](../crates/peasy-client/src/ollama-catalogue.json) on
`lnbits/peasy`’s `main` branch. Keep format version `1`, bundled `qwen3:0.6b`
first, and exactly five other reviewed choices with approximate download sizes
in bytes. Use explicit official-library tags; namespaces, remote registries,
cloud tags and `latest` are rejected. Check suitability for Peasy’s structured
requests and supported Ollama version before publishing. Clients with this
feature pick up the new list without a Peasy rebuild; it does not discover or
recommend new releases without maintainer review.

Select an installed model and choose **Remove model** to uninstall it after
confirmation. If it is the saved active Ollama model, first select another model
or provider and **Save provider**. Removal affects all apps using that local
Ollama service. Peasy confirms removal with a fresh installed-model list; after
an uncertain result, use **Refresh models** before retrying. Shared model data
may remain if other models use it.

On ISO-based systems, removing bundled Qwen3 0.6B does not free its immutable
Nix store files, and NixOS restores it when Ollama restarts. Optional downloads
are not restored automatically.

Provider setup is also available in the terminal:

```console
peasy --setup-provider
```

Ollama must be version 0.30.6 or newer. Peasy checks the server version, requests
an 8,192-token context, and retries an explicit context-overflow error once with
16,384 tokens. Larger contexts use more memory. Prompt truncation and context
shifting are disabled; an oversized request or incomplete response stops without
producing an action. Peasy requests thinking off where the model supports it.

To diagnose a local model, stop the existing Peasy UI process and launch it from
a terminal. Closing its window only hides it:

```console
pkill -x peasy-ui
PEASY_OLLAMA_DIAGNOSTICS=1 peasy-ui
```

The terminal reports the selected capability scope, instruction/schema byte
sizes, model, requested context, token counts and timings in nanoseconds,
without printing requests, configuration or responses.
Use `ollama ps` while a request runs to check the loaded model and actual context.
An Ollama log warning `truncating input prompt` means input was discarded; it
must not occur for Peasy requests with these controls. The runtime controls do
not establish model accuracy: compare the resulting proposals before applying.

## Upgrade

Update the Peasy source or flake lock, then rebuild the host:

```console
sudo nixos-rebuild switch --no-flake
```

Use the flake rebuild command from the flake section when applicable. Peasy
finishes existing requests, delivers their results, and exits when the fully
switched generation advertises a new daemon identity. Systemd then starts the
new executable with its new package source. Reopen the UI to use the new client.

The first upgrade from an older daemon needs one manual restart **after any
active operation has finished**, because old code cannot perform this handoff:

```console
sudo systemctl restart peasy-system
```

On any installation, including headless systems, `peasy --status` reports system
status without an AI provider or Wasm engine. After an interrupted operation,
`peasy --recover` shows the previous-generation recovery proposal and asks for
confirmation before requesting the usual administrator authorization. Recovery
changes the whole system generation; it does not undo database transactions or
other service side effects.

The UI's **System status and recovery** screen reports the running executable,
version, protocol and package source. A configuration written before an interrupted
build is restored on daemon startup when Peasy can establish that it still owns
the unfinished change. If activation may have started, the screen shows the
intended change and active/previous generation, and offers a freshly reviewed,
authorized restoration of the previous whole system generation. Service side
effects may need separate attention. If the previous generation is unavailable,
select a known-good generation in the boot menu and inspect the configuration.

## Package search

For a named application, the model may suggest an exact Nixpkgs attribute.
Peasy verifies it against the effective host package set, including overlays,
before creating a proposal. A missing or unavailable attribute falls back to
search. Uncertain names, alternatives and requested versions use search first.

Search results are cached in memory for four hours; empty results expire after
one minute. Active-generation, managed-module, host entry-file and flake-lock
changes invalidate the cache. Edits to other imported files may remain unseen
until expiry. Explicit availability checks and version-specific searches bypass
the cache. Creating a proposal always resolves its packages again and records
exact derivation paths; apply rejects a changed definition before activation
and asks for another review.

## Configuration backup and restore

Settings → **Backups and updates → Export backup** creates a portable backup of active Peasy-managed
standalone packages, appearance preferences and pinned pea instructions. Both
traditional and flake hosts are supported. Restore it into an existing NixOS
installation while keeping the destination's disks, bootloader, drivers, user
accounts and host configuration.

Service setups, network profiles and AppImages are saved for review and recreation
on the destination; they are not applied unchanged. Original host files, including
flake files and hardware modules, are archived separately when readable and are
never imported by the portable restore module. The backup reports unavailable
archives and excluded files.

Use Settings → **Backups and updates → Restore backup**, select the exported folder, choose Merge or
Replace, then review and authenticate before applying. No AI provider is required.
The restore uses the destination's package set
and flake lock; it is not a locked package closure or a backup of personal files
and databases. See [backup contents and restore steps](backups.md).

## Updates

After installation, open **Settings → Backups and updates** to check for a newer stable release and use
**Update Peasy**. Updates preserve the host configuration and flake lock. See
[updating Peasy](updates.md) for details and the one-time setup for older installs.
