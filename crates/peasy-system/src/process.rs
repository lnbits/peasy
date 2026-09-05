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

pub fn run(mut command: Command, timeout: Duration) -> Result<Output> {
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
        thread::spawn(move || {
            if drain_excess {
                // Drain verbose warnings without blocking a successful build;
                // retain the final cause following Nix's evaluation trace.
                return read_tail(stream, limit);
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
    fn stuck_child_is_terminated() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "while :; do :; done"]);
        let start = Instant::now();
        assert!(run(command, Duration::from_millis(100)).is_err());
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn verbose_stderr_is_bounded_without_failing_a_successful_command() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "head -c 3145728 /dev/zero >&2"]);
        let output = run(command, Duration::from_secs(10)).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stderr.len(), 2 * 1024 * 1024);
        assert_eq!(read_tail(&b"0123456789"[..], 4).unwrap(), b"6789");
        assert_eq!(read_tail(&b"ab"[..], 4).unwrap(), b"ab");
        assert_eq!(read_tail(&b"abcd"[..], 4).unwrap(), b"abcd");
    }
}
