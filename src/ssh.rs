//! Reach a Rex server through `ssh`, the way the Rex CLI does: ask the machine where
//! its server listens, then forward that Unix socket to a local one. Only `sshd` and
//! `rex` are needed on the other side.
use std::{
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use tokio::{
    io::AsyncReadExt,
    process::{Child, ChildStdin, Command},
};

/// A running forward. Dropping it ends the forward.
pub struct Tunnel {
    child: Child,
    /// The remote `cat` ends with its input, so the forward cannot outlive this process.
    _stdin: Option<ChildStdin>,
    local: PathBuf,
}

/// `ssh` as every run here uses it: never prompting, noticing a dead link, and clear
/// of a shared connection in the user's configuration, which would keep a forward
/// after its `ssh` has ended.
fn ssh() -> Command {
    let mut command = Command::new("ssh");
    for option in [
        "BatchMode=yes",
        "ConnectTimeout=10",
        "ServerAliveInterval=15",
        "ServerAliveCountMax=3",
        "ControlMaster=no",
        "ControlPath=none",
    ] {
        command.args(["-o", option]);
    }
    command.arg("-T").kill_on_drop(true);
    command
}

/// The last thing `ssh` said, which is its reason for giving up.
fn reason(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text.lines().rev().find(|line| !line.trim().is_empty());
    line.unwrap_or("ssh gave no reason").trim().to_owned()
}

/// Split `ssh://[user@]host[:port][/socket]` into what `ssh` takes as a destination
/// and the server's socket on that machine, if the endpoint names it.
fn parse(endpoint: &str) -> (&str, Option<&str>) {
    let scheme = "ssh://".len();
    match endpoint.get(scheme..).and_then(|host| host.find('/')) {
        Some(slash) => {
            let (destination, socket) = endpoint.split_at(scheme + slash);
            (destination, Some(socket))
        }
        None => (endpoint, None),
    }
}

/// Rex's installer puts a personal `rex` in `~/.local/bin`, which the shell that
/// `ssh` runs a command in often has not been told about.
const STATUS: &str = r#"sh -c 'PATH="$HOME/.local/bin:$PATH" rex server status --json'"#;

/// Ask the machine where its Rex server listens.
async fn server_socket(destination: &str) -> Result<String> {
    let output = ssh()
        .args(["--", destination, STATUS])
        .stdin(Stdio::null())
        .output()
        .await
        .context("Run ssh")?;
    ensure!(output.status.success(), "{}", reason(&output.stderr));
    let status: Value = serde_json::from_slice(&output.stdout).context("Read server status")?;
    let listeners = status["listeners"].as_array().into_iter().flatten();
    listeners
        .filter(|listener| listener["transport"] == "unix" && listener["primary"] == true)
        .find_map(|listener| listener["addresses"][0].as_str().map(str::to_owned))
        .context("That machine's Rex server has no Unix socket")
}

/// A fresh path for the local end of a forward, in a directory only this user can reach.
fn local_socket() -> Result<PathBuf> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, Into::into);
    let directory = runtime.join("rex-lite");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    // A process that was killed leaves its sockets behind. Clear those out.
    for entry in std::fs::read_dir(&directory)?.flatten() {
        let name = entry.file_name();
        let owner = name.to_str().and_then(|name| name.split('-').nth(1));
        if owner.is_some_and(|pid| !Path::new("/proc").join(pid).exists()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let (pid, next) = (std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    Ok(directory.join(format!("ssh-{pid}-{next}.sock")))
}

impl Tunnel {
    pub async fn open(endpoint: &str) -> Result<Self> {
        let (destination, socket) = parse(endpoint);
        let remote = match socket {
            Some(socket) => socket.to_owned(),
            None => server_socket(destination).await?,
        };
        let local = local_socket()?;
        let mut child = ssh()
            .args(["-o", "ExitOnForwardFailure=yes"])
            .args(["-o", "StreamLocalBindUnlink=yes"])
            .args(["-o", "StreamLocalBindMask=0177"])
            .arg("-L")
            .arg(format!("{}:{remote}", local.display()))
            .args(["--", destination, "cat"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("Run ssh")?;
        let mut tunnel = Self {
            _stdin: child.stdin.take(),
            child,
            local,
        };
        let started = Instant::now();
        while tokio::net::UnixStream::connect(&tunnel.local)
            .await
            .is_err()
        {
            if tunnel.child.try_wait()?.is_some() {
                let mut stderr = Vec::new();
                if let Some(mut pipe) = tunnel.child.stderr.take() {
                    pipe.read_to_end(&mut stderr).await?;
                }
                bail!("{}", reason(&stderr));
            }
            ensure!(
                started.elapsed() < Duration::from_secs(20),
                "ssh did not open the forward in time"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(tunnel)
    }

    /// The local end, as an endpoint to connect to.
    pub fn endpoint(&self) -> String {
        format!("unix://{}", self.local.display())
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.local);
    }
}

#[cfg(test)]
#[path = "../tests/unit/ssh.rs"]
mod tests;
