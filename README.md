<h1><a href="https://askpeasy.com"><img src="assets/peasy-wordmark.svg" alt="Peasy" width="210"></a></h1>

Tell your NixOS computer what you want. Install apps, change settings, ask
questions and get help using OpenAI or local Ollama.

<img src="assets/peasy-demo.gif" alt="Peasy reviewing example requests" width="600">

**Beta.** System changes require review and administrator authentication.
Peasy preserves your host configuration; the model cannot run arbitrary commands.

## Get started

- **New machine:** download the [installer ISO](https://github.com/lnbits/peasy/releases/latest).
  It includes GNOME, Ollama and Qwen3 0.6B. Follow the [verification and install steps](docs/iso.md).
- **Existing NixOS:** follow the [installation guide](docs/install.md).
- Open Peasy from the application menu or tray. Choose your provider in Settings.

Try “install Telegram”, “change to a blue dark theme”, or “What is NixOS?”

[Chat](docs/chat.md) · [Supported desktops](docs/desktop-compatibility.md) ·
[Backups](docs/backups.md) · [Updates](docs/updates.md)

## Development

```sh
nix build
./result/bin/peasy-ui
nix develop --command bash scripts/check-rust.sh
```

[Architecture](docs/architecture.md) · [Add a pea](peapod/README.md) ·
[Security](docs/security.md) · [Release checks](docs/release-validation.md)

[MIT license](LICENSE.md).
