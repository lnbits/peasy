use super::*;
use crate::{
    ModelAction, ModelEnvelope, PackageOperation, ThemeSettings, parse_packages_module,
    render_packages_module,
};

fn example() -> ManagedSetup {
    serde_json::from_str(include_str!("example.json")).unwrap()
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
