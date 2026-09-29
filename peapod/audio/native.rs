use crate::resource_native::*;
pub fn inspect(r: &dyn ResourceRunner) -> Result<Value> {
    let objects = json_run(r, Tool::PwDump, &["Node"])?;
    let nodes=objects.as_array().context("invalid PipeWire node list")?.iter().filter_map(|v| {
        let p=&v["info"]["props"];
        if !["Audio/Sink","Audio/Source"].iter().any(|c|p["media.class"]==*c) {return None;}
        Some(json!({"id":v["id"],"serial":p["object.serial"],"name":p["node.name"],"description":p["node.description"],"class":p["media.class"]}))
    }).take(128).collect::<Vec<_>>();
    Ok(
        json!({"devices":nodes,"limits":"Device defaults, volume and mute only; no per-application routing."}),
    )
}
pub fn snapshot(change: &ResourceChange, r: &dyn ResourceRunner) -> Result<Value> {
    let ResourceChange::Audio { id, .. } = change else {
        bail!("not an audio operation");
    };
    let data = inspect(r)?;
    let node = data["devices"]
        .as_array()
        .context("missing devices")?
        .iter()
        .find(|d| d["id"] == *id)
        .context("audio device is no longer present")?;
    if node["serial"].is_null() {
        bail!("audio device has no stable object serial");
    }
    Ok(node.clone())
}
pub fn apply(change: &ResourceChange, r: &dyn ResourceRunner) -> Result<()> {
    let ResourceChange::Audio { id, action, volume } = change else {
        bail!("not an audio operation");
    };
    let id = id.to_string();
    let value = match action {
        AudioAction::Volume => format!("{}%", volume.context("volume required")?),
        AudioAction::Mute => "1".into(),
        AudioAction::Unmute => "0".into(),
        AudioAction::Default => String::new(),
    };
    let mut args = vec![action.value(), &id];
    if !value.is_empty() {
        args.push(&value);
    }
    r.run(Tool::Wpctl, &args)?;
    Ok(())
}
