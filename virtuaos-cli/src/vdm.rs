//! `virtuaos vdm` — thin wrapper around the VDM shell installer shipped by
//! the `auxinux-virtua` package.
//!
//! This module does not implement VDM install/HA logic itself; it locates
//! `vdm-install.sh` on disk, validates/confirms input, translates each
//! subcommand into the matching `--flag` and hands off to the script with
//! stdio inherited so its own interactive prompts and progress still work.

use anyhow::{bail, Context, Result};
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::{Command, Stdio};

/// Locations checked for the VDM installer script, in order. Two paths
/// because the package layout moved between releases; keep both until the
/// old path is fully retired.
const INSTALLER_PATHS: &[&str] = &[
    "/opt/auxinux-virtua/INSTALL/vdm-install.sh",
    "/usr/lib/virtuaos/vdm-install.sh",
];

/// Find the first installer script that actually exists on this host.
fn installer() -> Result<&'static str> {
    INSTALLER_PATHS
        .iter()
        .copied()
        .find(|path| Path::new(path).is_file())
        .context("VDM installer is missing. Update the auxinux-virtua package first")
}

fn require_root() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("VDM management requires root. Run this command with sudo");
    }
    Ok(())
}

/// Ask for confirmation unless `assume_yes` is set. Refuses to silently
/// proceed when stdin isn't a terminal (e.g. run from a script) — the
/// caller must pass `--yes` explicitly in that case.
fn confirm(question: &str, assume_yes: bool) -> Result<()> {
    if assume_yes {
        return Ok(());
    }
    if !io::stdin().is_terminal() {
        bail!("confirmation required; rerun with --yes");
    }
    print!("{question} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes" | "o" | "oui") {
        bail!("cancelled");
    }
    Ok(())
}

/// Run the VDM installer script with `args`, inheriting stdio so its
/// interactive prompts and progress output reach the user directly.
fn run(args: &[String]) -> Result<()> {
    require_root()?;
    let script = installer()?;
    let status = Command::new("bash")
        .arg(script)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to run {script}"))?;
    if !status.success() {
        bail!("VDM command failed with status {status}");
    }
    Ok(())
}

/// Install VDM into its managed LXC container (`--install`), optionally
/// enabling HA failover in the same pass.
pub fn install(ha: bool, bridge: &str, ipv4: &str, gateway: Option<&str>) -> Result<()> {
    let mut args = vec![
        "--install".to_string(),
        format!("--lxc-bridge={bridge}"),
        format!("--lxc-ipv4={ipv4}"),
        "--auto-install-host-reqs".to_string(),
    ];
    if let Some(gateway) = gateway {
        args.push(format!("--lxc-gateway={gateway}"));
    }
    if ha {
        args.push("--enable-ha".to_string());
    }
    run(&args)
}

/// Update the existing VDM instance in place, preserving its database/config.
pub fn update() -> Result<()> {
    run(&["--update".to_string()])
}

/// Destroy the managed VDM LXC and its database (after confirmation).
pub fn uninstall(yes: bool) -> Result<()> {
    confirm("Remove the VDM LXC? The VDM database inside it will be deleted.", yes)?;
    run(&["--uninstall".to_string(), "--yes".to_string()])
}

/// Relocate the single VDM instance to another cluster node over SSH.
/// Rejects an empty target or one that looks like a stray flag
/// (`starts_with('-')`) before it ever reaches the shell script.
pub fn move_node(target: &str, yes: bool) -> Result<()> {
    if target.trim().is_empty() || target.starts_with('-') {
        bail!("invalid target node");
    }
    confirm(&format!("Stop VDM and move it to node {target}?"), yes)?;
    run(&[
        "--movenode".to_string(),
        format!("--target-node={target}"),
        "--yes".to_string(),
    ])
}

pub fn status() -> Result<()> {
    run(&["--status".to_string()])
}

/// Forward an HA sub-action (`enable` | `disable` | `status`) as `--ha-<action>`.
pub fn ha(action: &str) -> Result<()> {
    run(&[format!("--ha-{action}")])
}
