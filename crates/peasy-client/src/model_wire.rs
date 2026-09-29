//! Compact provider transport derived from the unchanged, versioned pea schema.
//! This changes JSON layout only; native actions and permission checks stay shared.
use anyhow::{Context, Result, bail};
use peasy_core::{ModelAction, ModelEnvelope, ValidationError};
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) const INSTRUCTIONS: &str = "Return one JSON object with a result field containing the chosen action and only that action's schema fields. Optional values within that action are null. Do not emit unrelated fields. The compact transport schema preserves the enabled pea's field constraints and permissions.";

fn fields(action: &str) -> Option<&'static [&'static str]> {
    Some(match action {
        "search_package" => &["query", "package_version"],
        "search_appimage" => &["query", "package_version", "repository"],
        "check_package" => &["query"],
        "install_package" => &["package", "message", "setup"],
        "remove_package" => &["package"],
        "set_theme" => &["theme_color", "theme_mode"],
        "connect_wifi" => &["ssid"],
        "connect_bluetooth" => &["device"],
        "create_calendar_event" => &["event_title", "event_start", "duration_minutes"],
        "set_hyprland_setting" => &["hyprland_setting", "hyprland_value"],
        "hyprland_dispatch" => &["hyprland_dispatch", "hyprland_argument"],
        "configure_network" => &["network"],
        "inspect_resources" => &["resource_query"],
        "change_resources" => &["resource_change"],
        "use_pea" | "disable_pea" => &["pea_id"],
        "explain" => &["message"],
        "cancel" | "discover_peas" | "list_themes" | "list_wifi" | "inspect_network"
        | "hyprland_status" => &[],
        _ => return None,
    })
}

pub(super) fn schema(declared: &Value) -> Value {
    let variants = declared["properties"]["action"]["enum"]
        .as_array().expect("host action enum")
        .iter().map(|action| {
            let action = action.as_str().expect("host action name");
            let mut properties = serde_json::Map::new();
            properties.insert("action".into(), json!({"type":"string", "enum":[action]}));
            let mut required = vec!["action"];
            for &field in fields(action).expect("host action has a compact representation") {
                // Frozen older schemas may omit a newer optional field.
                if let Some(constraint) = declared["properties"].get(field) {
                    properties.insert(field.into(), constraint.clone());
                    required.push(field);
                }
            }
            json!({"type":"object", "properties":properties, "required":required, "additionalProperties":false})
        }).collect::<Vec<_>>();
    json!({"type":"object", "properties":{"result":{"anyOf":variants}}, "required":["result"], "additionalProperties":false})
}

pub(super) fn decode(text: &str) -> Result<ModelAction> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Compact {
        result: ModelEnvelope,
    }
    let value: Value = serde_json::from_str(text)?;
    let envelope = if value.get("result").is_some() {
        let compact: Compact = serde_json::from_str(text)?;
        let object = value["result"]
            .as_object()
            .context("result must be an action object")?;
        let action = object
            .get("action")
            .and_then(Value::as_str)
            .context("action required")?;
        let allowed = fields(action).context("unknown action")?;
        if object
            .keys()
            .any(|key| key != "action" && !allowed.contains(&key.as_str()))
        {
            bail!(ValidationError::InvalidRequest(
                "response contains fields for a different action".into()
            ));
        }
        compact.result
    } else {
        // Existing local providers may still emit the flat envelope. It passes
        // the same native validation and pinned-pea permission gate.
        serde_json::from_str::<ModelEnvelope>(text)?
    };
    if envelope.action == "install_package" && envelope.package_version.is_some() {
        bail!(ValidationError::InvalidRequest(
            "version constraints require search_package before installation".into()
        ));
    }
    Ok(envelope.try_into()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_actions_preserve_the_native_contract() {
        let fixtures: Vec<Value> =
            serde_json::from_str(include_str!("../../../peapod/tests/actions.json")).unwrap();
        for fixture in fixtures {
            let envelope = &fixture["envelope"];
            let expected = decode(&envelope.to_string()).unwrap();
            let mut compact = envelope.as_object().unwrap().clone();
            let allowed = fields(envelope["action"].as_str().unwrap()).unwrap();
            compact.retain(|key, _| key == "action" || allowed.contains(&key.as_str()));
            let actual = decode(&json!({"result":compact}).to_string()).unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn compact_schema_preserves_every_declared_field_constraint() {
        let current = peasy_core::model_response_schema();
        let legacy: Value =
            serde_json::from_str(include_str!("../../../peapod/tests/api2-packages.json")).unwrap();
        for declared in [&current, &legacy["response_schema"]] {
            let compact = schema(declared);
            assert_eq!(compact["additionalProperties"], false);
            assert_eq!(compact["required"], json!(["result"]));
            let variants = compact["properties"]["result"]["anyOf"].as_array().unwrap();
            assert_eq!(
                variants.len(),
                declared["properties"]["action"]["enum"]
                    .as_array()
                    .unwrap()
                    .len()
            );
            for variant in variants {
                assert_eq!(variant["additionalProperties"], false);
                let properties = variant["properties"].as_object().unwrap();
                assert_eq!(
                    variant["required"].as_array().unwrap().len(),
                    properties.len()
                );
                for (key, constraint) in properties {
                    if key != "action" {
                        assert_eq!(constraint, &declared["properties"][key]);
                    }
                }
            }
            let install = variants
                .iter()
                .find(|v| v["properties"]["action"]["enum"][0] == "install_package")
                .unwrap();
            assert_eq!(install["properties"].as_object().unwrap().len(), 4);
        }
    }

    #[test]
    fn malformed_or_mixed_actions_cannot_hide_in_compact_output() {
        for text in [
            r#"{"result":{"action":"cancel"},"command":"root"}"#,
            r#"{"result":{"action":"cancel","package":"hello"}}"#,
            r#"{"result":{"action":"cancel","action":"list_wifi"}}"#,
            r#"{"result":{"action":"cancel"},"result":{"action":"list_wifi"}}"#,
            r#"{"action":"cancel","action":"list_wifi"}"#,
            r#"{"result":{"action":"install_package","package":"hello","package":"vlc"}}"#,
            r#"{"result":{"action":"install_package","package":"$(whoami)"}}"#,
            r#"{"result":{"action":"execute","command":"root"}}"#,
            r#"{"action":"install_package","package":"hello","package_version":"latest"}"#,
        ] {
            assert!(decode(text).is_err(), "accepted {text}");
        }
        assert_eq!(
            decode(r#"{"result":{"action":"cancel"}}"#).unwrap(),
            ModelAction::Cancel
        );
    }
}
