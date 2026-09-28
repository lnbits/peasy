use crate::resource_native::*;
pub fn inspect() -> Result<Value> {
    let passwd = read_bounded("/etc/passwd")?;
    let users = passwd
        .lines()
        .filter_map(|l| {
            let f = l.split(':').collect::<Vec<_>>();
            if f.len() != 7 {
                return None;
            }
            let uid = f[2].parse::<u32>().ok()?;
            if !(1000..65534).contains(&uid) {
                return None;
            }
            Some(json!({"name":f[0],"uid":uid,"shell":f[6]}))
        })
        .take(64)
        .collect::<Vec<_>>();
    let groups = read_bounded("/etc/group")?;
    let groups = groups
        .lines()
        .filter_map(|l| {
            let f = l.split(':').collect::<Vec<_>>();
            if f.len() != 4 {
                return None;
            }
            let members = f[3]
                .split(',')
                .filter(|n| users.iter().any(|u| u["name"] == *n))
                .collect::<Vec<_>>();
            if members.is_empty() {
                None
            } else {
                Some(json!({"group":f[0],"members":members}))
            }
        })
        .take(128)
        .collect::<Vec<_>>();
    Ok(
        json!({"users":users,"supplementary_groups":groups,"limits":"Local normal accounts only. Password hashes, home contents and authentication secrets are never read."}),
    )
}
