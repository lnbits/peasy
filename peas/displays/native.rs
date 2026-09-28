use crate::resource_native::*;
fn desktop() -> String {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase()
}
fn variant(v: &Value) -> &Value {
    v.get("data").unwrap_or(v)
}
pub fn inspect(r: &dyn ResourceRunner) -> Result<Value> {
    inspect_for(&desktop(), r)
}
fn inspect_for(desktop: &str, r: &dyn ResourceRunner) -> Result<Value> {
    if desktop.contains("gnome") {
        let value = json_run(
            r,
            Tool::Busctl,
            &[
                "--user",
                "--json=short",
                "call",
                "org.gnome.Mutter.DisplayConfig",
                "/org/gnome/Mutter/DisplayConfig",
                "org.gnome.Mutter.DisplayConfig",
                "GetCurrentState",
            ],
        )?;
        let data = &value["data"];
        let monitors = data[1].as_array().context("invalid Mutter monitors")?;
        let outputs = monitors
            .iter()
            .map(|m| json!({"connector":m[0][0],"identity":m[0],"modes":m[1],"properties":m[2]}))
            .collect::<Vec<_>>();
        return Ok(
            json!({"backend":"gnome","outputs":outputs,"layout":data[2],"properties":data[3]}),
        );
    }
    if desktop.contains("kde") || desktop.contains("plasma") {
        return Ok(
            json!({"backend":"plasma","configuration":json_run(r,Tool::KscreenDoctor,&["--json"])?}),
        );
    }
    if desktop.contains("hyprland") || std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        let monitors = json_run(r, Tool::Hyprctl, &["-j", "monitors", "all"])?;
        let monitors=monitors.as_array().context("invalid Hyprland monitors")?.iter().map(|m|json!({"name":m["name"],"description":m["description"],"serial":m["serial"],"width":m["width"],"height":m["height"],"refreshRate":m["refreshRate"],"x":m["x"],"y":m["y"],"scale":m["scale"],"disabled":m["disabled"],"availableModes":m["availableModes"]})).collect::<Vec<_>>();
        return Ok(json!({"backend":"hyprland","outputs":monitors}));
    }
    bail!("display layout control currently supports GNOME, Plasma and Hyprland sessions")
}
pub fn snapshot(change: &ResourceChange, r: &dyn ResourceRunner) -> Result<Value> {
    let data = inspect(r)?;
    validate_snapshot(change, &data)?;
    Ok(data)
}
fn validate_snapshot(change: &ResourceChange, data: &Value) -> Result<()> {
    let ResourceChange::Display {
        connector,
        mode,
        scale_percent,
        primary,
        ..
    } = change
    else {
        bail!("not a display operation");
    };
    match data["backend"].as_str() {
        Some("gnome") => {
            let outputs = data["outputs"].as_array().context("missing outputs")?;
            if outputs.iter().any(|o| {
                variant(&o["properties"]["is-underscanning"]) == true
                    || variant(&o["properties"]["current-color-mode"])
                        .as_u64()
                        .is_some_and(|v| v != 0)
            }) {
                bail!("use desktop settings for layouts with HDR or underscanning");
            }
            let output = outputs
                .iter()
                .find(|o| o["connector"] == *connector)
                .context("display disconnected")?;
            let selected = output["modes"]
                .as_array()
                .context("missing modes")?
                .iter()
                .find(|m| m[0] == *mode)
                .context("select an exact discovered display mode")?;
            if !selected[5].as_array().is_some_and(|scales| {
                scales
                    .iter()
                    .filter_map(Value::as_f64)
                    .any(|s| (s - *scale_percent as f64 / 100.).abs() < 0.001)
            }) {
                bail!("scale is unsupported for this mode");
            }
            let logical = data["layout"]
                .as_array()
                .context("missing display layout")?
                .iter()
                .find(|l| {
                    l[5].as_array()
                        .is_some_and(|ms| ms.iter().any(|m| m[0] == *connector))
                })
                .context("enable this display in desktop settings first")?;
            if logical[5].as_array().is_none_or(|ms| ms.len() != 1) {
                bail!("change mirrored displays in desktop settings");
            }
        }
        Some("plasma") => {
            let outputs = data["configuration"]["outputs"]
                .as_array()
                .context("missing outputs")?;
            let output = outputs
                .iter()
                .find(|o| o["name"] == *connector && o["connected"] == true)
                .context("display disconnected")?;
            let found = output["modes"].as_array().is_some_and(|modes| {
                modes.iter().any(|m| {
                    let size = &m["size"];
                    let rate = m["refreshRate"].as_f64().unwrap_or(0.);
                    let text = format!("{}x{}@{}", size["width"], size["height"], rate);
                    text == *mode
                })
            });
            if !found {
                bail!("select an exact discovered widthxheight@refresh display mode");
            }
        }
        Some("hyprland") => {
            if *primary {
                bail!("Hyprland has no equivalent primary-output flag");
            }
            let output = data["outputs"]
                .as_array()
                .context("missing outputs")?
                .iter()
                .find(|o| o["name"] == *connector)
                .context("display disconnected")?;
            if !output["availableModes"].as_array().is_some_and(|ms| {
                ms.iter()
                    .filter_map(Value::as_str)
                    .any(|m| m.trim_end_matches("Hz") == mode)
            }) {
                bail!("select an exact discovered display mode");
            }
        }
        _ => bail!("unsupported display backend"),
    }
    Ok(())
}
pub fn apply(change: &ResourceChange, snapshot: &Value, r: &dyn ResourceRunner) -> Result<()> {
    let ResourceChange::Display {
        connector,
        mode,
        scale_percent,
        x,
        y,
        primary,
    } = change
    else {
        bail!("not a display operation");
    };
    let scale = format!("{}", *scale_percent as f64 / 100.);
    match snapshot["backend"].as_str() {
        Some("hyprland") => {
            r.run(
                Tool::Hyprctl,
                &[
                    "keyword",
                    "monitor",
                    &format!("{connector},{mode},{x}x{y},{scale}"),
                ],
            )?;
        }
        Some("plasma") => {
            let mut args = vec![
                format!("output.{connector}.mode.{mode}"),
                format!("output.{connector}.scale.{scale}"),
                format!("output.{connector}.position.{x},{y}"),
            ];
            if *primary {
                args.push(format!("output.{connector}.priority.1"));
            }
            r.run(
                Tool::KscreenDoctor,
                &args.iter().map(String::as_str).collect::<Vec<_>>(),
            )?;
        }
        Some("gnome") => {
            let mut args = vec!["set".to_string()];
            let layout_mode = variant(&snapshot["properties"]["layout-mode"])
                .as_u64()
                .unwrap_or(1);
            args.extend([
                "--layout-mode".into(),
                if layout_mode == 1 {
                    "logical".into()
                } else {
                    "physical".into()
                },
            ]);
            for logical in snapshot["layout"].as_array().context("missing layout")? {
                let monitors = logical[5].as_array().context("missing logical monitors")?;
                let selected = monitors.iter().any(|m| m[0] == *connector);
                args.push("--logical-monitor".into());
                if selected && *primary || !*primary && logical[4] == true {
                    args.push("--primary".into());
                }
                let lx = if selected {
                    x.to_string()
                } else {
                    logical[0].to_string()
                };
                let ly = if selected {
                    y.to_string()
                } else {
                    logical[1].to_string()
                };
                let ls = if selected {
                    scale.clone()
                } else {
                    logical[2].to_string()
                };
                let transform = [
                    "normal",
                    "90",
                    "180",
                    "270",
                    "flipped",
                    "flipped-90",
                    "flipped-180",
                    "flipped-270",
                ]
                .get(logical[3].as_u64().unwrap_or(99) as usize)
                .context("unsupported display transform")?;
                args.extend([
                    "--x".into(),
                    lx,
                    "--y".into(),
                    ly,
                    "--scale".into(),
                    ls,
                    "--transform".into(),
                    (*transform).into(),
                ]);
                for m in monitors {
                    let name = m[0].as_str().context("invalid connector")?;
                    let output = snapshot["outputs"]
                        .as_array()
                        .context("missing outputs")?
                        .iter()
                        .find(|o| o["connector"] == name)
                        .context("missing monitor")?;
                    let current = output["modes"]
                        .as_array()
                        .context("missing modes")?
                        .iter()
                        .find(|mode| variant(&mode[6]["is-current"]) == true)
                        .context("missing current display mode")?;
                    let chosen = if name == connector {
                        mode.as_str()
                    } else {
                        current[0].as_str().context("invalid mode")?
                    };
                    args.extend([
                        "--monitor".into(),
                        name.into(),
                        "--mode".into(),
                        chosen.into(),
                    ]);
                }
            }
            let mut verify = args.iter().map(String::as_str).collect::<Vec<_>>();
            verify.insert(1, "--verify");
            r.run(Tool::Gdctl, &verify)?;
            r.run(
                Tool::Gdctl,
                &args.iter().map(String::as_str).collect::<Vec<_>>(),
            )?;
        }
        _ => bail!("unsupported display backend"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    struct Mock {
        response: Value,
        calls: RefCell<Vec<(Tool, Vec<String>)>>,
    }
    impl ResourceRunner for Mock {
        fn run(&self, tool: Tool, args: &[&str]) -> Result<String> {
            self.calls
                .borrow_mut()
                .push((tool, args.iter().map(|s| s.to_string()).collect()));
            if tool == Tool::Busctl {
                Ok(self.response.to_string())
            } else {
                Ok(String::new())
            }
        }
    }
    fn change() -> ResourceChange {
        ResourceChange::Display {
            connector: "DP-1".into(),
            mode: "1920x1080@60".into(),
            scale_percent: 100,
            x: 0,
            y: 0,
            primary: true,
        }
    }
    #[test]
    fn gnome_layout_preserves_other_monitors_and_verifies_before_apply() {
        let spec = |name| json!([name, "Vendor", "Model", "serial"]);
        let mode = json!(["1920x1080@60",1920,1080,60.0,1.0,[1.0,1.5,2.0],{"is-current":{"type":"b","data":true}}]);
        let r = Mock {
            response: json!({"data":[9,[[spec("DP-1"),[mode.clone()],{}],[spec("DP-2"),[mode],{}]],[[0,0,1.0,0,true,[spec("DP-1")],{}],[1920,0,1.5,0,false,[spec("DP-2")],{}]],{"layout-mode":{"type":"u","data":1}}]}),
            calls: RefCell::new(vec![]),
        };
        let data = inspect_for("gnome", &r).unwrap();
        validate_snapshot(&change(), &data).unwrap();
        apply(&change(), &data, &r).unwrap();
        let calls = r.calls.borrow();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[1].0, Tool::Gdctl);
        assert!(calls[1].1.contains(&"--verify".into()));
        let applied = &calls[2].1;
        assert!(applied.contains(&"DP-2".into()));
        assert!(applied.contains(&"1.5".into()));
        assert!(applied.contains(&"logical".into()));
        assert_eq!(applied.iter().filter(|a| *a == "--primary").count(), 1);
        let mut unsupported = data.clone();
        unsupported["outputs"][0]["properties"]["current-color-mode"] =
            json!({"type":"u","data":1});
        assert!(validate_snapshot(&change(), &unsupported).is_err());
    }
    #[test]
    fn plasma_and_hyprland_require_real_modes_and_supported_primary_semantics() {
        let data = json!({"backend":"plasma","configuration":{"outputs":[{"name":"DP-1","connected":true,"modes":[{"size":{"width":1920,"height":1080},"refreshRate":60.0}]}]}});
        validate_snapshot(&change(), &data).unwrap();
        let mut bad = change();
        if let ResourceChange::Display { mode, .. } = &mut bad {
            *mode = "3840x2160@240".into();
        }
        assert!(validate_snapshot(&bad, &data).is_err());
        let hypr = json!({"backend":"hyprland","outputs":[{"name":"DP-1","availableModes":["1920x1080@60Hz"]}]});
        assert!(validate_snapshot(&change(), &hypr).is_err());
        let mut live = change();
        if let ResourceChange::Display { primary, .. } = &mut live {
            *primary = false;
        }
        validate_snapshot(&live, &hypr).unwrap();
    }
}
