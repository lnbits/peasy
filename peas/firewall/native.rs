use crate::resource_native::*;
pub fn inspect(r: &dyn ResourceRunner) -> Result<Value> {
    // This view is augmented by the daemon with evaluated, structured NixOS
    // policy. Never dump raw nftables rules or claim to test external reachability.
    Ok(json!({"interfaces":json_run(r,Tool::Ip,&["-j","link","show"])?}))
}
