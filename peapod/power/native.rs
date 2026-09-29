use crate::resource_native::*;
pub fn inspect(r: &dyn ResourceRunner) -> Result<Value> {
    Ok(
        json!({"profiles":r.run(Tool::Powerprofilesctl,&["list"]).ok(),"current_profile":r.run(Tool::Powerprofilesctl,&["get"]).ok(),"battery":r.run(Tool::Upower,&["--show-info","/org/freedesktop/UPower/devices/DisplayDevice"]).ok(),"limits":"Unavailable services are reported as null. This does not enable a power-management daemon."}),
    )
}
pub fn snapshot(profile: PowerProfile, r: &dyn ResourceRunner) -> Result<Value> {
    let profiles = r.run(Tool::Powerprofilesctl, &["list"])?;
    let names = profiles
        .lines()
        .map(|l| {
            l.trim()
                .trim_start_matches('*')
                .trim()
                .trim_end_matches(':')
        })
        .filter(|l| ["balanced", "power-saver", "performance"].contains(l))
        .collect::<Vec<_>>();
    if !names.contains(&profile.value()) {
        bail!("requested power profile is not supported");
    }
    Ok(json!({"profiles":names,"current":r.run(Tool::Powerprofilesctl,&["get"])?.trim()}))
}
