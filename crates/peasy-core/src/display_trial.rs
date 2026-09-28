//! Display-only watchdog protocol. The worker owns restoration independently of
//! the UI/CLI lifetime; only an explicit keep message commits a trial.
use crate::{
    ResourceChange,
    resource_native::{self, ResourceRunner, SessionRunner},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{ChildStdin, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

pub const CONFIRM_SECONDS: u64 = 20;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    change: ResourceChange,
    snapshot: Value,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Reply {
    Ready,
    Finished { kept: bool },
    Error { message: String },
}

pub struct DisplayTrial {
    input: Option<ChildStdin>,
    replies: mpsc::Receiver<Reply>,
    deadline: Instant,
}
impl DisplayTrial {
    pub fn start(change: &ResourceChange, snapshot: &Value) -> Result<Self> {
        use std::os::unix::process::CommandExt;
        if !matches!(change, ResourceChange::Display { .. }) {
            bail!("not a display change");
        }
        let path = std::env::var_os("PEASY_DISPLAY_GUARD")
            .map(std::path::PathBuf::from)
            .unwrap_or(std::env::current_exe()?.with_file_name("peasy-display-guard"));
        if !path.is_absolute() {
            bail!("display guard path must be absolute");
        }
        let mut child = Command::new(path)
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("start display safety worker")?;
        let mut input = child.stdin.take().context("missing worker input")?;
        let output = child.stdout.take().context("missing worker output")?;
        let (tx, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else {
                    break;
                };
                if line.len() > 8192 {
                    break;
                }
                let Ok(reply) = serde_json::from_str(&line) else {
                    break;
                };
                if tx.send(reply).is_err() {
                    break;
                }
            }
            let _ = child.wait();
        });
        serde_json::to_writer(
            &mut input,
            &Request {
                change: change.clone(),
                snapshot: snapshot.clone(),
            },
        )?;
        writeln!(input)?;
        input.flush()?;
        match replies.recv_timeout(Duration::from_secs(250))? {
            Reply::Ready => Ok(Self {
                input: Some(input),
                replies,
                deadline: Instant::now() + Duration::from_secs(CONFIRM_SECONDS),
            }),
            Reply::Error { message } => bail!("{message}"),
            _ => bail!("display safety worker did not start a trial"),
        }
    }
    pub fn seconds_remaining(&self) -> u64 {
        self.deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .div_ceil(1000) as u64
    }
    pub fn finish(mut self, keep: bool) -> Result<String> {
        if keep && let Some(mut input) = self.input.take() {
            // A timeout can race the click. The worker's final reply decides
            // whether settings were kept, including when its pipe has closed.
            let _ = writeln!(input, "keep").and_then(|()| input.flush());
        }
        self.input.take(); // EOF also requests immediate restoration.
        match self.replies.recv_timeout(Duration::from_secs(250))? {
            Reply::Finished { kept } => Ok(if kept {
                "Display settings kept."
            } else {
                "Previous display settings restored."
            }
            .into()),
            Reply::Error { message } => bail!("{message}"),
            Reply::Ready => bail!("unexpected display worker reply"),
        }
    }
}

fn send(reply: Reply) -> Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, &reply)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}
fn run_trial(
    change: &ResourceChange,
    snapshot: &Value,
    r: &dyn ResourceRunner,
    confirm: impl FnOnce() -> bool,
) -> Result<bool> {
    // Revalidation happens before any effect; restoration is needed even when
    // the compositor reports a failure after partially applying a layout.
    resource_native::prepare_display(change, snapshot, r)?;
    if let Err(error) = resource_native::set_display(change, snapshot, r) {
        resource_native::restore_display(change, snapshot, r).context(format!(
            "display apply failed ({error:#}); restoration also failed"
        ))?;
        return Err(error);
    }
    if confirm() {
        return Ok(true);
    }
    resource_native::restore_display(change, snapshot, r)
        .context("could not restore previous display settings; use desktop settings")?;
    Ok(false)
}
fn worker_inner() -> Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .context("display trials require a user runtime directory")?;
    let path = std::path::PathBuf::from(runtime).join("peasy-display-trial.lock");
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(path)?;
    if lock.metadata()?.uid() != nix::unistd::getuid().as_raw() {
        bail!("invalid display lock owner");
    }
    lock.try_lock()
        .context("another display trial is already active")?;
    let mut input = BufReader::new(std::io::stdin());
    let mut request = String::new();
    input.by_ref().take(64 * 1024 + 1).read_line(&mut request)?;
    if request.len() > 64 * 1024 {
        bail!("display request too large");
    }
    let request: Request = serde_json::from_str(&request)?;
    let kept = run_trial(&request.change, &request.snapshot, &SessionRunner, || {
        if send(Reply::Ready).is_err() {
            return false;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let keep = input.take(16).read_line(&mut line).is_ok() && line == "keep\n";
            let _ = tx.send(keep);
        });
        rx.recv_timeout(Duration::from_secs(CONFIRM_SECONDS))
            .unwrap_or(false)
    })?;
    send(Reply::Finished { kept })
}
pub fn worker() {
    if let Err(error) = worker_inner() {
        let _ = send(Reply::Error {
            message: format!("{error:#}"),
        });
    }
}
