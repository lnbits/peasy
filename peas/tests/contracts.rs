//! Cross-pea compatibility: existing model vocabulary and serialized action shapes.
use crate::{EngineInput, ModelAction, ModelEnvelope};
use serde_json::Value;
use std::collections::BTreeSet;

#[test]
fn existing_actions_keep_their_validation_and_wire_format() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("actions.json")).unwrap();
    for case in cases {
        let envelope: ModelEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let action = ModelAction::try_from(envelope).unwrap();
        assert_eq!(
            serde_json::to_value(action).unwrap(),
            case["input"]["action"]
        );
        let input: EngineInput = serde_json::from_value(case["input"].clone()).unwrap();
        assert_eq!(serde_json::to_value(input).unwrap(), case["input"]);
    }
}

#[test]
fn compatibility_cases_cover_every_advertised_action() {
    let cases: Vec<Value> = serde_json::from_str(include_str!("actions.json")).unwrap();
    let schema: Value = serde_json::from_str(include_str!("model-schema.json")).unwrap();
    let covered: BTreeSet<_> = cases
        .iter()
        .map(|case| case["envelope"]["action"].as_str().unwrap())
        .collect();
    let advertised: BTreeSet<_> = schema["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| action.as_str().unwrap())
        .collect();
    assert_eq!(covered, advertised);
}
