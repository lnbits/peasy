//! Exercise every existing action and both package membership rejection paths.
use crate::decide;

#[test]
fn existing_actions_keep_their_policy_decisions() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!("actions.json")).unwrap();
    for case in cases {
        let input = serde_json::from_value(case["input"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(decide(input)).unwrap(),
            case["decision"]
        );
    }
}

#[test]
fn setup_cannot_bypass_wasm_validation_or_candidate_membership() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!("actions.json")).unwrap();
    let input: peasy_core::EngineInput = serde_json::from_value(cases[0]["input"].clone()).unwrap();
    let mut missing = input.clone();
    missing.candidates.clear();
    assert!(matches!(
        decide(missing),
        peasy_core::EngineDecision::Reject(_)
    ));
    let mut invalid = input;
    if let peasy_core::ModelAction::InstallPackage {
        setup: Some(setup), ..
    } = &mut invalid.action
    {
        setup.enable = vec!["system.activationScripts.inject.text".into()];
    } else {
        panic!("fixture must exercise setup");
    }
    assert!(matches!(
        decide(invalid),
        peasy_core::EngineDecision::Reject(_)
    ));
}
