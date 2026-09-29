//! Declarative resource fixture for Nix evaluation; never applies anything.
use peasy_core::{resources::*, *};
pub fn render() -> String {
    let mut state = PackageState::default();
    for value in
        serde_json::from_str::<Vec<serde_json::Value>>(include_str!("resource-changes.json"))
            .unwrap()
    {
        let change: ResourceChange = serde_json::from_value(value).unwrap();
        if change.persistent() {
            let uid = if matches!(change, ResourceChange::UserCreate { .. }) {
                1001
            } else {
                1000
            };
            state.resources = state
                .resources
                .changed(&change, Some(("peasytest", uid)))
                .unwrap();
        }
    }
    state.resources = state
        .resources
        .changed(
            &ResourceChange::UserCreate {
                name: "secondguest".into(),
            },
            Some(("secondguest", 1002)),
        )
        .unwrap();
    state.resources = state
        .resources
        .changed(
            &ResourceChange::UserDisabled {
                name: "secondguest".into(),
                disabled: true,
            },
            None,
        )
        .unwrap();
    state.resources.services.push(ManagedService::Libvirtd);
    state.resources.groups[0].groups.push("libvirtd".into());
    let setup: ManagedSetup =
        serde_json::from_str(include_str!("../system_configuration/example.json")).unwrap();
    state = state.with_setup(setup).unwrap();
    render_packages_module(&state).unwrap()
}
