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
    let mut entries: Vec<(&str,Vec<String>,Vec<String>,&str)> = vec![
        ("packages", vec!["Find, install and remove Nixpkgs applications and their reviewed system integration".into()], vec!["packages".into()], include_str!("../../../peapod/packages/instructions.txt")),
        ("system_configuration", vec!["Configure application dependencies, reviewed NixOS enable options and caller-bound groups".into()], vec!["packages".into()], include_str!("../../../peapod/system_configuration/instructions.txt")),
        ("appimages", vec!["Discover and install pinned upstream AppImage releases".into()], vec!["packages".into()], include_str!("../../../peapod/appimages/instructions.txt")),
        ("appearance", vec!["Inspect and configure supported desktop colours and light or dark appearance".into()], vec!["appearance".into()], include_str!("../../../peapod/appearance/instructions.txt")),
        ("wifi", vec!["Discover nearby Wi-Fi networks and connect to an exact SSID".into()], vec!["wifi".into()], include_str!("../../../peapod/wifi/instructions.txt")),
        ("bluetooth", vec!["Discover and connect or pair Bluetooth devices".into()], vec!["bluetooth".into()], include_str!("../../../peapod/bluetooth/instructions.txt")),
        ("calendar", vec!["Prepare local calendar events for import".into()], vec!["calendar".into()], include_str!("../../../peapod/calendar/instructions.txt")),
        ("hyprland", vec!["Inspect and control supported live Hyprland settings and window actions".into()], vec!["hyprland".into()], include_str!("../../../peapod/hyprland/instructions.txt")),
        ("networking", vec!["Inspect interfaces, connection profiles, IPv4 addressing and routes".into(),"Configure DHCP, static IPv4, DNS, shared connectivity, Wi-Fi modes and profile activation".into()], vec!["network.read".into(),"network.session".into(),"network.system".into()], include_str!("../../../peapod/networking/instructions.txt")),    ];
    entries.push((
        "diagnostics",
        vec!["System diagnostics: bounded inspection".into()],
        vec!["diagnostics.read".into()],
        include_str!("../../../peapod/diagnostics/instructions.txt"),
    ));
    entries.push((
        "services",
        vec!["Services: bounded inspection and reviewed resource management".into()],
        vec!["services.read".into(), "services.write".into()],
        include_str!("../../../peapod/services/instructions.txt"),
    ));
    entries.push((
        "storage",
        vec!["Storage: bounded inspection and reviewed resource management".into()],
        vec!["storage.read".into(), "storage.write".into()],
        include_str!("../../../peapod/storage/instructions.txt"),
    ));
    entries.push((
        "nix_maintenance",
        vec!["Nix maintenance: bounded inspection and reviewed resource management".into()],
        vec![
            "nix_maintenance.read".into(),
            "nix_maintenance.write".into(),
        ],
        include_str!("../../../peapod/nix_maintenance/instructions.txt"),
    ));
    entries.push((
        "users",
        vec!["Users: bounded inspection and reviewed resource management".into()],
        vec!["users.read".into(), "users.write".into()],
        include_str!("../../../peapod/users/instructions.txt"),
    ));
    entries.push((
        "firewall",
        vec!["Firewall: bounded inspection and reviewed resource management".into()],
        vec!["firewall.read".into(), "firewall.write".into()],
        include_str!("../../../peapod/firewall/instructions.txt"),
    ));
    entries.push((
        "printing",
        vec!["Printing: bounded inspection and reviewed resource management".into()],
        vec!["printing.read".into(), "printing.write".into()],
        include_str!("../../../peapod/printing/instructions.txt"),
    ));
    entries.push((
        "displays",
        vec!["Displays: bounded inspection and reviewed resource management".into()],
        vec!["displays.read".into(), "displays.write".into()],
        include_str!("../../../peapod/displays/instructions.txt"),
    ));
    entries.push((
        "audio",
        vec!["Audio: bounded inspection and reviewed resource management".into()],
        vec!["audio.read".into(), "audio.write".into()],
        include_str!("../../../peapod/audio/instructions.txt"),
    ));
    entries.push((
        "power",
        vec!["Power: bounded inspection and reviewed resource management".into()],
        vec!["power.read".into(), "power.write".into()],
        include_str!("../../../peapod/power/instructions.txt"),
    ));
    entries.push((
        "applications",
        vec!["Find and open installed desktop applications".into()],
        vec!["applications.read".into(), "applications.write".into()],
        include_str!("../../../peapod/applications/instructions.txt"),
    ));
    for (id, capabilities, permissions, instructions) in entries {
        let manifest = PeaManifest {
            id: id.into(),
            version: "1.5.0".into(),
            host_api: HOST_API,
            capabilities,
            response_schema: schema_for_permissions(&permissions),
            permissions,
            instructions: instructions.trim_end().into(),
        };
        manifest.validate().unwrap();
        write(
            &root.join(format!("peapod/{id}/pea.json")),
            &manifest,
            check,
        );
    }
    write(
        &root.join("peapod/tests/model-schema.json"),
        &peasy_core::model_response_schema(),
        check,
    );
}
