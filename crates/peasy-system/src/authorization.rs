use anyhow::{Context, Result, bail};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

pub const ACTION: &str = "io.github.peasy.apply";

#[derive(Clone, Copy, Debug)]
pub struct Peer {
    pub uid: u32,
    pub pid: i32,
    pub start_time: u64,
}

impl Peer {
    pub fn capture(uid: u32, pid: i32) -> Result<Self> {
        if pid <= 0 {
            bail!("invalid IPC peer process");
        }
        Ok(Self {
            uid,
            pid,
            start_time: process_start_time(pid)?,
        })
    }

    fn subject(&self) -> String {
        format!("{},{},{}", self.pid, self.start_time, self.uid)
    }
}

fn process_start_time(pid: i32) -> Result<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).context("IPC peer has exited")?;
    // comm can contain spaces and parentheses; field 22 is starttime.
    stat.rsplit_once(')')
        .and_then(|(_, fields)| fields.split_whitespace().nth(19))
        .context("invalid peer process metadata")?
        .parse()
        .context("invalid peer process start time")
}

pub trait Authorizer: Send + Sync {
    fn authorize(&self, peer: &Peer) -> Result<()>;
}

pub struct PolkitAuthorizer(pub PathBuf);

impl Authorizer for PolkitAuthorizer {
    fn authorize(&self, peer: &Peer) -> Result<()> {
        if !self.0.is_absolute() {
            bail!("trusted pkcheck executable must be absolute");
        }
        if process_start_time(peer.pid)? != peer.start_time {
            bail!("IPC peer process changed");
        }
        let mut command = Command::new(&self.0);
        command.env_clear().args([
            "--action-id",
            ACTION,
            "--process",
            &peer.subject(),
            "--allow-user-interaction",
        ]);
        let output = crate::process::run(command, Duration::from_secs(180))?;
        if !output.status.success() {
            bail!(
                "System change was not authorized. Authenticate through the desktop authorization dialog or run Peasy in an interactive terminal."
            );
        }
        if process_start_time(peer.pid)? != peer.start_time {
            bail!("IPC peer exited during authorization");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_binds_pid_start_time_and_uid() {
        let peer = Peer::capture(1234, std::process::id() as i32).unwrap();
        assert!(peer.start_time > 0);
        assert_eq!(
            peer.subject(),
            format!("{},{},1234", std::process::id(), peer.start_time)
        );
        assert!(Peer::capture(1234, -1).is_err());
    }
}
