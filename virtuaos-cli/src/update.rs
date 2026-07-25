//! The updater: apt orchestration + Virtua service restart.

use anyhow::{bail, Context, Result};
use owo_colors::OwoColorize;
use std::io::{self, Write};
use std::process::Command;

/// Virtua services to restart after the panel package is upgraded.
const VIRTUA_UNITS: &[&str] = &[
    "auxinuxvirtual-runner.service",
    "auxinuxvirtual-api.service",
];

fn require_root() -> Result<()> {
    // geteuid() is always safe to call.
    if unsafe { libc::geteuid() } != 0 {
        bail!("this command must be run as root (try: sudo virtuaos update)");
    }
    Ok(())
}

/// Run a command with inherited stdio (so apt's own progress shows through).
fn run_inherit(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed to launch `{program}`"))?;
    if !status.success() {
        bail!("`{program}` exited with {status}");
    }
    Ok(())
}

/// Packages apt would install/upgrade, parsed from a simulated dist-upgrade.
fn pending_upgrades() -> Result<Vec<String>> {
    let out = Command::new("apt-get")
        .args(["-s", "dist-upgrade"])
        .output()
        .context("failed to run `apt-get -s dist-upgrade`")?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .lines()
        .filter_map(|l| l.strip_prefix("Inst "))
        .filter_map(|l| l.split_whitespace().next())
        .map(|s| s.to_string())
        .collect())
}

/// Best-effort count of upgradable packages (0 on any error).
pub fn upgradable_count() -> Result<usize> {
    Ok(pending_upgrades().map(|v| v.len()).unwrap_or(0))
}

/// Annotate a package name with a human hint.
fn annotate(pkg: &str) -> String {
    match pkg {
        "kernel-virtua" => " (kernel — reboot required)".yellow().to_string(),
        "auxinux-virtua" => " (Virtua panel)".cyan().to_string(),
        "virtuaos-cli" => " (this tool)".dimmed().to_string(),
        _ => String::new(),
    }
}

/// `virtuaos update` — refresh apt indexes, list pending upgrades, and
/// (unless `check`) apply them. Restarts the Virtua services when
/// `auxinux-virtua` was upgraded, and refreshes the release-identity files
/// via [`crate::branding::apply`] when `virtuaos-cli` itself was NOT part of
/// this run (see the comment below for why that ordering matters).
pub fn run(check: bool, yes: bool) -> Result<()> {
    require_root()?;

    println!("{}", "→ Refreshing package lists…".bold());
    run_inherit("apt-get", &["update"])?;

    let upgrades = pending_upgrades()?;
    if upgrades.is_empty() {
        println!("\n{} Everything is already up to date.", "✓".green());
        return Ok(());
    }

    println!(
        "\n{} {} package(s) to upgrade:",
        "⬆".yellow(),
        upgrades.len().to_string().bold()
    );
    for p in &upgrades {
        println!("   • {}{}", p, annotate(p));
    }

    if check {
        println!(
            "\n{} check-only — nothing applied. Run {} to upgrade.",
            "ℹ".blue(),
            "virtuaos update".bold()
        );
        return Ok(());
    }

    if !yes && !confirm("\nApply these updates now?")? {
        println!("Aborted.");
        return Ok(());
    }

    println!("\n{}", "→ Applying updates…".bold());
    run_inherit("apt-get", &["dist-upgrade", "-y"])?;

    if upgrades.iter().any(|p| p == "auxinux-virtua") {
        println!("\n{}", "→ Enabling and restarting Virtua services…".bold());
        for unit in VIRTUA_UNITS {
            if let Err(e) = run_inherit("systemctl", &["enable", unit]) {
                eprintln!("  {} {unit}: {e}", "warn".yellow());
            }
            if let Err(e) = run_inherit("systemctl", &["restart", unit]) {
                eprintln!("  {} {unit}: {e}", "warn".yellow());
            }
        }
    }

    // Keep the VirtuaOS release identity (os-release / issue) in sync. When
    // virtuaos-cli itself was upgraded, its postinst already stamped the NEW
    // version — re-stamping from THIS (now older) process would revert it, so
    // only refresh when the cli was NOT part of this upgrade.
    if !upgrades.iter().any(|p| p == "virtuaos-cli") {
        if let Err(e) = crate::branding::apply() {
            eprintln!("  {} branding refresh skipped: {e}", "warn".yellow());
        }
    }

    println!("\n{} Update complete.", "✓".green());
    if upgrades.iter().any(|p| p == "kernel-virtua") {
        println!(
            "{} A new kernel was installed — reboot to activate it ({}).",
            "⚠".yellow(),
            "sudo reboot".bold()
        );
    }
    Ok(())
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_lowercase().as_str(),
        "y" | "yes" | "o" | "oui"
    ))
}
