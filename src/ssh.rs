//! Reach a Rex server through `ssh`, the way the Rex CLI does: ask the machine where
//! its server listens, then forward that Unix socket to a local one. Only `sshd` and
//! `rex` are needed on the other side.
use std::{
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
    process::Stdio,
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, ChildStdin, Command},
};

use crate::wire::{self, Target};

/// How long the other machine may take to say where its server is, and again to
/// open the forward.
const PATIENCE: Duration = Duration::from_secs(20);
/// How much of what `ssh` says is kept: enough for its last line.
const KEPT: usize = 2048;

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

/// The server's socket on its own machine, from the status that machine's `rex` printed.
fn remote_socket(status: &Value) -> Option<String> {
    // Every `rex` names the endpoint it found the server at. Only the newer ones list
    // all it listens on, which is the place to look when that endpoint is no socket.
    let named = status["endpoint"].as_str().and_then(|endpoint| {
        match wire::targets(endpoint).ok()?.as_slice() {
            [Target::Unix(path)] if path.is_absolute() => Some(path.to_str()?.to_owned()),
            _ => None,
        }
    });
    named.or_else(|| {
        let listeners = status["listeners"].as_array().into_iter().flatten();
        listeners
            .filter(|listener| listener["transport"] == "unix" && listener["primary"] == true)
            .find_map(|listener| listener["addresses"][0].as_str().map(str::to_owned))
    })
}

/// Ask the machine where its Rex server listens.
async fn server_socket(destination: &str) -> Result<String> {
    let mut status = ssh();
    status.args(["--", destination, STATUS]).stdin(Stdio::null());
    // `ssh` gives up on a machine that does not answer, but not on a login or a
    // command that hangs. Given up on here, it is killed.
    let output = tokio::time::timeout(PATIENCE, status.output()).await;
    let output = output
        .context("That machine did not say in time where its Rex server is")?
        .context("Run ssh")?;
    ensure!(output.status.success(), "{}", reason(&output.stderr));
    let status: Value = serde_json::from_slice(&output.stdout).context("Read server status")?;
    remote_socket(&status).context("That machine's Rex server has no Unix socket")
}

/// Read what `ssh` says for as long as it runs, so that it never waits on a full
/// pipe, and keep the last of it.
async fn drain(mut pipe: impl AsyncRead + Unpin) -> Vec<u8> {
    let (mut said, mut chunk) = (Vec::new(), [0; 1024]);
    while let Ok(read @ 1..) = pipe.read(&mut chunk).await {
        said.extend_from_slice(&chunk[..read]);
        let old = said.len().saturating_sub(KEPT);
        said.drain(..old);
    }
    said
}

/// Make a directory that only `user` can reach, or be sure that the one already
/// there is such. In a place others can write to, one of them may have made it first.
fn private(directory: &Path, user: u32) -> Result<()> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(directory)?;
    // Asked of the name itself: a link to a directory is not one.
    let found = std::fs::symlink_metadata(directory)?;
    ensure!(
        found.is_dir() && found.uid() == user,
        "{} is not a directory of this user's own",
        directory.display()
    );
    Ok(())
}

/// A fresh path for the local end of a forward, in a directory only this user can reach.
fn local_socket() -> Result<PathBuf> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    // SAFETY: geteuid has no arguments and cannot fail.
    let user = unsafe { libc::geteuid() };
    // Where there is no runtime directory, the place is everyone's, and each user
    // needs a name there of their own.
    let directory = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(runtime) => PathBuf::from(runtime).join("rex-lite"),
        None => std::env::temp_dir().join(format!("rex-lite-{user}")),
    };
    private(&directory, user)?;
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
        let stderr = child.stderr.take().context("Read ssh")?;
        // Left running once the forward is open, to end when `ssh` does.
        let mut said = tokio::spawn(drain(stderr));
        let mut tunnel = Self { _stdin: child.stdin.take(), child, local };
        let started = Instant::now();
        while tokio::net::UnixStream::connect(&tunnel.local).await.is_err() {
            if tunnel.child.try_wait()?.is_some() {
                // Its last words may still be on their way. Something it started
                // can hold the pipe open, so they are not waited on for long.
                let said = tokio::time::timeout(Duration::from_secs(1), &mut said).await;
                let said = said.ok().and_then(Result::ok).unwrap_or_default();
                bail!("{}", reason(&said));
            }
            ensure!(started.elapsed() < PATIENCE, "ssh did not open the forward in time");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(tunnel)
    }

    /// The local end, as an endpoint to connect to: a bare path, which is taken as
    /// it is written.
    pub fn endpoint(&self) -> String {
        self.local.display().to_string()
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
