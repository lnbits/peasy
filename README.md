# Peasy

## Tell your computer what you want.

Peasy is a simple way to manage your computer using natural language.

Want Telegram? Tell Peasy to install it.

Want Telegram gone? Tell Peasy to remove it.

Want to change a system setting? Just describe what you want.

Peasy translates those instructions into safe, understandable changes to your NixOS configuration.

---

## Why NixOS?

Projects like Omarchy have the right idea: make Linux easier to use by letting people describe what they want instead of manually configuring everything.

But building that idea on Arch means adding an abstraction layer on top of an operating system that was never designed to work this way.

NixOS already is.

NixOS is already declarative. Your packages, services, desktop configuration and much of your system state can be described as code.

That makes it unusually well suited to an AI agent.

Instead of giving an agent unrestricted access to a terminal and hoping it makes the right changes, Peasy can work primarily with the Nix configuration that defines the machine.

The agent changes the configuration. NixOS builds the result.

And because NixOS generations are reproducible and reversible, changes don't have to be permanent mistakes.

---

### Arch

> “I installed and changed a bunch of things over the last 18 months.
> I think this is how my computer is configured.”

### NixOS

> “This file is my computer.”

That difference is what makes Peasy possible.

Peasy isn't trying to bolt AI onto Linux.

It's giving AI an operating system it can actually understand.

---

## What Peasy does

Peasy is a small, free-software natural-language interface for NixOS, GNOME,
and Hyprland.
It provides matching compact launchers for GNOME and Hyprland, a
GTK4/libadwaita request-and-review application, and the `peasy` terminal
command.

Peasy supports a closed set of typed capabilities:

- search, install, remove, and check the availability of real Nixpkgs packages;
- resolve explicitly named GitHub organisations/repositories directly (including
  repository renames), or discover likely upstream AppImage releases when no
  source was named, then install a selected version as a pinned Nix package;
- list and set GNOME accent colours and light/dark/system appearance;
- inspect a running Hyprland session and control common appearance, layout,
  workspace, focus, floating, and fullscreen actions through typed `hyprctl`;
- list and connect to nearby Wi-Fi networks;
- discover and connect Bluetooth devices; and
- prepare a calendar event and open it in the default calendar for final import.

The selected OpenAI or local Ollama model interprets language but receives no tools. NixOS changes show the exact
generated-Nix diff, require confirmation, build a complete proposed generation,
and activate only after a successful build. Live desktop actions show their own
typed preview and require confirmation. Peasy does not accept arbitrary Nix,
commands, paths, or general machine-control requests from the model.

## Build

```console
nix build
```

For development:

```console
nix develop
cargo test --workspace --exclude peasy-engine
cargo build -p peasy-engine --target wasm32-unknown-unknown
```

The flake exposes:

- `packages.<system>.default` and `packages.<system>.peasy` for the complete
  desktop package
- `packages.<system>.peasy-core` for the CLI, policy engine, and system service
  without GTK, libadwaita, tray binaries, desktop files, or GNOME assets
- `nixosModules.default`
- `devShells.<system>.default`
- unit, Wasm-import, system-sandbox, and GNOME-panel checks

## Install from configuration.nix

Peasy does not require the host system to use flakes. From a local checkout,
add the module to the existing `/etc/nixos/configuration.nix`:

```nix
{ lib, ... }:
{
  imports = [
    ./hardware-configuration.nix
    /home/alice/src/peasy/nix/module.nix
  ] ++ lib.optional
    (builtins.pathExists ./.peasy/peasy-managed.nix)
    ./.peasy/peasy-managed.nix;

  services.peasy = {
    enable = true;

    # Required only for a development checkout below ProtectHome.
    configurationReadPaths = [ "/home/alice/src/peasy" ];
  };
}
```

Then use the normal channel-based rebuild command:

```console
sudo nixos-rebuild switch --no-flake
sudo systemctl restart peasy-system
```

Log out of GNOME and back in after installing or upgrading the extension. Peasy
deliberately does not restart its daemon during a Peasy-initiated system switch,
because doing so would sever the apply result; the explicit restart above loads
a newly installed Peasy daemon after an administrator-initiated upgrade.

For a published release, use a pinned store-backed source instead of a home
checkout; no `configurationReadPaths` exception is then needed:

```nix
{ lib, ... }:
let
  peasy = builtins.fetchTarball {
    url = "https://github.com/OWNER/peasy/archive/refs/tags/v0.1.0.tar.gz";
    sha256 = "sha256-REPLACE-WITH-RELEASE-HASH";
  };
in
{
  imports = [ (peasy + "/nix/module.nix") ] ++ lib.optional
    (builtins.pathExists ./.peasy/peasy-managed.nix)
    ./.peasy/peasy-managed.nix;
  services.peasy.enable = true;
}
```

## Optional flake-host integration

The Peasy project remains a flake and exports `nixosModules.default`. A host
whose complete module graph is assembled in `flake.nix` should select the
optional flake rebuild mode:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    peasy.url = "github:peasy-nixos/peasy";
  };

  outputs =
    { nixpkgs, peasy, ... }:
    {
      nixosConfigurations.my-host = nixpkgs.lib.nixosSystem {
        system = "x86_64-linux";
        modules = [
          peasy.nixosModules.default
          {
            services.peasy = {
              enable = true;
              hostFlake = "/etc/nixos#my-host";
            };
          }
          ./configuration.nix
        ];
      };
    };
}
```

Apply it:

```console
sudo nixos-rebuild switch --flake .#my-host
```

`hostFlake` is optional and defaults to `null`. Without it, Peasy evaluates
`services.peasy.hostConfiguration`, which defaults to
`/etc/nixos/configuration.nix`. The flake's `configuration.nix` must include the
same optional `.peasy/peasy-managed.nix` import shown above. Peasy forces a
local `path:` flake build so the newly created managed file is included even
before it is committed to Git.

The module installs the CLI, GTK app, desktop file, native Peasy GNOME Shell
launcher, and Hyprland StatusNotifier launcher; starts `peasy-system`; and
enables NetworkManager and Bluetooth by default. No extension download or
manual desktop-file copy is required.

## Minimal and headless NixOS

On a server, minimal installation, or any machine without a graphical session,
disable Peasy's desktop integration:

```nix
{ lib, ... }:
{
  imports = [ /path/to/peasy/nix/module.nix ] ++ lib.optional
    (builtins.pathExists ./.peasy/peasy-managed.nix)
    ./.peasy/peasy-managed.nix;

  services.peasy = {
    enable = true;
    desktop.enable = false;

    # Optional: run the language model locally on this machine.
    ollama.enable = true;
  };
}
```

This automatically selects the `peasy-core` derivation. It contains only the
`peasy` CLI, `peasy-system`, and the zero-import Wasm policy engine. It does not
enable NetworkManager or Bluetooth, create GNOME user units, install the panel
extension, or bring GTK/libadwaita into its runtime closure. The CLI still
supports OpenAI and local Ollama and retains the same proposal, diff, build,
confirmation, activation, and rollback boundaries. Desktop-only live actions
require their corresponding tools and session and are not enabled by the
headless module profile.

Build just this package directly with:

```console
nix build .#peasy-core
```

Because Peasy changes the system generation, access to its local IPC socket is
limited to members of the NixOS `wheel` administrator group.

## AI provider: OpenAI or local Ollama

On first use Peasy presents provider setup. At any time, open Peasy from the
mint panel/tray circle and select the settings cog in the application. There you
can switch providers, replace or remove the OpenAI key, and select an Ollama
model that Peasy has verified is installed. Terminal setup is also available:

```console
peasy --setup-provider
peasy --setup-key
```

The settings view also provides **Export system** beside a short restore
description. It creates a private `peasy-system-config` folder containing the
complete host configuration directory, Peasy's source, a deterministic
`.peasy/peasy-managed.nix` with every package/AppImage/appearance change made
through Peasy, and a top-level module plus exact restore instructions. Provider
credentials and API keys are never included. The host files may contain
sensitive administrator-authored values, so review the folder before sharing;
replace the bundled hardware configuration when restoring to different
hardware.

`--setup-key` is a compatibility shortcut that replaces the key and selects
OpenAI. The key is stored only for that desktop user in
`$XDG_CONFIG_HOME/peasy/openai-key` (normally
`~/.config/peasy/openai-key`) with mode `0600`. It is not stored in Nix, logged,
or sent to `peasy-system`. Non-secret provider/model selection is stored in a
separate mode-`0600` `provider.json` file.

Peasy uses OpenAI's Responses API with `store: false`, no model tools, and a
strict JSON Schema response format, following the current
[official OpenAI Responses API documentation](https://developers.openai.com/api/reference/cli/resources/responses/methods/create).

For a fully local model, enable Ollama in the same Peasy module configuration:

```nix
services.peasy = {
  enable = true;
  ollama.enable = true;
};
```

After rebuilding, install a model once with `ollama pull MODEL`, open Peasy's
settings cog, choose **Ollama (local)**, and select the detected model. Peasy
connects only to `http://127.0.0.1:11434`; it does not expose Ollama through the
firewall or accept model-controlled URLs. It uses Ollama's native `POST
/api/chat` interface with `stream: false`, deterministic options, and the same
closed JSON Schema in `format`, as specified by the official
[Ollama chat](https://docs.ollama.com/api/chat),
[structured outputs](https://docs.ollama.com/capabilities/structured-outputs),
and [model listing](https://docs.ollama.com/api/tags) documentation.

## Terminal use

Interactive mode:

```console
$ peasy
Peasy
Tell your computer what you want.

Peasy › install telegram
```

Direct requests:

```console
peasy "install telegram"
peasy "install vlc"
peasy "remove telegram"
peasy "change to a blue dark theme"
peasy "what wifis are available?"
peasy "connect to wifi CoolCafe"
peasy "connect to my Beats headphones"
peasy "set a meeting for 27 September at 10am for Walk with Dad"
peasy "can I install OBS on this machine?"
```

Every changing request displays a typed preview and asks for confirmation.
Package and theme proposals display red/green lines from the exact Peasy-owned
Nix module that will be built.

Confirmed GNOME appearance changes are stored in the NixOS generation and also
applied immediately to the invoking user's live session. A fixed unprivileged
user service synchronizes that validated state on login and when switching
between Peasy-generated generations. The first upgrade from an older Peasy
release that locked these dconf keys requires one final logout/login.

## GNOME panel

After login, the Peasy GNOME Shell extension places a compact mint circle in
the top-right panel. Selecting it opens the Peasy application, where requests,
progress, package choices, reviewable diffs, confirmation, results, and provider
settings share one consistent interface.

The extension is shipped inside the Nix package and enabled both declaratively
and by a fixed XDG login action for existing GNOME profiles.

## Hyprland

On Hyprland, Peasy starts its StatusNotifier launcher automatically. A bar with
a tray host, such as Waybar with its `tray` module enabled, displays the same
mint-circle Peasy icon; selecting it opens the same request-and-review UI. The
CLI also works directly.

Peasy detects whether the running compositor exposes Hyprland's legacy
hyprlang IPC or the current Lua IPC and uses that session's own `hyprctl`.
Supported requests include inspecting the active workspace/window/monitors,
changing gaps, borders, rounding, animations, blur, opacity, natural scrolling,
and layout, switching workspaces, moving the active window, changing focus, and
toggling floating or fullscreen.

These are live compositor actions and the review says so explicitly. They do
not edit `hyprland.conf` or `hyprland.lua`; reloading Hyprland restores the
user's declarative configuration. Peasy never exposes raw `hyprctl`, Lua,
`exec`, plugin, kill, or shutdown actions to the model.

## What an apply does

1. The selected OpenAI or local Ollama model returns a strictly validated
   intent; it has no tools.
2. `peasy-system` runs a real `nix search --json` when package data is needed,
   filters results through Nixpkgs' host-platform availability metadata, ranks
   desktop applications ahead of development/support packages, and keeps a
   small time-bounded in-memory search cache.
3. Trusted code verifies selected package attributes with `nix eval`. Choosing
   an exact displayed result does not require another model request.
   If the user chooses external discovery, the unprivileged client searches
   stable GitHub releases, filters for the host architecture, and presents the
   repository and asset candidates. The selected asset is hashed into the Nix
   store before a second, detailed review; it is never executed during search.
4. The daemon computes the exact before/after module and returns a red/green
   diff bound to a short-lived proposal token.
5. After confirmation, it atomically updates the imported
   `.peasy/peasy-managed.nix`. This file is Peasy's only durable state.
6. It builds the normal host `configuration.nix` or flake and verifies that the
   resulting generation contains the exact reviewed Peasy state. A failed build
   restores the previous managed module.
7. Only a successful NixOS result inside `/nix/store` is handed to the fixed,
   root-only activation helper, installed as the system generation, and
   switched.

Peasy never rewrites `configuration.nix`, hardware configuration, or
`flake.lock`; it owns only the explicitly imported managed module. Normal manual
rebuilds therefore include Peasy's changes, while ordinary NixOS generations
and boot rollbacks work without a parallel state database. See
[architecture](docs/architecture.md).

## Security

The model receives an explicitly constructed boundary: the redacted request,
Peasy-managed packages/theme state, current local time, relevant validated
package candidates, and a bounded system profile generated from the active
evaluated NixOS configuration. That profile contains release/platform tokens,
closed desktop/Peasy-variant values, and installed system package names. It does
not contain `configuration.nix`, imported source, or arbitrary option values.
Nearby Wi-Fi scans, Bluetooth addresses, files, environment, credentials, and
arbitrary configuration are excluded. A Wi-Fi password typed in a sentence is
removed before the model request is constructed and later sent to NetworkManager
through stdin rather than a process argument.

The Wasmtime engine imports no host capabilities. Root IPC has no command or path
method. The daemon is denied home directories and IP networking, while the Nix
daemon may fetch store paths. System proposal tokens are random, one-use,
short-lived, peer-UID-bound, and rejected if the reviewed base state changed.

The full threat model and hardening rationale are in
[docs/security.md](docs/security.md).

## Tests

Fast tests:

```console
cargo test --workspace --exclude peasy-engine
```

All flake checks, including Nix builds and the zero-import assertion:

```console
nix flake check
```

The system sandbox VM runs `peasy-core` with `desktop.enable = false`, proving
the backend service and real NixOS evaluation path work without a graphical
environment. A separate closure check rejects GTK, libadwaita, or GNOME Shell
dependencies in the core output.

Run the expensive VM checks individually:

```console
nix build .#checks.x86_64-linux.sandbox
nix build .#checks.x86_64-linux.gnome-tray
```

The sandbox VM starts the real IPC service, checks typed IPC, builds a non-flake
NixOS expression containing the generated dconf shape, and exercises production
filesystem restrictions against a home secret and unrelated `/etc` writes. The
graphical VM starts GNOME, proves the Peasy extension executed, verifies its
panel files are installed, launches the GTK review app, and captures the panel.

## License

MIT
