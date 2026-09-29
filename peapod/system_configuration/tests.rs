use super::*;
use crate::{
    ModelAction, ModelEnvelope, PackageOperation, ThemeSettings, parse_packages_module,
    render_packages_module,
};

fn example() -> ManagedSetup {
    serde_json::from_str(include_str!("example.json")).unwrap()
}

fn postgres() -> ManagedSetup {
    serde_json::from_str(include_str!("postgresql-example.json")).unwrap()
}

#[test]
fn postgres_has_bounded_versions_and_caller_bound_access() {
    let mut setup = postgres();
    setup.normalize().unwrap();
    let mut mismatch = setup.clone();
    mismatch.settings.postgresql.as_mut().unwrap().package = "postgresql_18".into();
    assert!(mismatch.normalize().is_err());
    for package in [
        "postgresql",
        "hello",
        "postgresql_17; abort",
        "postgresql_999",
    ] {
        let mut invalid = setup.clone();
        invalid.settings.postgresql.as_mut().unwrap().package = package.into();
        assert!(invalid.normalize().is_err());
    }
    for user in [
        "root",
        "postgres",
        "pg_read_all_data",
        "user-name",
        "x'; DROP ROLE postgres;--",
    ] {
        let mut invalid = setup.clone();
        invalid.user = Some(user.into());
        assert!(invalid.normalize().is_err(), "{user}");
    }
    setup.user = None;
    assert!(setup.normalize().is_err());
    setup.uid = None;
    setup.settings.postgresql.as_mut().unwrap().caller_database = false;
    setup.normalize().unwrap();
    let invalid = serde_json::json!({"package":"postgresql_17", "caller_database":true,"authentication":"trust"});
    assert!(serde_json::from_value::<PostgresqlSetup>(invalid).is_err());
}

#[test]
fn postgres_shared_service_roundtrips_and_withdraws_without_data_deletion() {
    let first = postgres();
    let original = PackageState::default()
        .with_change(PackageOperation::Install, "postgresql_17")
        .unwrap();
    let state = original.with_setup(first.clone()).unwrap();
    assert!(state.packages.is_empty());
    let text = render_packages_module(&state).unwrap();
    assert_eq!(parse_packages_module(&text).unwrap(), state);
    assert!(text.contains("services.postgresql.package = pkgs.postgresql_17;"));
    assert!(!text.contains("      \"postgresql_17\""));
    let mut second = first.clone();
    second.package = "hello".into();
    let shared = state.with_setup(second).unwrap();
    let text = render_packages_module(&shared).unwrap();
    assert_eq!(text.matches("services.postgresql.ensureUsers =").count(), 1);
    let removed = shared.without_setup("postgresql_17").unwrap();
    assert!(
        render_packages_module(&removed)
            .unwrap()
            .contains("services.postgresql.enable = true")
    );
    let removed = removed.without_setup("hello").unwrap();
    let text = render_packages_module(&removed).unwrap();
    assert!(!text.contains("postgresql"));
    assert_eq!(removed, PackageState::default());
    let mut changed = first;
    changed.settings.postgresql.as_mut().unwrap().package = "postgresql_18".into();
    assert!(state.with_setup(changed).is_err());
}

#[test]
fn setup_is_data_not_arbitrary_nix_or_accounts() {
    let valid = example();
    for key in [
        "system.activationScripts.evil.text",
        "security.sudo.enable",
        "services.openssh.enable",
        "virtualisation.libvirtd.enable; abort",
        "${builtins.readFile /etc/shadow}",
    ] {
        let mut setup = valid.clone();
        setup.settings.enable = vec![key.into()];
        assert!(setup.normalize().is_err(), "{key}");
    }
    for group in ["wheel", "docker", "root", "libvirtd\n"] {
        let mut setup = valid.clone();
        setup.settings.groups = vec![group.into()];
        assert!(setup.normalize().is_err());
    }
    let mut setup = valid.clone();
    setup.settings.enable.clear();
    assert!(setup.normalize().is_err());
    setup = valid.clone();
    setup.settings.packages = vec!["hello;reboot".into()];
    assert!(setup.normalize().is_err());
    setup = valid.clone();
    setup.settings.packages = vec!["hello".into(); 9];
    assert!(setup.normalize().is_err());
    for user in ["root", "", "ben\";", "${abort}"] {
        setup = valid.clone();
        setup.user = Some(user.into());
        assert!(setup.normalize().is_err());
    }
    setup = valid;
    setup.uid = Some(0);
    assert!(setup.normalize().is_err());
    assert!(
        serde_json::from_value::<SystemSetup>(serde_json::json!({
            "packages": [], "enable": ["services.printing.enable"], "groups": [], "user": "root"
        }))
        .is_err()
    );
}

#[test]
fn setup_envelope_preserves_the_closed_boundary() {
    let plan = example().settings;
    let envelope: ModelEnvelope = serde_json::from_value(serde_json::json!({
        "action": "install_package", "package": "virt-manager", "setup": plan
    }))
    .unwrap();
    assert!(matches!(
        ModelAction::try_from(envelope).unwrap(),
        ModelAction::InstallPackage { setup: Some(_), .. }
    ));
    let envelope: ModelEnvelope = serde_json::from_value(serde_json::json!({
        "action": "remove_package", "package": "virt-manager", "setup": plan
    }))
    .unwrap();
    assert!(ModelAction::try_from(envelope).is_err());
}

#[test]
fn shared_contributions_and_other_peas_survive_setup_removal() {
    let first = example();
    let mut second = first.clone();
    second.package = "virt-viewer".into();
    second.settings.packages = vec!["hello".into()];
    let original = PackageState::default()
        .with_change(PackageOperation::Install, "hello")
        .unwrap();
    let state = original
        .with_setup(first.clone())
        .unwrap()
        .with_setup(second)
        .unwrap();
    let state = state
        .with_theme(&ThemeSettings {
            accent_color: Some(crate::AccentColor::Green),
            color_scheme: None,
        })
        .unwrap();
    assert_eq!(state.setups.len(), 2);
    let text = render_packages_module(&state).unwrap();
    assert_eq!(
        text.matches("  virtualisation.libvirtd.enable = true;")
            .count(),
        1
    );
    assert_eq!(
        text.matches("  users.users.\"peasytest\".extraGroups")
            .count(),
        1
    );
    assert_eq!(parse_packages_module(&text).unwrap(), state);
    assert!(parse_packages_module(&(text + "\nservices.openssh.enable = true;\n")).is_err());
    let removed = state.without_setup("virt-manager").unwrap();
    assert_eq!(removed.setup_dependents("hello"), ["virt-viewer"]);
    assert!(
        removed
            .with_change(PackageOperation::Remove, "hello")
            .unwrap()
            .effective_packages()
            .contains("hello")
    );
    assert!(
        render_packages_module(&removed)
            .unwrap()
            .contains("  virtualisation.libvirtd.enable = true;")
    );
    assert!(!removed.effective_packages().contains("virt-manager"));
    let removed = removed.without_setup("virt-viewer").unwrap();
    assert_eq!(removed.packages, ["hello"]);
    assert!(
        !render_packages_module(&removed)
            .unwrap()
            .contains("extraGroups")
    );
    assert_eq!(removed.theme, state.theme);
    assert_eq!(
        original
            .with_setup(first)
            .unwrap()
            .without_setup("virt-manager")
            .unwrap(),
        original
    );
}

#[test]
fn old_state_remains_byte_compatible_and_new_state_is_canonical() {
    let old = r#"{"packages":["hello"],"appimages":[],"theme":{"accent_color":null,"color_scheme":null}}"#;
    let state: PackageState = serde_json::from_str(old).unwrap();
    assert_eq!(serde_json::to_string(&state).unwrap(), old);
    assert!(!render_packages_module(&state).unwrap().contains("setups"));
    let configured = state.with_setup(example()).unwrap();
    let mut tampered = render_packages_module(&configured).unwrap();
    tampered = tampered.replace(
        "  virtualisation.libvirtd.enable = true;",
        "  services.openssh.enable = true;",
    );
    assert!(parse_packages_module(&tampered).is_err());
}

#[test]
fn every_capability_roundtrips_shares_and_withdraws() {
    for (option, description) in SYSTEM_ENABLE_OPTIONS {
        assert!(!description.is_empty());
        let mut setup = example();
        setup.settings.enable = vec![option.to_string()];
        if *option == "programs.appimage.binfmt" {
            setup
                .settings
                .enable
                .push("programs.appimage.enable".into());
        }
        setup.settings.groups = SYSTEM_GROUPS
            .iter()
            .filter(|(_, required)| required == option)
            .map(|(group, _)| group.to_string())
            .collect();
        if setup.settings.groups.is_empty() {
            setup.user = None;
            setup.uid = None;
        }
        let mut shared = setup.clone();
        shared.package = "hello".into();
        let state = PackageState::default()
            .with_setup(setup)
            .unwrap()
            .with_setup(shared)
            .unwrap()
            .with_theme(&ThemeSettings {
                accent_color: Some(crate::AccentColor::Blue),
                color_scheme: None,
            })
            .unwrap();
        let module = render_packages_module(&state).unwrap();
        assert_eq!(parse_packages_module(&module).unwrap(), state);
        assert_eq!(
            module.matches(&format!("  {option} = true;")).count(),
            1,
            "{option}"
        );
        assert_eq!(module.matches("programs.dconf.enable =").count(), 1);
        let removed = state.without_setup("virt-manager").unwrap();
        assert!(
            render_packages_module(&removed)
                .unwrap()
                .contains(&format!("  {option} = true;"))
        );
        assert!(removed.without_setup("hello").unwrap().setups.is_empty());
    }
}

#[test]
fn device_access_and_service_group_prerequisites_are_closed() {
    for (group, required) in SYSTEM_GROUPS {
        let mut setup = example();
        setup.settings.groups = vec![group.to_string()];
        setup.settings.enable.clear();
        if required.is_empty() {
            setup.normalize().unwrap();
        } else {
            assert!(setup.normalize().is_err(), "{group}");
            setup.settings.enable.push(required.to_string());
            setup.normalize().unwrap();
        }
    }
    let mut setup = example();
    setup.settings.enable = vec!["programs.appimage.binfmt".into()];
    setup.settings.groups.clear();
    setup.user = None;
    setup.uid = None;
    assert!(setup.normalize().is_err());
    setup
        .settings
        .enable
        .push("programs.appimage.enable".into());
    setup.normalize().unwrap();
    for forbidden in ["wheel", "root", "disk", "input", "podman"] {
        setup.settings.groups = vec![forbidden.into()];
        assert!(setup.normalize().is_err());
    }
}

#[test]
fn manual_instructions_are_complete_or_rejected_never_truncated() {
    let message = format!(
        "1. Edit the host module.\n{}\n2. Rebuild your existing flake target.",
        "Instructions. ".repeat(80)
    );
    let envelope: ModelEnvelope =
        serde_json::from_value(serde_json::json!({"action":"explain", "message":message})).unwrap();
    assert_eq!(
        ModelAction::try_from(envelope).unwrap(),
        ModelAction::Explain {
            message: message.clone()
        }
    );
    for action in ["explain", "install_package"] {
        let envelope: ModelEnvelope = serde_json::from_value(serde_json::json!({"action":action, "package": if action == "install_package" { Some("hello") } else { None }, "message":"x".repeat(crate::MAX_MODEL_MESSAGE_CHARS + 1)})).unwrap();
        assert!(ModelAction::try_from(envelope).is_err());
    }
}
