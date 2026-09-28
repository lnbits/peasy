//! Exercise the real watchdog process and its pipe/timeout contract against a
//! deterministic compositor fixture, including client disconnect and failures.
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn fixture() -> (tempfile::TempDir, Value) {
    let dir = tempfile::tempdir().unwrap();
    let outputs = json!([{"name":"DP-1","description":"Fixture","serial":"test","width":1920,"height":1080,"refreshRate":60.0,"x":0,"y":0,"scale":1.0,"disabled":false,"transform":0,"availableModes":["1920x1080@60Hz"]}]);
    std::fs::write(dir.path().join("outputs"), outputs.to_string()).unwrap();
    let shell = Command::new("sh")
        .args(["-c", "command -v sh"])
        .output()
        .unwrap();
    let script = format!(
        "#!{}\nif [ \"$1\" = -j ]; then cat \"$DISPLAY_FIXTURE/outputs\"; exit; fi\nprintf '%s\\n' \"$*\" >> \"$DISPLAY_FIXTURE/calls\"\nif [ -f \"$DISPLAY_FIXTURE/fail-once\" ]; then rm \"$DISPLAY_FIXTURE/fail-once\"; exit 1; fi\n",
        String::from_utf8(shell.stdout).unwrap().trim()
    );
    let tool = dir.path().join("hyprctl");
    std::fs::write(&tool, script).unwrap();
    std::fs::set_permissions(tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    (dir, json!({"backend":"hyprland","outputs":outputs}))
}
fn start(dir: &tempfile::TempDir, snapshot: Value) -> std::process::Child {
    let mut child = Command::new(env!("CARGO_BIN_EXE_peasy-display-guard"))
        .env("XDG_RUNTIME_DIR", dir.path())
        .env("XDG_CURRENT_DESKTOP", "Hyprland")
        .env("DISPLAY_FIXTURE", dir.path())
        .env("PEASY_HYPRCTL", dir.path().join("hyprctl"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let request = json!({"change":{"operation":"display","connector":"DP-1","mode":"1920x1080@60","scale_percent":150,"x":-1920,"y":0,"primary":false},"snapshot":snapshot});
    writeln!(child.stdin.as_mut().unwrap(), "{request}").unwrap();
    child
}
fn wait(child: &mut std::process::Child) {
    let end = Instant::now() + Duration::from_secs(35);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > end {
            child.kill().unwrap();
            panic!("watchdog did not finish");
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}
#[test]
fn keep_revert_disconnect_timeout_and_partial_failure() {
    for action in ["keep", "revert", "disconnect", "timeout", "failure"] {
        let (dir, snapshot) = fixture();
        if action == "failure" {
            std::fs::write(dir.path().join("fail-once"), "").unwrap();
        }
        let mut child = start(&dir, snapshot.clone());
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        if action == "failure" {
            assert_eq!(reply["status"], "error");
        } else {
            assert_eq!(reply["status"], "ready");
            // A second process cannot overlap the first trial.
            let second = start(&dir, snapshot);
            let response = second.wait_with_output().unwrap();
            assert!(String::from_utf8_lossy(&response.stdout).contains("already active"));
            match action {
                "keep" => writeln!(child.stdin.as_mut().unwrap(), "keep").unwrap(),
                "revert" => writeln!(child.stdin.as_mut().unwrap(), "revert").unwrap(),
                "disconnect" => {
                    child.stdin.take();
                }
                _ => {}
            }
            wait(&mut child);
            line.clear();
            output.read_line(&mut line).unwrap();
            let reply: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(reply["status"], "finished", "{reply}");
            assert_eq!(reply["kept"], action == "keep");
        }
        wait(&mut child);
        let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
        assert!(
            calls.starts_with("keyword monitor DP-1,1920x1080@60,-1920x0,1.5,transform,0\n"),
            "{calls}"
        );
        assert_eq!(calls.lines().count(), if action == "keep" { 1 } else { 2 });
        if action != "keep" {
            assert!(calls.ends_with("keyword monitor DP-1,1920x1080@60,0x0,1,transform,0\n"));
        }
    }
}
#[test]
fn stale_snapshot_never_mutates_or_restores() {
    let (dir, mut snapshot) = fixture();
    snapshot["outputs"][0]["serial"] = json!("replaced");
    let child = start(&dir, snapshot);
    let output = child.wait_with_output().unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("changed"));
    assert!(!dir.path().join("calls").exists());
}
