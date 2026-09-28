use crate::resource_native::*;
fn flatten(
    nodes: &[Value],
    removable: bool,
    protected: bool,
    serial: Option<&str>,
    result: &mut Vec<Value>,
) {
    for node in nodes {
        let children = node["children"].as_array().cloned().unwrap_or_default();
        let removable = removable || node["rm"] == true || node["tran"] == "usb";
        let protected = protected || tree_protected(node);
        let serial = node["serial"].as_str().or(serial);
        result.push(json!({"device":node["name"],"kind":node["type"],"removable":removable,"protected":protected,"read_only":node["ro"],"bytes":node["size"],"filesystem":node["fstype"],"uuid":node["uuid"],"mounts":node["mountpoints"],"device_number":node["maj:min"],"serial":serial,"children":children.len()}));
        flatten(&children, removable, protected, serial, result);
    }
}
fn tree_protected(node: &Value) -> bool {
    node["fstype"] == "swap"
        || ["crypt", "lvm", "raid0", "raid1"]
            .iter()
            .any(|kind| node["type"] == *kind)
        || node["mountpoints"].as_array().is_some_and(|ms| {
            ms.iter().filter_map(Value::as_str).any(|m| {
                !(m.starts_with("/run/media/")
                    || m.starts_with("/media/")
                    || m.starts_with("/mnt/"))
            })
        })
        || node["children"]
            .as_array()
            .is_some_and(|cs| cs.iter().any(tree_protected))
}
pub fn inspect(r: &dyn ResourceRunner) -> Result<Value> {
    let value = json_run(
        r,
        Tool::Lsblk,
        &[
            "--json",
            "--bytes",
            "--paths",
            "--output",
            "NAME,TYPE,RM,RO,SIZE,FSTYPE,UUID,MOUNTPOINTS,TRAN,MAJ:MIN,SERIAL",
        ],
    )?;
    let mut devices = vec![];
    flatten(
        value["blockdevices"]
            .as_array()
            .context("invalid block device list")?,
        false,
        false,
        None,
        &mut devices,
    );
    if devices.len() > 128 {
        bail!("too many block devices");
    }
    Ok(json!({"devices":devices}))
}
fn object_path(device: &str) -> String {
    let name = device.strip_prefix("/dev/").expect("validated device");
    format!(
        "/org/freedesktop/UDisks2/block_devices/{}",
        name.bytes()
            .map(|b| if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("_{b:02x}")
            })
            .collect::<String>()
    )
}
pub fn snapshot(change: &ResourceChange, r: &dyn ResourceRunner) -> Result<Value> {
    if matches!(
        change,
        ResourceChange::PersistentMount { present: false, .. }
    ) {
        return Ok(json!({}));
    }
    let state = inspect(r)?;
    let devices = state["devices"].as_array().context("missing devices")?;
    let selected = devices
        .iter()
        .find(|d| match change {
            ResourceChange::Disk { device, .. } => d["device"] == *device,
            ResourceChange::PersistentMount { uuid, .. } => d["uuid"] == *uuid,
            _ => false,
        })
        .context("selected filesystem is no longer present")?;
    if selected["removable"] != true
        || selected["protected"] != false
        || selected["read_only"] != false
        || selected["children"] != 0
        || !["part", "disk"].iter().any(|k| selected["kind"] == *k)
    {
        bail!("only writable leaf filesystems on removable non-system disks are supported");
    }
    let mounts = selected["mounts"]
        .as_array()
        .map(|m| m.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    if let ResourceChange::PersistentMount { filesystem, .. } = change
        && selected["filesystem"] != filesystem.value()
    {
        bail!("filesystem type does not match the discovered UUID");
    }
    if let ResourceChange::Disk { device, action, .. } = change {
        let system = r.run(
            Tool::Busctl,
            &[
                "--system",
                "get-property",
                "org.freedesktop.UDisks2",
                &object_path(device),
                "org.freedesktop.UDisks2.Block",
                "HintSystem",
            ],
        )?;
        if system.trim() != "b false" {
            bail!("UDisks identifies this as a system device");
        }
        match action {
            DiskAction::Format | DiskAction::Mount if !mounts.is_empty() => {
                bail!("unmount the selected filesystem before this operation")
            }
            DiskAction::Unmount if mounts.is_empty() => bail!("selected filesystem is not mounted"),
            _ => {}
        }
    }
    Ok(selected.clone())
}
pub fn apply(change: &ResourceChange, r: &dyn ResourceRunner) -> Result<()> {
    let ResourceChange::Disk {
        device,
        action,
        filesystem,
    } = change
    else {
        bail!("not a disk operation");
    };
    match action {
        DiskAction::Mount | DiskAction::Unmount => {
            r.run(Tool::Udisksctl, &[action.value(), "--block-device", device])?;
        }
        DiskAction::Format => {
            r.run(
                Tool::Busctl,
                &[
                    "--system",
                    "--timeout=120",
                    "call",
                    "org.freedesktop.UDisks2",
                    &object_path(device),
                    "org.freedesktop.UDisks2.Block",
                    "Format",
                    "sa{sv}",
                    filesystem.context("filesystem required")?.value(),
                    "1",
                    "take-ownership",
                    "b",
                    "true",
                ],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_partition_protects_the_entire_disk() {
        let tree = json!({"name":"/dev/sda","rm":true,"children":[{"name":"/dev/sda1","mountpoints":["/boot"]},{"name":"/dev/sda2","mountpoints":[null]}]});
        let mut out = vec![];
        flatten(&[tree], false, false, None, &mut out);
        assert!(out.iter().all(|d| d["protected"] == true));
    }

    #[test]
    fn partitions_retain_the_physical_disk_identity() {
        let tree = json!({"name":"/dev/sdb","serial":"usb123","children":[{"name":"/dev/sdb1","serial":null}]});
        let mut out = vec![];
        flatten(&[tree], false, false, None, &mut out);
        assert_eq!(out[1]["serial"], "usb123");
    }
}
