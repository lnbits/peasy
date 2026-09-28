use crate::*;
use serde_json::{Value, json};
pub fn model_response_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "resource_query": resources::query_schema(),
            "resource_change": resources::change_schema(),
            "pea_id": {"type":["string","null"],"maxLength":48},
            "network": network_schema(),
            "setup": setup_schema(),
            "action": { "type": "string", "description": "Choose the capability that best fulfills the user's actual request.", "enum": ["inspect_resources", "change_resources", "discover_peas", "use_pea", "disable_pea", "inspect_network", "configure_network", "search_package", "search_appimage", "check_package", "list_themes", "list_wifi", "hyprland_status", "install_package", "remove_package", "set_theme", "set_hyprland_setting", "hyprland_dispatch", "connect_wifi", "connect_bluetooth", "create_calendar_event", "explain", "cancel"] },
            "query": { "type": ["string", "null"], "description": "Concise package, application, project, device, or upstream search name; never the whole user sentence.", "maxLength": MAX_QUERY_BYTES },
            "package": { "type": ["string", "null"], "description": "Exact package attribute from package_candidates or peasy_installed_packages.", "maxLength": MAX_ATTRIBUTE_BYTES },
            "package_version": { "type": ["string", "null"], "maxLength": 64 },
            "repository": { "type": ["string", "null"], "description": "Exact GitHub owner/repository for an upstream AppImage when known.", "maxLength": 201 },
            "message": { "type": ["string", "null"], "description": "Concise user-facing explanation when useful.", "maxLength": crate::MAX_MODEL_MESSAGE_CHARS },
            "theme_color": { "type": ["string", "null"], "enum": ["blue", "teal", "green", "yellow", "orange", "red", "pink", "purple", "slate", null] },
            "theme_mode": { "type": ["string", "null"], "enum": ["system", "light", "dark", null] },
            "ssid": { "type": ["string", "null"], "maxLength": MAX_SSID_BYTES },
            "device": { "type": ["string", "null"], "maxLength": MAX_QUERY_BYTES },
            "event_title": { "type": ["string", "null"], "description": "Concise calendar event title inferred from the request.", "maxLength": MAX_EVENT_TITLE_BYTES },
            "event_start": { "type": ["string", "null"], "description": "Local date and time in exactly YYYY-MM-DDTHH:MM:SS format, resolved relative to current_local_time.", "minLength": LOCAL_DATETIME_BYTES, "maxLength": LOCAL_DATETIME_BYTES },
            "duration_minutes": { "type": ["integer", "null"], "minimum": 5, "maximum": 1440 },
            "hyprland_setting": { "type": ["string", "null"], "enum": ["gaps_inner", "gaps_outer", "border_size", "corner_radius", "animations", "blur", "active_opacity", "inactive_opacity", "natural_scroll", "layout", null] },
            "hyprland_value": { "type": ["string", "null"], "maxLength": 32 },
            "hyprland_dispatch": { "type": ["string", "null"], "enum": ["switch_workspace", "move_window_to_workspace", "focus_direction", "toggle_floating", "toggle_fullscreen", null] },
            "hyprland_argument": { "type": ["string", "null"], "maxLength": 32 }
        },
        "required": ["resource_query", "resource_change", "pea_id", "network", "action", "query", "package", "package_version", "repository", "message", "theme_color", "theme_mode", "ssid", "device", "event_title", "event_start", "duration_minutes", "hyprland_setting", "hyprland_value", "hyprland_dispatch", "hyprland_argument", "setup"]
    })
}
