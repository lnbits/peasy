//! Preserve the existing contract while allowing the explicitly added setup field.
use crate::{agent_capability_guide, model_instructions, model_schema};

#[test]
fn existing_model_contract_is_preserved_with_additive_setup_schema() {
    assert_eq!(
        model_instructions(),
        include_str!("model-instructions.txt").trim_end_matches('\n')
    );
    assert_eq!(
        agent_capability_guide(),
        include_str!("capability-guide.txt").trim_end_matches('\n')
    );
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("model-schema.json")).unwrap();
    assert_eq!(model_schema(), expected);
}
