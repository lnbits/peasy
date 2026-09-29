use crate::{resources::*, *};
use serde_json::json;

#[test]
fn every_resource_operation_has_a_closed_wire_contract() {
    let operations: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("resource-changes.json")).unwrap();
    for value in operations {
        let change: ResourceChange = serde_json::from_value(value.clone()).unwrap();
        change.validate().unwrap();
        assert_eq!(serde_json::to_value(&change).unwrap(), value);
        let envelope: ModelEnvelope =
            serde_json::from_value(json!({"action":"change_resources","resource_change":value}))
                .unwrap();
        assert_eq!(
            ModelAction::try_from(envelope).unwrap(),
            ModelAction::ChangeResources { change }
        );
        let mut hostile = value.clone();
        hostile["command"] = json!("rm -rf /");
        assert!(serde_json::from_value::<ResourceChange>(hostile).is_err());
    }
    for domain in [
        ResourceDomain::Diagnostics,
        ResourceDomain::Services,
        ResourceDomain::Storage,
        ResourceDomain::NixMaintenance,
        ResourceDomain::Users,
        ResourceDomain::Firewall,
        ResourceDomain::Printing,
        ResourceDomain::Displays,
        ResourceDomain::Audio,
        ResourceDomain::Power,
    ] {
        ResourceQuery {
            domain,
            target: None,
        }
        .validate()
        .unwrap();
    }
}

#[test]
fn hostile_resource_values_are_rejected_before_execution() {
    for value in [
        json!({"operation":"service","unit":"--root=/tmp.service","action":"restart"}),
        json!({"operation":"service","unit":"x/../../sshd.service","action":"start"}),
        json!({"operation":"disk","device":"/dev/../etc/passwd","action":"format","filesystem":"ext4"}),
        json!({"operation":"disk","device":"/dev/sdb1","action":"mount","filesystem":"ext4"}),
        json!({"operation":"persistent_mount","uuid":"abcd","name":"${abort}","filesystem":"ext4","present":true}),
        json!({"operation":"firewall","tcp":[0],"udp":[],"trusted_interfaces":[]}),
        json!({"operation":"user_create","name":"root"}),
        json!({"operation":"user_groups","groups":["wheel"]}),
        json!({"operation":"nix_delete_generations","generations":[]}),
        json!({"operation":"audio","id":1,"action":"volume","volume":101}),
        json!({"operation":"printer","name":"printer","action":"add","uri":"file:///etc/shadow"}),
        json!({"operation":"printer","name":"printer","action":"add","uri":"ipp://user:pass@host/printer"}),
        json!({"operation":"display","connector":"DP-1,disable","mode":"1920x1080@60","scale_percent":100,"x":0,"y":0,"primary":false}),
    ] {
        assert!(
            serde_json::from_value::<ResourceChange>(value.clone()).is_err()
                || serde_json::from_value::<ResourceChange>(value)
                    .unwrap()
                    .validate()
                    .is_err()
        );
    }
}

#[test]
fn new_peas_cannot_cross_domains_or_gain_write_permissions() {
    let mut manifest: pea::PeaManifest =
        serde_json::from_str(include_str!("../diagnostics/pea.json")).unwrap();
    let read = ModelAction::InspectResources {
        query: ResourceQuery {
            domain: ResourceDomain::Diagnostics,
            target: None,
        },
    };
    let write = ModelAction::ChangeResources {
        change: ResourceChange::Service {
            unit: "caddy.service".into(),
            action: ServiceAction::Restart,
        },
    };
    assert!(manifest.permits(&read));
    assert!(!manifest.permits(&write));
    manifest.permissions = vec!["services.read".into()];
    manifest.response_schema = pea::schema_for_permissions(&manifest.permissions);
    assert!(!manifest.permits(&read));
    assert!(!manifest.permits(&write));
    manifest.permissions.push("services.write".into());
    manifest.response_schema = pea::schema_for_permissions(&manifest.permissions);
    assert!(manifest.permits(&write));
    let old: pea::PeaManifest = serde_json::from_str(include_str!("api2-packages.json")).unwrap();
    assert!(!old.permits(&read));
    assert!(!old.permits(&write));
    manifest.host_api = 3;
    assert!(manifest.validate().is_err());
}

#[test]
fn managed_resources_roundtrip_and_survive_unrelated_changes_and_portable_restore() {
    let mut state = PackageState::default();
    for value in
        serde_json::from_str::<Vec<serde_json::Value>>(include_str!("resource-changes.json"))
            .unwrap()
    {
        let change: ResourceChange = serde_json::from_value(value).unwrap();
        if !change.persistent() {
            continue;
        }
        let caller = match &change {
            ResourceChange::UserCreate { name } => Some((name.as_str(), 1001)),
            _ => Some(("peasytest", 1000)),
        };
        state.resources = state.resources.changed(&change, caller).unwrap();
    }
    let source = render_packages_module(&state).unwrap();
    assert_eq!(parse_packages_module(&source).unwrap(), state);
    let added = state
        .with_change(PackageOperation::Install, "hello")
        .unwrap();
    assert_eq!(added.resources, state.resources);
    let backup = PortableBackup::from_state(&PackageState::default());
    for mode in [RestoreMode::Merge, RestoreMode::Replace] {
        assert_eq!(
            backup.restore(&state, mode).unwrap().resources,
            state.resources
        );
    }
    assert!(source.contains("/dev/disk/by-uuid/abcd-1234"));
    assert!(source.contains("initialHashedPassword = \"!\""));
    assert!(source.contains("peasyBase.imports or []"));
    assert!(
        parse_packages_module(
            &source.replace("allowedTCPPorts = [ 8080 ]", "allowedTCPPorts = [ 22 ]")
        )
        .is_err()
    );
}
