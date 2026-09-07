//! Run the compatibility corpus through the actual import-free Wasm guest.
use crate::EngineHost;

#[test]
#[ignore = "build peasy-engine for wasm32-unknown-unknown and set PEASY_TEST_ENGINE; Nix runs this automatically"]
fn compiled_wasm_preserves_all_pea_decisions() {
    let path =
        std::env::var_os("PEASY_TEST_ENGINE").expect("PEASY_TEST_ENGINE must name the built guest");
    // EngineHost::load rejects every import, including WASI and host functions.
    let engine = EngineHost::load(std::path::Path::new(&path)).unwrap();
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!("actions.json")).unwrap();
    for case in cases {
        let input = serde_json::from_value(case["input"].clone()).unwrap();
        let decision = engine.resolve(&input).unwrap();
        assert_eq!(serde_json::to_value(decision).unwrap(), case["decision"]);
    }
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!("actions.json")).unwrap();
    let mut input: peasy_core::EngineInput =
        serde_json::from_value(cases[0]["input"].clone()).unwrap();
    let mut missing = input.clone();
    missing.candidates.clear();
    assert!(matches!(
        engine.resolve(&missing).unwrap(),
        peasy_core::EngineDecision::Reject(_)
    ));
    if let peasy_core::ModelAction::InstallPackage {
        setup: Some(setup), ..
    } = &mut input.action
    {
        setup.groups = vec!["wheel".into()];
    }
    assert!(matches!(
        engine.resolve(&input).unwrap(),
        peasy_core::EngineDecision::Reject(_)
    ));
}
