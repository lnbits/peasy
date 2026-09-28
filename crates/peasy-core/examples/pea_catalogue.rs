//! Regenerate or verify the official data-only pea artifacts. No external code.
use peasy_core::pea::{HOST_API, PeaManifest, schema_for_permissions};
use std::{fs, path::Path};
fn write(path: &Path, value: &impl serde::Serialize, check: bool) {
    let bytes = serde_json::to_string_pretty(value).unwrap() + "\n";
    if check {
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            bytes,
            "stale generated file {}",
            path.display()
        );
    } else {
        fs::write(path, bytes).unwrap();
    }
}
fn main() {
    let check = std::env::args().any(|s| s == "--check");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let entries: Vec<(&str,Vec<String>,Vec<String>,&str)> = vec![
        ("packages", vec!["Find, install and remove Nixpkgs applications and their reviewed system integration".into()], vec!["packages".into()], include_str!("../../../peas/packages/instructions.txt")),
        ("system_configuration", vec!["Configure application dependencies, reviewed NixOS enable options and caller-bound groups".into()], vec!["packages".into()], include_str!("../../../peas/system_configuration/instructions.txt")),
        ("appimages", vec!["Discover and install pinned upstream AppImage releases".into()], vec!["packages".into()], include_str!("../../../peas/appimages/instructions.txt")),
        ("appearance", vec!["Inspect and configure supported desktop colours and light or dark appearance".into()], vec!["appearance".into()], include_str!("../../../peas/appearance/instructions.txt")),
        ("wifi", vec!["Discover nearby Wi-Fi networks and connect to an exact SSID".into()], vec!["wifi".into()], include_str!("../../../peas/wifi/instructions.txt")),
        ("bluetooth", vec!["Discover and connect or pair Bluetooth devices".into()], vec!["bluetooth".into()], include_str!("../../../peas/bluetooth/instructions.txt")),
        ("calendar", vec!["Prepare local calendar events for import".into()], vec!["calendar".into()], include_str!("../../../peas/calendar/instructions.txt")),
        ("hyprland", vec!["Inspect and control supported live Hyprland settings and window actions".into()], vec!["hyprland".into()], include_str!("../../../peas/hyprland/instructions.txt")),
        ("networking", vec!["Inspect interfaces, connection profiles, IPv4 addressing and routes".into(),"Configure DHCP, static IPv4, DNS, shared connectivity, Wi-Fi modes and profile activation".into()], vec!["network.read".into(),"network.session".into(),"network.system".into()], include_str!("../../../peas/networking/instructions.txt")),    ];
    for (id, capabilities, permissions, instructions) in entries {
        let manifest = PeaManifest {
            id: id.into(),
            version: "1.2.0".into(),
            host_api: HOST_API,
            capabilities,
            response_schema: schema_for_permissions(&permissions),
            permissions,
            instructions: instructions.trim_end().into(),
        };
        manifest.validate().unwrap();
        write(&root.join(format!("peas/{id}/pea.json")), &manifest, check);
    }
    write(
        &root.join("peas/tests/model-schema.json"),
        &peasy_core::model_response_schema(),
        check,
    );
}
