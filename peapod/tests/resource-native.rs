use super::*;
use std::cell::RefCell;
use std::collections::VecDeque;
struct Mock {
    replies: RefCell<VecDeque<(Tool, String)>>,
    calls: RefCell<Vec<(Tool, Vec<String>)>>,
}
impl Mock {
    fn new(replies: Vec<(Tool, Value)>) -> Self {
        Self {
            replies: RefCell::new(
                replies
                    .into_iter()
                    .map(|(t, v)| {
                        (
                            t,
                            v.as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| v.to_string()),
                        )
                    })
                    .collect(),
            ),
            calls: RefCell::new(vec![]),
        }
    }
}
impl ResourceRunner for Mock {
    fn run(&self, tool: Tool, args: &[&str]) -> Result<String> {
        self.calls
            .borrow_mut()
            .push((tool, args.iter().map(|s| s.to_string()).collect()));
        let (expected, out) = self
            .replies
            .borrow_mut()
            .pop_front()
            .expect("unexpected command");
        assert_eq!(tool, expected);
        Ok(out)
    }
}
fn block(uuid: &str, system: bool) -> Value {
    json!({"blockdevices":[{"name":"/dev/sdb","type":"disk","rm":true,"ro":false,"size":100000,"fstype":"ext4","uuid":uuid,"mountpoints":if system {vec![Some("/")]}else{vec![None]},"tran":"usb","maj:min":"8:16","serial":"device123"}]})
}
#[test]
fn destructive_disk_operations_reject_system_devices_and_replacement() {
    let change = ResourceChange::Disk {
        device: "/dev/sdb".into(),
        action: DiskAction::Format,
        filesystem: Some(FileSystem::Ext4),
    };
    let protected = Mock::new(vec![(Tool::Lsblk, block("one", true))]);
    assert!(snapshot(&change, &protected).is_err());
    assert_eq!(protected.calls.borrow().len(), 1);
    let original = Mock::new(vec![
        (Tool::Lsblk, block("one", false)),
        (Tool::Busctl, json!("b false")),
    ]);
    let reviewed = snapshot(&change, &original).unwrap();
    let replaced = Mock::new(vec![
        (Tool::Lsblk, block("two", false)),
        (Tool::Busctl, json!("b false")),
    ]);
    assert!(
        apply_live(&change, &reviewed, &replaced)
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    assert_eq!(replaced.calls.borrow().len(), 2);
}
#[test]
fn disk_format_uses_only_the_fixed_udisks_method() {
    let change = ResourceChange::Disk {
        device: "/dev/sdb".into(),
        action: DiskAction::Format,
        filesystem: Some(FileSystem::Exfat),
    };
    let mock = Mock::new(vec![
        (Tool::Lsblk, block("one", false)),
        (Tool::Busctl, json!("b false")),
        (Tool::Lsblk, block("one", false)),
        (Tool::Busctl, json!("b false")),
        (Tool::Busctl, json!("")),
    ]);
    let reviewed = snapshot(&change, &mock).unwrap();
    crate::cancellation::Cancellation::default()
        .scope(|| apply_live(&change, &reviewed, &mock))
        .unwrap();
    let calls = mock.calls.borrow();
    let last = &calls.last().unwrap().1;
    assert!(last.contains(&"org.freedesktop.UDisks2.Block".into()));
    assert!(last.contains(&"Format".into()));
    assert!(last.contains(&"exfat".into()));
    assert!(!last.contains(&"tear-down".into()));
}
#[test]
fn service_identity_and_infrastructure_are_checked_before_control() {
    let r = Mock::new(vec![]);
    assert!(services::snapshot("peasy-system.service", &r).is_err());
    assert!(r.calls.borrow().is_empty());
    let change = ResourceChange::Service {
        unit: "caddy.service".into(),
        action: ServiceAction::Restart,
    };
    let old = "Id=caddy.service\nLoadState=loaded\nActiveState=active\nInvocationID=one\nFragmentPath=/nix/store/one/caddy.service\n";
    let new = old.replace("InvocationID=one", "InvocationID=two");
    let r = Mock::new(vec![
        (Tool::Systemctl, json!(old)),
        (Tool::Systemctl, json!(new)),
    ]);
    let reviewed = snapshot(&change, &r).unwrap();
    assert!(apply_live(&change, &reviewed, &r).is_err());
    assert_eq!(r.calls.borrow().len(), 2);
}
#[test]
fn audio_rejects_reused_ids_and_accepts_fixed_volume_control() {
    let node = |serial| json!([{"id":42,"info":{"props":{"object.serial":serial,"node.name":"speaker","media.class":"Audio/Sink"}}}]);
    let change = ResourceChange::Audio {
        id: 42,
        action: AudioAction::Volume,
        volume: Some(50),
    };
    let r = Mock::new(vec![(Tool::PwDump, node(10)), (Tool::PwDump, node(11))]);
    let reviewed = snapshot(&change, &r).unwrap();
    assert!(apply_live(&change, &reviewed, &r).is_err());
    let r = Mock::new(vec![(Tool::PwDump, node(10)), (Tool::Wpctl, json!(""))]);
    crate::cancellation::Cancellation::default()
        .scope(|| apply_live(&change, &reviewed, &r))
        .unwrap();
    assert_eq!(
        r.calls.borrow().last().unwrap().1,
        ["set-volume", "42", "50%"]
    );
}
#[test]
fn printer_add_cannot_replace_existing_queues_or_invent_uris() {
    let change = ResourceChange::Printer {
        name: "office".into(),
        action: PrinterAction::Add,
        uri: Some("ipp://printer.local/ipp/print".into()),
    };
    let r = Mock::new(vec![
        (
            Tool::Lpstat,
            json!("device for office: ipp://printer.local/ipp/print\n"),
        ),
        (Tool::Lpinfo, json!("network ipp://printer.local/ipp/print")),
        (Tool::Ippfind, json!("")),
        (Tool::Lpstat, json!("system default destination: office")),
    ]);
    assert!(snapshot(&change, &r).is_err());
    let r = Mock::new(vec![
        (Tool::Lpstat, json!("")),
        (
            Tool::Lpinfo,
            json!("network ipp://different.local/ipp/print"),
        ),
        (Tool::Ippfind, json!("")),
        (Tool::Lpstat, json!("")),
    ]);
    assert!(snapshot(&change, &r).is_err());
}
#[test]
fn power_profile_requires_an_advertised_profile() {
    let r = Mock::new(vec![(
        Tool::Powerprofilesctl,
        json!("* balanced:\n  power-saver:\n"),
    )]);
    assert!(
        snapshot(
            &ResourceChange::PowerProfile {
                profile: PowerProfile::Performance
            },
            &r
        )
        .is_err()
    );
}
