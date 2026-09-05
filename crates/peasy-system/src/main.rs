mod activation;
mod authorization;
mod nix_backend;
mod process;
mod server;
mod state;

use anyhow::{Context, Result};
use clap::Parser;
use nix_backend::{BackendConfig, NixBackend, ProcessRunner, RebuildTarget};
use server::Server;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Parser)]
#[command(
    name = "peasy-system",
    about = "Peasy's typed NixOS configuration service"
)]
struct Args {
    #[arg(long, default_value = "/run/peasy/peasy.sock")]
    socket: PathBuf,
    #[arg(long, default_value = "/run/peasy")]
    runtime_dir: PathBuf,
    #[arg(long)]
    nix: Option<PathBuf>,
    #[arg(long)]
    nixos_rebuild: Option<PathBuf>,
    #[arg(long)]
    nix_env: Option<PathBuf>,
    #[arg(long)]
    systemctl: Option<PathBuf>,
    #[arg(long)]
    pkcheck: Option<PathBuf>,
    #[arg(long, default_value = "/etc/peasy/appimage-policy.json")]
    appimage_policy: PathBuf,
    #[arg(long)]
    nixpkgs: Option<PathBuf>,
    #[arg(long)]
    managed_module: Option<PathBuf>,
    #[arg(long)]
    system: Option<String>,
    #[arg(long)]
    host_configuration: Option<PathBuf>,
    #[arg(long)]
    host_flake: Option<String>,
    #[arg(long, hide = true)]
    self_test_sandbox: bool,
    #[arg(long, hide = true)]
    render_test_theme: bool,
    #[arg(long, hide = true)]
    render_test_appimage: bool,
    #[arg(long, hide = true)]
    activate: bool,
    #[arg(long, hide = true)]
    reconcile_managed_state: Option<PathBuf>,
}

fn sandbox_self_test() -> Result<()> {
    let home_denied = std::fs::read_to_string("/home/testuser/private.txt").is_err();
    let etc_denied = std::fs::write("/etc/peasy-security-test", "must not be written").is_err();
    let proc_root_denied =
        std::fs::read_to_string("/proc/1/root/home/testuser/private.txt").is_err();
    if home_denied && etc_denied && proc_root_denied {
        println!("home-read=denied etc-write=denied");
        Ok(())
    } else {
        anyhow::bail!(
            "sandbox failure: home-read-denied={home_denied} etc-write-denied={etc_denied}"
        )
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.self_test_sandbox {
        return sandbox_self_test();
    }
    if args.render_test_theme {
        let state = peasy_core::PackageState {
            packages: vec!["hello".into()],
            appimages: Vec::new(),
            theme: peasy_core::ThemeSettings {
                accent_color: Some(peasy_core::AccentColor::Blue),
                color_scheme: Some(peasy_core::ColorScheme::Dark),
            },
        };
        print!("{}", peasy_core::render_packages_module(&state)?);
        return Ok(());
    }
    if args.render_test_appimage {
        let state = peasy_core::PackageState {
            packages: Vec::new(),
            appimages: vec![peasy_core::AppImagePackage {
                id: "appimage.example.nostr-chat".into(),
                display_name: "Nostr ${builtins.toString 7} Chat".into(),
                repository: "example/nostr-chat".into(),
                version: "1.2".into(),
                release_tag: "v1.2".into(),
                asset_name: "nostr-chat-x86_64.AppImage".into(),
                url: "https://github.com/example/nostr-chat/releases/download/v1.2/nostr-chat-x86_64.AppImage".into(),
                hash: "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
                architecture: peasy_core::AppImageArchitecture::X86_64,
                size: 42_000_000,
            }],
            theme: peasy_core::ThemeSettings::default(),
        };
        print!("{}", peasy_core::render_packages_module(&state)?);
        return Ok(());
    }
    if let Some(active_state) = args.reconcile_managed_state {
        return state::restore_managed_from_generation(
            &active_state,
            args.managed_module
                .as_deref()
                .context("--managed-module is required")?,
        );
    }
    if args.activate {
        return activation::run_helper(
            &args.runtime_dir,
            args.nix_env.as_deref().context("--nix-env is required")?,
        );
    }
    let rebuild_target = match (args.host_configuration, args.host_flake) {
        (Some(path), None) => RebuildTarget::Configuration { path },
        (None, Some(reference)) => RebuildTarget::Flake {
            reference,
            nixos_rebuild: args
                .nixos_rebuild
                .context("--nixos-rebuild is required with --host-flake")?,
        },
        _ => anyhow::bail!("exactly one of --host-configuration or --host-flake is required"),
    };
    let config = BackendConfig {
        appimage_policy: args.appimage_policy,
        runtime_dir: args.runtime_dir,
        nix: args.nix.context("--nix is required")?,
        systemctl: args.systemctl.context("--systemctl is required")?,
        nixpkgs: args.nixpkgs.context("--nixpkgs is required")?,
        system: args.system.context("--system is required")?,
        managed_module: args
            .managed_module
            .context("--managed-module is required")?,
        rebuild_target,
    };
    let backend = Arc::new(NixBackend::new(config, Arc::new(ProcessRunner))?);
    let authorizer = Arc::new(authorization::PolkitAuthorizer(
        args.pkcheck.context("--pkcheck is required")?,
    ));
    Server::new(args.socket, backend, authorizer)
        .context("starting Peasy system service")?
        .run()
}
