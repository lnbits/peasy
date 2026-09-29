mod activation;
mod authorization;
mod nix_backend;
mod pea_fetch;
mod postgresql_probe;
mod process;
mod recovery;
#[path = "../../../peapod/tests/resource_fixture.rs"]
mod resource_fixture;
mod resource_helper;
mod server;
mod state;
mod update_check;

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
    #[arg(long)]
    identity: Option<PathBuf>,
    #[arg(long, default_value = "/run/current-system")]
    active_system: PathBuf,
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
    #[arg(long, default_value = "/etc/peasy/pea-policy.json")]
    pea_policy: PathBuf,
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
    resource_helper: bool,
    #[arg(long, hide = true)]
    fetch_pea: bool,
    #[arg(long, hide = true)]
    check_releases: bool,
    #[arg(long, hide = true)]
    inspect_postgresql: bool,
    #[arg(long, hide = true)]
    render_test_theme: bool,
    #[arg(long, hide = true)]
    render_test_appimage: bool,
    #[arg(long, hide = true)]
    render_test_setup: bool,
    #[arg(long, hide = true)]
    render_test_resources: bool,
    #[arg(long, hide = true)]
    render_test_update: bool,
    #[arg(long, hide = true)]
    render_test_capabilities: bool,
    #[arg(long, hide = true)]
    render_test_postgresql: bool,
    #[arg(long, hide = true)]
    render_test_network: bool,
    #[arg(long, hide = true)]
    activate: bool,
    #[arg(long, hide = true)]
    check_activation: bool,
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
    if args.render_test_resources {
        print!("{}", resource_fixture::render());
        return Ok(());
    }
    if args.resource_helper {
        return resource_helper::run(&args.runtime_dir);
    }
    if args.check_activation {
        return activation::check_guard(&args.runtime_dir);
    }
    if args.inspect_postgresql {
        return postgresql_probe::run(&args.runtime_dir);
    }
    if args.check_releases {
        return update_check::run();
    }
    if args.fetch_pea {
        return pea_fetch::run();
    }
    if args.self_test_sandbox {
        return sandbox_self_test();
    }
    if args.render_test_update {
        let setup: peasy_core::ManagedSetup = serde_json::from_str(include_str!(
            "../../../peapod/system_configuration/example.json"
        ))?;
        let release = peasy_core::PeasyRelease {
            format: 1,
            version: env!("CARGO_PKG_VERSION").into(),
            tag: format!("v{}", env!("CARGO_PKG_VERSION")),
            revision: "a".repeat(40),
            sha256: "b".repeat(64),
        };
        let state = peasy_core::PackageState::default()
            .with_setup(setup)?
            .with_peasy_release(&release)?;
        print!("{}", peasy_core::render_packages_module(&state)?);
        return Ok(());
    }
    if args.render_test_network {
        let plan: peasy_core::NetworkPlan =
            serde_json::from_str(include_str!("../../../peapod/networking/example.json"))?;
        print!(
            "{}",
            peasy_core::render_packages_module(
                &peasy_core::PackageState::default().with_network(&plan)?
            )?
        );
        return Ok(());
    }
    if args.render_test_capabilities {
        let mut cases = Vec::new();
        for (option, _) in peasy_core::SYSTEM_ENABLE_OPTIONS {
            let mut enable = vec![option.to_string()];
            if *option == "programs.appimage.binfmt" {
                enable.push("programs.appimage.enable".into());
            }
            let groups: Vec<String> = peasy_core::SYSTEM_GROUPS
                .iter()
                .filter(|(_, required)| required == option)
                .map(|(group, _)| group.to_string())
                .collect();
            let setup = peasy_core::ManagedSetup {
                package: "hello".into(),
                user: (!groups.is_empty()).then(|| "peasytest".into()),
                uid: (!groups.is_empty()).then_some(1000),
                settings: peasy_core::SystemSetup {
                    packages: vec![],
                    enable,
                    groups: groups.clone(),
                    postgresql: None,
                },
            };
            let state = peasy_core::PackageState::default()
                .with_setup(setup)?
                .with_theme(&peasy_core::ThemeSettings {
                    accent_color: Some(peasy_core::AccentColor::Blue),
                    color_scheme: None,
                })?;
            cases.push(serde_json::json!({ "option": option, "groups": groups, "module": peasy_core::render_packages_module(&state)? }));
        }
        let groups: Vec<String> = peasy_core::SYSTEM_GROUPS
            .iter()
            .filter(|(_, required)| required.is_empty())
            .map(|(group, _)| group.to_string())
            .collect();
        let setup = peasy_core::ManagedSetup {
            package: "hello".into(),
            user: Some("peasytest".into()),
            uid: Some(1000),
            settings: peasy_core::SystemSetup {
                packages: vec![],
                enable: vec![],
                groups: groups.clone(),
                postgresql: None,
            },
        };
        cases.push(serde_json::json!({ "option": null, "groups": groups, "module": peasy_core::render_packages_module(&peasy_core::PackageState::default().with_setup(setup)?)? }));
        println!("[");
        for case in cases {
            let groups = case["groups"]
                .as_array()
                .expect("test groups")
                .iter()
                .map(|g| g.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            println!(
                "{{ option = {}; groups = [ {} ]; module = (\n{}\n); }}",
                case["option"],
                groups,
                case["module"].as_str().expect("test module")
            );
        }
        println!("]");
        return Ok(());
    }
    if args.render_test_setup {
        let setup: peasy_core::ManagedSetup = serde_json::from_str(include_str!(
            "../../../peapod/system_configuration/example.json"
        ))?;
        let state = peasy_core::PackageState::default().with_setup(setup)?;
        print!("{}", peasy_core::render_packages_module(&state)?);
        return Ok(());
    }
    if args.render_test_postgresql {
        let setup: peasy_core::ManagedSetup = serde_json::from_str(include_str!(
            "../../../peapod/system_configuration/postgresql-example.json"
        ))?;
        let state = peasy_core::PackageState::default().with_setup(setup)?;
        print!("{}", peasy_core::render_packages_module(&state)?);
        return Ok(());
    }
    if args.render_test_theme {
        let state = peasy_core::PackageState {
            resources: peasy_core::ResourceState::default(),
            peasy_release: None,
            packages: vec!["hello".into()],
            setups: Vec::new(),
            networks: Vec::new(),
            peas: Vec::new(),
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
            resources: peasy_core::ResourceState::default(),
            peasy_release: None,
            packages: Vec::new(),
            setups: Vec::new(),
            networks: Vec::new(),
            peas: Vec::new(),
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
    let _ = args.nixos_rebuild; // Accept the previous module's CLI during upgrades.
    let rebuild_target = match (args.host_configuration, args.host_flake) {
        (Some(path), None) => RebuildTarget::Configuration { path },
        (None, Some(reference)) => RebuildTarget::Flake { reference },
        _ => anyhow::bail!("exactly one of --host-configuration or --host-flake is required"),
    };
    let config = BackendConfig {
        identity: args.identity,
        active_system: args.active_system,
        appimage_policy: args.appimage_policy,
        pea_policy: args.pea_policy,
        pea_fetch_output: "/run/peasy-pea-fetch/pea.json".into(),
        network_profiles_dir: "/etc/NetworkManager/system-connections".into(),
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
