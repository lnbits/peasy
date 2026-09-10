//! Bounded child execution. Limits apply to the entire process group, including
//! Nix evaluation descendants, and readers never accumulate unlimited output.
use anyhow::{Context, Result, bail};
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

pub fn run(command: &mut Command, timeout: Duration) -> Result<Output> {
    let cancellation = crate::cancellation::Cancellation::current();
    cancellation.check()?;
    let mut child = command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting trusted executable")?;
    let overflow = Arc::new(AtomicBool::new(false));
    let reader = |stream: Box<dyn Read + Send>, limit: usize, drain_excess: bool| {
        let overflow = Arc::clone(&overflow);
        let sink = crate::progress::current();
        thread::spawn(move || {
            if drain_excess {
                // Drain verbose warnings without blocking a successful build;
                // retain the final cause following Nix's evaluation trace.
                return read_tail(
                    ProgressReader {
                        stream,
                        sink,
                        pending: Vec::new(),
                        discarding: false,
                    },
                    limit,
                );
            }
            let mut bytes = Vec::new();
            let result = stream.take(limit as u64 + 1).read_to_end(&mut bytes);
            if bytes.len() > limit {
                overflow.store(true, Ordering::Release);
            }
            result.map(|_| bytes)
        })
    };
    let stdout = reader(
        Box::new(child.stdout.take().context("missing stdout")?),
        32 * 1024 * 1024,
        false,
    );
    let stderr = reader(
        Box::new(child.stderr.take().context("missing stderr")?),
        2 * 1024 * 1024,
        true,
    );
    let started = Instant::now();
    let result = loop {
        if let Err(error) = cancellation.check() {
            break Err(error.into());
        }
        if overflow.load(Ordering::Acquire) || started.elapsed() > timeout {
            break Err(anyhow::anyhow!(
                "trusted command exceeded its time or output limit"
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) if stdout.is_finished() && stderr.is_finished() => break Ok(status),
            Ok(_) => thread::sleep(Duration::from_millis(25)),
            Err(error) => break Err(error.into()),
        }
    };
    if result.is_err() {
        // Child and descendants were assigned this group before exec. Never
        // target the daemon's own group.
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(child.id() as i32),
            nix::sys::signal::Signal::SIGKILL,
        );
        let _ = child.kill();
    }
    let _ = child.wait();
    let out = stdout
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader failed"))??;
    let err = stderr
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader failed"))??;
    let status = result?;
    if overflow.load(Ordering::Acquire) {
        bail!("trusted command exceeded its output limit");
    }
    Ok(Output {
        status,
        stdout: out,
        stderr: err,
    })
}

struct ProgressReader {
    stream: Box<dyn Read + Send>,
    sink: Option<crate::progress::Sink>,
    pending: Vec<u8>,
    discarding: bool,
}
impl Read for ProgressReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.stream.read(buf)?;
        if let Some(sink) = &self.sink {
            for b in &buf[..n] {
                if *b == b'\n' {
                    if !self.discarding
                        && let Some(stage) = crate::progress::nix_stage(&self.pending)
                    {
                        sink(stage);
                    }
                    self.pending.clear();
                    self.discarding = false;
                } else if self.pending.len() < 65536 && !self.discarding {
                    self.pending.push(*b);
                } else {
                    self.pending.clear();
                    self.discarding = true;
                }
            }
        }
        Ok(n)
    }
}

fn read_tail(mut stream: impl Read, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut bytes = vec![0; limit];
    let mut next = 0;
    let mut full = false;
    loop {
        let count = stream.read(&mut bytes[next..])?;
        if count == 0 {
            break;
        }
        next += count;
        if next == limit {
            next = 0;
            full = true;
        }
    }
    if full {
        bytes.rotate_left(next);
    } else {
        bytes.truncate(next);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_reader_handles_small_reads_and_discards_oversized_records() {
        let stages = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = stages.clone();
        let mut bytes = vec![b'x'; 70000];
        bytes.extend_from_slice(b"\n@nix {\"action\":\"start\",\"type\":105}\n");
        let mut reader = ProgressReader {
            stream: Box::new(std::io::Cursor::new(bytes)),
            sink: Some(Arc::new(move |s| observed.lock().unwrap().push(s))),
            pending: Vec::new(),
            discarding: false,
        };
        let mut small = [0; 7];
        while reader.read(&mut small).unwrap() != 0 {}
        assert_eq!(*stages.lock().unwrap(), [crate::OperationStage::Building]);
        assert!(reader.pending.is_empty());
    }

    #[test]
    fn cancellation_terminates_child_and_pipe_holding_descendants() {
        let temp = tempfile::tempdir().unwrap();
        let ready = temp.path().join("ready");
        let ready_worker = ready.clone();
        let token = crate::cancellation::Cancellation::default();
        let worker_token = token.clone();
        let worker = thread::spawn(move || {
            worker_token.scope(|| {
                let mut command = Command::new("/bin/sh");
                command
                    .args(["-c", "sleep 30 & echo ready > \"$1\"; wait", "test"])
                    .arg(ready_worker);
                run(&mut command, Duration::from_secs(10))
            })
        });
        let start = Instant::now();
        while !ready.exists() && start.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(ready.exists());
        token.cancel();
        assert!(
            worker
                .join()
                .unwrap()
                .unwrap_err()
                .downcast_ref::<crate::cancellation::Cancelled>()
                .is_some()
        );
        assert!(start.elapsed() < Duration::from_secs(8));
    }

    #[test]
    fn precancelled_command_never_starts() {
        let token = crate::cancellation::Cancellation::default();
        token.cancel();
        let error = token
            .scope(|| run(&mut Command::new("/nonexistent"), Duration::from_secs(1)))
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::cancellation::Cancelled>()
                .is_some()
        );
    }
    #[test]
    fn stuck_child_is_terminated() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "while :; do :; done"]);
        let start = Instant::now();
        assert!(run(&mut command, Duration::from_millis(100)).is_err());
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn verbose_stderr_is_bounded_without_failing_a_successful_command() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "head -c 3145728 /dev/zero >&2"]);
        let output = run(&mut command, Duration::from_secs(10)).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stderr.len(), 2 * 1024 * 1024);
        assert_eq!(read_tail(&b"0123456789"[..], 4).unwrap(), b"6789");
        assert_eq!(read_tail(&b"ab"[..], 4).unwrap(), b"ab");
        assert_eq!(read_tail(&b"abcd"[..], 4).unwrap(), b"abcd");
    }
}
