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
    let mut actual = model_schema();
    assert_eq!(
        actual["properties"]["setup"],
        crate::system_configuration::schema()
    );
    actual["properties"]
        .as_object_mut()
        .unwrap()
        .remove("setup");
    actual["required"]
        .as_array_mut()
        .unwrap()
        .retain(|field| field != "setup");
    assert_eq!(actual, expected);
}
