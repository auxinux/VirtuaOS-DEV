//! Backend bridge: write the answers, launch `virtuaos-install --backend`, and
//! read the progress file it streams. All the real work (partition/copy/chroot/
//! grub) stays in the proven bash backend — this only orchestrates and observes.

use crate::model::Answers;
use anyhow::{Context, Result};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::process::{Child, Command, Stdio};

pub const CONF_PATH: &str = "/run/virtuaos-install.conf";
pub const PROGRESS_PATH: &str = "/run/virtuaos-install.progress";
pub const BACKEND_BIN: &str = "/usr/local/sbin/virtuaos-install";
pub const LOG_PATH: &str = "/var/log/virtuaos-install.log";

/// Terminal install state parsed from the progress file.
#[derive(Clone, Default)]
pub struct Progress {
    pub pct: u16,
    pub label: String,
    pub detail: String,
    /// "running" | "done" | "failed" | "" (not started yet)
    pub status: String,
}

/// Write the conf the backend reads. 0600 — it carries plaintext passwords on a
/// tmpfs (/run); the backend removes it as soon as it has loaded the answers.
pub fn write_conf(ans: &Answers) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(CONF_PATH)
        .with_context(|| format!("opening {CONF_PATH}"))?;
    f.write_all(ans.to_conf().as_bytes())
        .with_context(|| format!("writing {CONF_PATH}"))?;
    Ok(())
}

/// Spawn the backend. Its stdout/stderr go to the log file so raw tool output
/// (parted, rsync, grub-install) never corrupts the TUI; the structured
/// progress comes back through PROGRESS_PATH.
pub fn spawn_backend() -> Result<Child> {
    // Reset the progress file so a stale one from a previous run isn't read.
    let _ = fs::remove_file(PROGRESS_PATH);

    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(LOG_PATH)
        .or_else(|_| File::create("/dev/null"))
        .context("opening install log")?;
    let log_err = log.try_clone().context("cloning log handle")?;

    let child = Command::new(BACKEND_BIN)
        .arg("--backend")
        .arg(format!("--conf={CONF_PATH}"))
        .env("VIRTUAOS_INSTALL_PROGRESS", PROGRESS_PATH)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .spawn()
        .with_context(|| format!("spawning {BACKEND_BIN}"))?;
    Ok(child)
}

/// Read + parse the latest progress snapshot. Returns None until the first write.
pub fn read_progress() -> Option<Progress> {
    let raw = fs::read_to_string(PROGRESS_PATH).ok()?;
    let mut p = Progress::default();
    for line in raw.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "pct" => p.pct = v.trim().parse().unwrap_or(p.pct).min(100),
            "label" => p.label = v.trim().to_string(),
            "detail" => p.detail = v.trim().to_string(),
            "status" => p.status = v.trim().to_string(),
            _ => {}
        }
    }
    Some(p)
}
