use crate::resource_native::*;
pub fn inspect(r: &dyn ResourceRunner) -> Result<Value> {
    let memory = read_bounded("/proc/meminfo")?;
    let memory = memory
        .lines()
        .filter(|l| {
            ["MemTotal:", "MemAvailable:", "SwapTotal:", "SwapFree:"]
                .iter()
                .any(|p| l.starts_with(p))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let failed = json_run(
        r,
        Tool::Systemctl,
        &["list-units", "--failed", "--no-pager", "--output=json"],
    )?;
    Ok(
        json!({"load":read_bounded("/proc/loadavg")?,"memory":memory,"io_pressure":read_bounded("/proc/pressure/io").ok(),"memory_pressure":read_bounded("/proc/pressure/memory").ok(),"disk_usage":r.run(Tool::Df,&["-B1","--output=source,size,used,avail,pcent,target","/","/nix/store"] )?,"failed_units":failed.as_array().map(|a|a.iter().take(64).map(|u|json!({"unit":u["unit"],"active":u["active"],"sub":u["sub"]})).collect::<Vec<_>>()),"routes":json_run(r,Tool::Ip,&["-j","route","show"]).ok(),"limits":"Point-in-time readings, not proof of cause. Route presence does not establish Internet connectivity. No journal messages or process command lines are sent to the model."}),
    )
}
