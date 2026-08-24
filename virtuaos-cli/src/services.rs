//! `virtuaos services` — systemd service management for the Virtua stack.
//!
//! Lists the AuxiNux/Virtua services with their active state and lets the
//! operator restart or inspect them without reaching for raw `systemctl`.

use anyhow::{bail, Context, Result};
use owo_colors::OwoColorize;
use std::process::Command;

/// Services that make up the Virtua host stack.
const SERVICES: &[&str] = &[
    "auxinuxvirtual-runner",
    "auxinuxvirtual-api",
    "docker",
    "containerd",
    "virtqemud",
    "libvirtd",
];

fn capture(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn require_root() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("Service management requires root. Run this command with sudo");
    }
    Ok(())
}

fn active_state(unit: &str) -> String {
    capture("systemctl", &["is-active", unit]).unwrap_or_else(|| "unknown".into())
}

fn enabled_state(unit: &str) -> String {
    capture("systemctl", &["is-enabled", unit]).unwrap_or_else(|| "unknown".into())
}

/// `vos services` — list the Virtua stack services and their state.
pub fn list() -> Result<()> {
    println!("{:<28} {:<12} {:<12}", "SERVICE".bold(), "STATE".bold(), "ENABLED".bold());
    for unit in SERVICES {
        let state = active_state(unit);
        let enabled = enabled_state(unit);
        let state_colored = if state == "active" {
            state.green().to_string()
        } else if state == "failed" {
            state.red().to_string()
        } else {
            state.yellow().to_string()
        };
        println!("{:<28} {:<12} {:<12}", unit, state_colored, enabled);
    }
    Ok(())
}

/// `vos services restart <unit>` — restart a service (defaults to the whole stack).
pub fn restart(unit: Option<&str>) -> Result<()> {
    require_root()?;
    let targets: Vec<&str> = match unit {
        Some(u) => vec![u],
        None => SERVICES.to_vec(),
    };
    for t in targets {
        let status = Command::new("systemctl")
            .args(["restart", t])
            .status()
            .with_context(|| format!("failed to restart {t}"))?;
        if status.success() {
            println!("{} {t}: restarted", "●".green());
        } else {
            println!("{} {t}: restart failed", "✗".red());
        }
    }
    Ok(())
}

/// `vos services status <unit>` — detailed status of one service.
pub fn status(unit: &str) -> Result<()> {
    let state = active_state(unit);
    let enabled = enabled_state(unit);
    println!("  {:<12} {}", "Service", unit);
    println!("  {:<12} {}", "State", if state == "active" { state.green().to_string() } else { state.red().to_string() });
    println!("  {:<12} {}", "Enabled", enabled);
    Ok(())
}
