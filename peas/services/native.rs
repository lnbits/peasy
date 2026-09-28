use crate::resource_native::*;
pub fn inspect(target: Option<&str>, r: &dyn ResourceRunner) -> Result<Value> {
    if let Some(name) = target {
        unit(name)?;
        let output=r.run(Tool::Systemctl,&["show","--no-pager","--property=Id,LoadState,ActiveState,SubState,Result,ExecMainCode,ExecMainStatus,InvocationID,FragmentPath","--",name])?;
        let fields: serde_json::Map<String, Value> = output
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_owned(), json!(v)))
            .collect();
        Ok(Value::Object(fields))
    } else {
        let data = json_run(
            r,
            Tool::Systemctl,
            &[
                "list-units",
                "--all",
                "--type=service",
                "--no-pager",
                "--output=json",
            ],
        )?;
        let units = data.as_array().context("invalid systemd units")?;
        Ok(
            json!({"units":units.iter().take(256).map(|u|json!({"unit":u["unit"],"load":u["load"],"active":u["active"],"sub":u["sub"]})).collect::<Vec<_>>(),"truncated":units.len()>256}),
        )
    }
}
pub fn snapshot(name: &str, r: &dyn ResourceRunner) -> Result<Value> {
    unit(name)?;
    if [
        "peasy",
        "systemd",
        "dbus",
        "nix-daemon",
        "polkit",
        "display-manager",
    ]
    .iter()
    .any(|p| name.starts_with(p))
    {
        bail!("this infrastructure service is protected");
    }
    let value = inspect(Some(name), r)?;
    if value["Id"] != name
        || value["LoadState"] != "loaded"
        || value["FragmentPath"].as_str().is_none_or(str::is_empty)
    {
        bail!("select an existing loaded service");
    }
    Ok(value)
}
