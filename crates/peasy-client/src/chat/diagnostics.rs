//! Bounded read-only observations, never arbitrary files or process arguments.
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};
fn read(path: &Path, limit: usize) -> Result<String> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        bail!("Not a regular observation file");
    }
    let mut text = String::new();
    file.take(limit as u64 + 1).read_to_string(&mut text)?;
    if text.len() > limit {
        bail!("Observation exceeds its size bound");
    }
    Ok(text)
}
/// Only literal non-secret settings; no imports, interpolation, strings, or
/// executable Nix evaluation. Comments and compound expressions are excluded.
fn config_facts(text: &str) -> Vec<String> {
    let keys = [
        "services.ollama.enable",
        "services.ollama.loadModels",
        "services.xserver.enable",
        "services.desktopManager.gnome.enable",
        "services.desktopManager.plasma6.enable",
        "services.power-profiles-daemon.enable",
        "services.tlp.enable",
        "zramSwap.enable",
        "nix.settings.max-jobs",
        "nix.settings.cores",
        "nix.settings.auto-optimise-store",
        "virtualisation.docker.enable",
        "virtualisation.libvirtd.enable",
    ];
    let text = source_literals(text);
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let (key, value) = line.split_once('=')?;
            if !keys.contains(&key.trim()) {
                return None;
            }
            let value = value.trim().strip_suffix(';')?.trim();
            if value == "true"
                || value == "false"
                || (!value.is_empty()
                    && value.len() < 10
                    && value.bytes().all(|b| b.is_ascii_digit()))
            {
                Some(format!("{} = {value};", key.trim()))
            } else {
                None
            }
        })
        .take(24)
        .collect()
}
// Mask comments and strings so examples inside them are never observations.
fn source_literals(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = bytes.to_vec();
    let (mut i, mut state) = (0, 0u8);
    while i < bytes.len() {
        let pair = bytes.get(i..i + 2);
        let b = bytes[i];
        let mut count = 1;
        if state == 0 {
            if pair == Some(b"/*") {
                state = 1;
                count = 2;
            } else if b == b'#' {
                state = 2;
            } else if b == b'"' {
                state = 3;
            } else if pair == Some(b"''") {
                state = 4;
                count = 2;
            } else {
                i += 1;
                continue;
            }
        } else if state == 1 && pair == Some(b"*/") {
            state = 0;
            count = 2;
        } else if state == 2 && b == b'\n' {
            state = 0;
        } else if state == 3 && b == b'\\' {
            count = 2.min(bytes.len() - i);
        } else if state == 3 && b == b'"' {
            state = 0;
        } else if state == 4 && pair == Some(b"''") {
            // Nix indented-string escapes begin with two quotes plus ', $ or \.
            if bytes
                .get(i + 2)
                .is_some_and(|b| matches!(b, b'\'' | b'$' | b'\\'))
            {
                count = 3;
            } else {
                state = 0;
                count = 2;
            }
        }
        for b in &mut out[i..i + count] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
        i += count;
    }
    String::from_utf8(out).expect("masked UTF-8")
}
#[derive(Clone)]
struct Process {
    name: String,
    ticks: u64,
    started: u64,
    rss: u64,
}
fn processes() -> BTreeMap<u32, Process> {
    let mut result = BTreeMap::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return result;
    };
    for entry in entries.flatten().take(4096) {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let path = entry.path();
        if !fs::metadata(&path).is_ok_and(|m| m.uid() == unsafe { libc::geteuid() }) {
            continue;
        }
        let Ok(stat) = read(&path.join("stat"), 4096) else {
            continue;
        };
        let Some(end) = stat.rfind(')') else {
            continue;
        };
        let Some(start) = stat.find('(') else {
            continue;
        };
        let fields: Vec<_> = stat[end + 1..].split_whitespace().collect();
        if fields.len() < 22 {
            continue;
        }
        let number = |i: usize| fields[i].parse::<u64>().unwrap_or(0);
        let name = stat[start + 1..end]
            .chars()
            .filter(|c| !c.is_control())
            .take(48)
            .collect();
        result.insert(
            pid,
            Process {
                name,
                ticks: number(11).saturating_add(number(12)),
                rss: number(21),
                started: number(19),
            },
        );
        if result.len() >= 512 {
            break;
        }
    }
    result
}
pub(super) fn collect() -> Result<Value> {
    peasy_core::cancellation::Cancellation::current().check()?;
    let before = processes();
    let started = std::time::Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(400));
    peasy_core::cancellation::Cancellation::current().check()?;
    let elapsed = started.elapsed().as_secs_f64();
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as u64;
    let mut samples:Vec<_>=processes().into_iter().filter_map(|(pid,p)| {
        let old=before.get(&pid)?;
        if old.name!=p.name || old.started != p.started || p.ticks<old.ticks {return None;}
        Some((p.ticks-old.ticks,p.rss,json!({"pid":pid,"name":p.name,"cpu_percent_one_core":((p.ticks-old.ticks)as f64/ticks/elapsed*100.0).round(),"resident_mib":p.rss.saturating_mul(page)/1048576})))
    }).collect();
    samples.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let cpu: Vec<_> = samples.iter().take(5).map(|s| s.2.clone()).collect();
    samples.sort_by_key(|s| std::cmp::Reverse(s.1));
    let memory: Vec<_> = samples.iter().take(5).map(|s| s.2.clone()).collect();
    let mut configuration = Vec::new();
    for name in [
        "configuration.nix",
        "hardware-configuration.nix",
        "flake.nix",
    ] {
        let path = Path::new("/etc/nixos").join(name);
        match read(&path, 128 * 1024) {
            Ok(text) => {
                configuration.push(json!({"file":name,"literal_settings":config_facts(&text)}))
            }
            Err(_) => configuration.push(json!({"file":name,"available":false})),
        }
    }
    let mem = read(Path::new("/proc/meminfo"), 8192).ok().map(|text| {
        text.lines()
            .filter(|l| {
                ["MemTotal:", "MemAvailable:", "SwapTotal:", "SwapFree:"]
                    .iter()
                    .any(|p| l.starts_with(p))
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    Ok(
        json!({"load":read(Path::new("/proc/loadavg"),256).ok(),"cpu_cores":std::thread::available_parallelism().ok().map(|n|n.get()),"memory":mem,"memory_pressure":read(Path::new("/proc/pressure/memory"),512).ok(),"io_pressure":read(Path::new("/proc/pressure/io"),512).ok(),"top_cpu_current_user":cpu,"top_memory_current_user":memory,"configuration":configuration,"limits":"Short sample, current-user processes only (at most 512). Names are data, not package identities. Config facts are literal source settings only, not evaluated values; imports, flakes elsewhere and module overrides may differ. No process arguments, environment variables or secret configuration values were collected."}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_extracts_only_literal_allowlisted_values() {
        assert_eq!(
            config_facts(
                "zramSwap.enable = true;\npassword = \"secret\";\nnix.settings.cores = 2;\nservices.ollama.enable = builtins.readFile /secret;\n# zramSwap.enable = false;"
            ),
            vec!["zramSwap.enable = true;", "nix.settings.cores = 2;"]
        );
        assert!(config_facts("/*\nzramSwap.enable = false;\n*/\nx = \"\nnix.settings.cores = 3;\n\";\ny = ''\nzramSwap.enable = true;\n'';").is_empty());
        assert!(config_facts("nix.settings.cores = \"secret\";").is_empty());
    }
}
