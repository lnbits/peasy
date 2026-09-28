use crate::resource_native::*;
pub fn inspect(r: &dyn ResourceRunner) -> Result<Value> {
    Ok(
        json!({"generations":r.run(Tool::NixEnv,&["--profile","/nix/var/nix/profiles/system","--list-generations"] )?,"current_system":std::fs::read_link("/run/current-system").ok(),"booted_system":std::fs::read_link("/run/booted-system").ok(),"store_filesystem":r.run(Tool::Df,&["-B1","--output=size,used,avail,pcent","/nix/store"] )?,"limits":"Filesystem usage includes other files when the store shares its filesystem. Removing generation references and garbage collection are separate reviewed operations."}),
    )
}
pub fn snapshot(generations: &[u64], r: &dyn ResourceRunner) -> Result<Value> {
    let state = inspect(r)?;
    let listing = state["generations"]
        .as_str()
        .context("missing generations")?;
    let available = listing
        .lines()
        .filter_map(|l| l.split_whitespace().next()?.parse::<u64>().ok())
        .collect::<Vec<_>>();
    if generations.iter().any(|g| !available.contains(g)) {
        bail!("a selected generation no longer exists");
    }
    let current = std::fs::canonicalize("/nix/var/nix/profiles/system")?;
    let booted = std::fs::canonicalize("/run/booted-system")?;
    let active = std::fs::canonicalize("/run/current-system")?;
    let mut selected = Vec::new();
    for g in generations {
        let path = std::fs::canonicalize(format!("/nix/var/nix/profiles/system-{g}-link"))?;
        if path == current || path == booted || path == active {
            bail!("current, active and booted generations are protected");
        }
        selected.push(json!({"generation":g,"system":path}));
    }
    let mut retained = Vec::new();
    let mut rollback = false;
    for g in available.iter().filter(|g| !generations.contains(g)) {
        let path = std::fs::canonicalize(format!("/nix/var/nix/profiles/system-{g}-link"))?;
        if path != current {
            rollback = true;
        }
        retained.push(json!({"generation":g,"system":path}));
    }
    if !rollback {
        bail!("retain at least one distinct rollback generation");
    }
    Ok(
        json!({"selected":selected,"retained":retained,"profile":current,"active":active,"booted":booted}),
    )
}
pub fn remove(generations: &[u64], r: &dyn ResourceRunner) -> Result<()> {
    let ids = generations.iter().map(u64::to_string).collect::<Vec<_>>();
    let mut args = vec![
        "--profile",
        "/nix/var/nix/profiles/system",
        "--delete-generations",
    ];
    args.extend(ids.iter().map(String::as_str));
    r.run(Tool::NixEnv, &args)?;
    Ok(())
}
