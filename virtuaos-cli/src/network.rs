//! `virtuaos network` — inspect and (re)configure host networking.
//!
//! Backed by NetworkManager (`nmcli`), which uniformly handles wired and WiFi,
//! DHCP and static addressing. `status` is read-only; `config` is an interactive
//! wizard (pick NIC, scan/join WiFi, choose DHCP or a static IP) that mirrors the
//! installer's network step and both applies the change live AND persists it.

use anyhow::{anyhow, bail, Result};
use owo_colors::OwoColorize;
use std::io::{self, BufRead, Write};
use std::process::Command;

/// Entry point: `virtuaos network` (status) / `virtuaos network config`.
pub fn run(config: bool) -> Result<()> {
    if config {
        config_wizard()
    } else {
        status()
    }
}

/// Whether the `nmcli` binary is installed and runnable at all.
fn have_nmcli() -> bool {
    Command::new("nmcli")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run nmcli, returning trimmed stdout (Err carries stderr on failure).
fn nmcli(args: &[&str]) -> Result<String> {
    let out = Command::new("nmcli")
        .args(args)
        .output()
        .map_err(|e| anyhow!("nmcli not available: {e}"))?;
    if !out.status.success() {
        bail!(
            "nmcli {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// `nmcli -t` terse output escapes ':' as '\:'. Split honoring the escaping.
fn nm_split(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut esc = false;
    for c in line.chars() {
        if esc {
            cur.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == ':' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

// ── status ──────────────────────────────────────────────────────────────────

/// `virtuaos network` — list interfaces (skipping loopback) with their link
/// state and active connection, plus IPv4 address/gateway/DNS for any that
/// are currently connected.
pub fn status() -> Result<()> {
    if !have_nmcli() {
        bail!("NetworkManager (nmcli) is not available on this system.");
    }
    println!("{}", "── Network interfaces ────────────────────".dimmed());
    let devs = nmcli(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"])?;
    for line in devs.lines() {
        let f = nm_split(line);
        if f.len() < 3 {
            continue;
        }
        let (dev, ty, st) = (&f[0], &f[1], &f[2]);
        if ty == "loopback" || dev == "lo" {
            continue;
        }
        let conn = f.get(3).cloned().unwrap_or_default();
        let icon = match ty.as_str() {
            "wifi" => "📶",
            "ethernet" => "🔌",
            _ => "  ",
        };
        let dot = if st == "connected" {
            "●".green().to_string()
        } else {
            "○".red().to_string()
        };
        println!(
            "  {dot} {icon} {:<12} {:<10} {:<14} {}",
            dev,
            ty,
            st,
            if conn.is_empty() {
                "—".dimmed().to_string()
            } else {
                conn
            }
        );
        // Show IPv4 details for connected devices.
        if st == "connected" {
            if let Ok(info) = nmcli(&[
                "-t",
                "-f",
                "IP4.ADDRESS,IP4.GATEWAY,IP4.DNS",
                "device",
                "show",
                dev,
            ]) {
                for l in info.lines() {
                    let kv = nm_split(l);
                    if kv.len() < 2 || kv[1].is_empty() {
                        continue;
                    }
                    let key = kv[0].split('[').next().unwrap_or(&kv[0]);
                    let label = match key {
                        "IP4.ADDRESS" => "addr",
                        "IP4.GATEWAY" => "gw",
                        "IP4.DNS" => "dns",
                        _ => continue,
                    };
                    println!("      {:<5} {}", label.dimmed(), kv[1]);
                }
            }
        }
    }
    Ok(())
}

// ── config wizard ─────────────────────────────────────────────────────────────

fn stdin_line() -> Result<String> {
    let mut s = String::new();
    io::stdin().lock().read_line(&mut s)?;
    Ok(s.trim().to_string())
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    io::stdout().flush()?;
    stdin_line()
}

/// Numbered menu; returns the selected 0-based index.
fn choose(title: &str, items: &[String]) -> Result<usize> {
    println!("\n{}", title.bold());
    for (i, it) in items.iter().enumerate() {
        println!("  {:>2}) {}", i + 1, it);
    }
    loop {
        let raw = prompt(&format!("{} ", "›".cyan()))?;
        if let Ok(n) = raw.parse::<usize>() {
            if n >= 1 && n <= items.len() {
                return Ok(n - 1);
            }
        }
        println!("  {} 1..{}", "enter a number".red(), items.len());
    }
}

/// Loose IPv4 dotted-quad check (each octet fits in a `u8`).
fn ip_ok(s: &str) -> bool {
    let p: Vec<&str> = s.trim().split('.').collect();
    p.len() == 4 && p.iter().all(|x| !x.is_empty() && x.parse::<u8>().is_ok())
}
/// IPv4 CIDR check, e.g. `192.168.1.50/24`.
fn cidr_ok(s: &str) -> bool {
    match s.trim().split_once('/') {
        Some((ip, pfx)) => ip_ok(ip) && pfx.parse::<u8>().map(|n| n <= 32).unwrap_or(false),
        None => false,
    }
}

/// `virtuaos network config` — interactive wizard: pick a NIC, join WiFi
/// (scanning + optional password) if it's a wireless adapter, then choose
/// DHCP or a static IPv4/gateway/DNS, and apply + persist via [`apply`].
fn config_wizard() -> Result<()> {
    if !have_nmcli() {
        bail!("NetworkManager (nmcli) is not available on this system.");
    }
    if !is_root() {
        bail!("Run as root:  sudo virtuaos network config");
    }

    // 1. Pick an interface.
    let dev_raw = nmcli(&["-t", "-f", "DEVICE,TYPE,STATE", "device"])?;
    let ifaces: Vec<(String, String, String)> = dev_raw
        .lines()
        .filter_map(|l| {
            let f = nm_split(l);
            if f.len() < 3 {
                return None;
            }
            if f[1] == "ethernet" || f[1] == "wifi" {
                Some((f[0].clone(), f[1].clone(), f[2].clone()))
            } else {
                None
            }
        })
        .collect();
    if ifaces.is_empty() {
        bail!("No Ethernet/WiFi interface found.");
    }
    let labels: Vec<String> = ifaces
        .iter()
        .map(|(d, t, s)| format!("{:<12} {:<9} {}", d, t, s))
        .collect();
    let idx = choose("Network interface / Interface réseau:", &labels)?;
    let (dev, kind, _) = ifaces[idx].clone();

    let mut ssid = String::new();
    let mut wifi_pw = String::new();

    // 2. WiFi: scan + join.
    if kind == "wifi" {
        let _ = Command::new("rfkill").args(["unblock", "all"]).output();
        println!("{}", "Scanning WiFi… / Recherche WiFi…".dimmed());
        let _ = nmcli(&["device", "wifi", "rescan", "ifname", &dev]);
        let scan = nmcli(&[
            "-t",
            "-f",
            "SSID,SIGNAL,SECURITY",
            "device",
            "wifi",
            "list",
            "ifname",
            &dev,
        ])?;
        let mut seen = std::collections::HashSet::new();
        let mut nets: Vec<(String, String, bool)> = Vec::new();
        for l in scan.lines() {
            let f = nm_split(l);
            let s = f.first().map(|x| x.trim()).unwrap_or("");
            if s.is_empty() || !seen.insert(s.to_string()) {
                continue;
            }
            let sig = f.get(1).cloned().unwrap_or_default();
            let sec = f.get(2).map(|x| x.trim()).unwrap_or("");
            let secured = !(sec.is_empty() || sec == "--");
            nets.push((s.to_string(), sig, secured));
        }
        if nets.is_empty() {
            bail!("No WiFi network found on {dev}.");
        }
        let labels: Vec<String> = nets
            .iter()
            .map(|(s, sig, sec)| format!("{:<28} {:>3}% {}", s, sig, if *sec { "🔒" } else { "" }))
            .collect();
        let wi = choose("WiFi networks / Réseaux WiFi:", &labels)?;
        ssid = nets[wi].0.clone();
        if nets[wi].2 {
            wifi_pw = prompt("WiFi password / Mot de passe WiFi: ")?;
            if wifi_pw.chars().count() < 8 {
                bail!("WiFi password must be at least 8 characters.");
            }
        }
    }

    // 3. DHCP or static.
    let method = choose(
        "Addressing / Attribution:",
        &[
            "Automatic (DHCP) / Automatique (DHCP)".to_string(),
            "Static IP / IP fixe".to_string(),
        ],
    )?;
    let (mut addr, mut gw, mut dns) = (String::new(), String::new(), String::new());
    if method == 1 {
        loop {
            addr = prompt("IP/prefix (e.g. 192.168.1.50/24): ")?;
            if cidr_ok(&addr) {
                break;
            }
            println!("  {}", "invalid — use IP/prefix".red());
        }
        gw = prompt("Gateway / Passerelle (optional): ")?;
        if !gw.is_empty() && !ip_ok(&gw) {
            bail!("Invalid gateway IP.");
        }
        dns = prompt("DNS (space-separated, optional) [1.1.1.1 8.8.8.8]: ")?;
        if dns.is_empty() {
            dns = "1.1.1.1 8.8.8.8".to_string();
        }
    }

    apply(&dev, &kind, &ssid, &wifi_pw, method == 1, &addr, &gw, &dns)?;
    println!("\n{} {}", "✓".green(), "Network applied and saved.".bold());
    println!("{}", "Run `virtuaos network` to verify.".dimmed());
    Ok(())
}

/// Create (or recreate) a dedicated `virtuaos-<dev>` NetworkManager
/// connection profile for `dev` and bring it up. Any previous profile we
/// created for this device is deleted first so the result is deterministic
/// regardless of what was configured before.
#[allow(clippy::too_many_arguments)]
fn apply(
    dev: &str,
    kind: &str,
    ssid: &str,
    wifi_pw: &str,
    is_static: bool,
    addr: &str,
    gw: &str,
    dns: &str,
) -> Result<()> {
    let con = format!("virtuaos-{dev}");
    // Replace any previous profile we created so settings are deterministic.
    let _ = nmcli(&["connection", "delete", &con]);

    if kind == "wifi" {
        let mut args = vec![
            "device", "wifi", "connect", ssid, "ifname", dev, "name", &con,
        ];
        if !wifi_pw.is_empty() {
            args.push("password");
            args.push(wifi_pw);
        }
        nmcli(&args)?;
    } else {
        nmcli(&[
            "connection",
            "add",
            "type",
            "ethernet",
            "ifname",
            dev,
            "con-name",
            &con,
            "autoconnect",
            "yes",
        ])?;
    }

    if is_static {
        let mut args = vec![
            "connection",
            "modify",
            &con,
            "ipv4.method",
            "manual",
            "ipv4.addresses",
            addr,
        ];
        if !gw.is_empty() {
            args.push("ipv4.gateway");
            args.push(gw);
        }
        if !dns.is_empty() {
            args.push("ipv4.dns");
            args.push(dns);
        }
        nmcli(&args)?;
    } else {
        // DHCP: clear any leftover static fields.
        nmcli(&[
            "connection",
            "modify",
            &con,
            "ipv4.method",
            "auto",
            "ipv4.addresses",
            "",
            "ipv4.gateway",
            "",
            "ipv4.dns",
            "",
        ])?;
    }

    nmcli(&["connection", "up", &con])?;
    Ok(())
}
