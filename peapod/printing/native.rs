use crate::resource_native::*;
pub fn inspect(r: &dyn ResourceRunner) -> Result<Value> {
    let queues = r.run(Tool::Lpstat, &["-v"])?;
    let discovered = r.run(Tool::Lpinfo, &["-v"]).unwrap_or_default();
    let network = r
        .run(
            Tool::Ippfind,
            &["-T", "3", "_ipp._tcp", "_ipps._tcp", "--print"],
        )
        .unwrap_or_default();
    let queues=queues.lines().filter_map(|l| {let (name,uri)=l.strip_prefix("device for ")?.split_once(": ")?;identifier(name,64).ok()?;Some(json!({"name":name,"uri":if uri.contains(['@', '?', '#']){"[credential-bearing URI hidden]"}else{uri}}))}).take(64).collect::<Vec<_>>();
    let uris = discovered
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .chain(network.lines())
        .filter(|u| {
            (u.starts_with("ipp://") || u.starts_with("ipps://")) && !u.contains(['@', '?', '#'])
        })
        .take(64)
        .collect::<Vec<_>>();
    Ok(
        json!({"queues":queues,"discovered_ipp_uris":uris,"default":r.run(Tool::Lpstat,&["-d"]).ok()}),
    )
}
pub fn snapshot(change: &ResourceChange, r: &dyn ResourceRunner) -> Result<Value> {
    let ResourceChange::Printer { name, action, uri } = change else {
        bail!("not a printer operation");
    };
    let data = inspect(r)?;
    let queue = data["queues"]
        .as_array()
        .context("missing printers")?
        .iter()
        .find(|p| p["name"] == *name);
    if *action == PrinterAction::Add {
        if queue.is_some() {
            bail!("cannot replace an existing CUPS queue");
        }
        if !data["discovered_ipp_uris"]
            .as_array()
            .is_some_and(|uris| uris.iter().any(|u| u.as_str() == uri.as_deref()))
        {
            bail!("select an exact discovered IPP URI");
        }
        Ok(json!({"name":name,"uri":uri,"new":true}))
    } else {
        Ok(queue.context("selected printer no longer exists")?.clone())
    }
}
pub fn apply(change: &ResourceChange, r: &dyn ResourceRunner) -> Result<()> {
    let ResourceChange::Printer { name, action, uri } = change else {
        bail!("not a printer operation");
    };
    match action {
        PrinterAction::Add => {
            r.run(
                Tool::Lpadmin,
                &[
                    "-p",
                    name,
                    "-E",
                    "-v",
                    uri.as_deref().context("URI required")?,
                    "-m",
                    "everywhere",
                ],
            )?;
        }
        PrinterAction::Default => {
            r.run(Tool::Lpoptions, &["-d", name])?;
        }
        PrinterAction::Test => {
            let page = std::env::var("PEASY_CUPS_TESTPAGE")
                .context("packaged CUPS test page is unavailable")?;
            if !page.starts_with("/nix/store/") {
                bail!("test page must be a packaged resource");
            }
            r.run(Tool::Lp, &["-d", name, "--", &page])?;
        }
    }
    Ok(())
}
