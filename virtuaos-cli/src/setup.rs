//! `virtuaos setup` — live progress viewer for an in-flight Virtua install/upgrade.
//!
//! The bundled `INSTALL/install.sh` (run by the `auxinux-virtua` .deb setup unit)
//! writes a machine-readable snapshot to `/run/auxinux-virtua-setup.progress` at
//! every phase. This command tails that file and draws a small progress box:
//!
//!     ┌ AuxiNux Virtua — Provisioning ──────────┐
//!     │ [██████████████░░░░░░░░] 64%  14/22      │
//!     │ ▸ Build project                          │
//!     │   elapsed 02:41                          │
//!     └──────────────────────────────────────────┘
//!
//! It draws an interactive TUI on a real terminal, prints plain line updates when
//! stdout is not a TTY (e.g. piped into a journal), and `--once` prints a single
//! snapshot for scripting (exit 0 = running/done, 1 = failed/none).

use anyhow::Result;
use owo_colors::OwoColorize;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_PROGRESS_PATH: &str = "/run/auxinux-virtua-setup.progress";
const BAR_CELLS: usize = 22;
const INNER: usize = 42; // printable width between the box borders

/// Snapshot file path; overridable via AUXINUX_PROGRESS_FILE (mirrors install.sh).
fn progress_path() -> String {
    std::env::var("AUXINUX_PROGRESS_FILE").unwrap_or_else(|_| DEFAULT_PROGRESS_PATH.to_string())
}

#[derive(Default, Clone)]
struct Progress {
    step: u32,
    total: u32,
    pct: u32,
    status: String, // running | done | failed
    mode: String,
    started: u64,
    ts: u64,
    label: String,
}

/// Parse the `key=value` snapshot file. Returns None if absent/unreadable.
fn read_progress() -> Option<Progress> {
    let raw = std::fs::read_to_string(progress_path()).ok()?;
    let mut p = Progress::default();
    for line in raw.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "step" => p.step = v.parse().unwrap_or(0),
            "total" => p.total = v.parse().unwrap_or(0),
            "pct" => p.pct = v.parse().unwrap_or(0),
            "status" => p.status = v.to_string(),
            "mode" => p.mode = v.to_string(),
            "started" => p.started = v.parse().unwrap_or(0),
            "ts" => p.ts = v.parse().unwrap_or(0),
            "label" => p.label = v.to_string(),
            _ => {}
        }
    }
    if p.status.is_empty() {
        p.status = "running".into();
    }
    Some(p)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn fmt_elapsed(secs: u64) -> String {
    let (m, s) = (secs / 60, secs % 60);
    if m >= 60 {
        format!("{:02}:{:02}:{:02}", m / 60, m % 60, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}

fn isatty_stdout() -> bool {
    // 1 = STDOUT_FILENO
    unsafe { libc::isatty(1) == 1 }
}

/// Pad/truncate a string to `width` display columns (single-width chars assumed —
/// true for all glyphs we emit). Counts chars, not bytes, so box edges line up.
fn pad(s: &str, width: usize) -> String {
    let count = s.chars().count();
    if count >= width {
        s.chars().take(width).collect()
    } else {
        let mut out = String::from(s);
        out.extend(std::iter::repeat(' ').take(width - count));
        out
    }
}

fn bar(pct: u32) -> String {
    let filled = (pct as usize * BAR_CELLS / 100).min(BAR_CELLS);
    let mut b = String::with_capacity(BAR_CELLS * 3);
    for _ in 0..filled {
        b.push('█');
    }
    for _ in filled..BAR_CELLS {
        b.push('░');
    }
    b
}

/// Render the four content lines (without borders), already padded to INNER.
fn content_lines(p: &Progress) -> [String; 3] {
    let total = if p.total == 0 { 0 } else { p.total };
    let counter = if total > 0 {
        format!("{}/{}", p.step, total)
    } else {
        format!("{}", p.step)
    };
    let line1 = format!("[{}] {:>3}%  {}", bar(p.pct), p.pct, counter);

    let marker = match p.status.as_str() {
        "done" => "✓",
        "failed" => "✗",
        _ => "▸",
    };
    let label = if p.label.is_empty() {
        "starting…".to_string()
    } else {
        p.label.clone()
    };
    let line2 = format!("{} {}", marker, label);

    let elapsed = now_secs().saturating_sub(if p.started > 0 { p.started } else { now_secs() });
    let foot = match p.status.as_str() {
        "done" => format!("done in {}", fmt_elapsed(elapsed)),
        "failed" => format!("failed after {}", fmt_elapsed(elapsed)),
        _ => format!("elapsed {}", fmt_elapsed(elapsed)),
    };
    let line3 = format!("  {}", foot);

    [pad(&line1, INNER), pad(&line2, INNER), pad(&line3, INNER)]
}

fn draw_tui(p: &Progress) {
    // Move cursor home + clear screen, then redraw.
    print!("\x1b[H\x1b[2J");

    let title = " AuxiNux Virtua — Provisioning ";
    let title_len = title.chars().count();
    // Match the content rows: "│ " + INNER + "│"  →  top must span INNER+1 after ┌.
    let dashes = (INNER + 1).saturating_sub(title_len);
    let top = format!("┌{}{}┐", title, "─".repeat(dashes));

    let [l1, l2, l3] = content_lines(p);
    let bottom = format!("└{}┘", "─".repeat(INNER + 1));

    println!("{}", top.bold());
    // line1: bar — colour by status
    let bar_line = format!("│ {}│", l1);
    match p.status.as_str() {
        "done" => println!("{}", bar_line.green()),
        "failed" => println!("{}", bar_line.red()),
        _ => println!("{}", bar_line.cyan()),
    }
    println!("│ {}│", l2);
    println!("{}", format!("│ {}│", l3).dimmed());
    println!("{}", bottom.bold());

    if p.status == "running" {
        println!(
            "{}",
            "  Ctrl-C to detach (the install keeps running)".dimmed()
        );
    }
    use std::io::Write;
    let _ = std::io::stdout().flush();
}

fn is_terminal_state(status: &str) -> bool {
    status == "done" || status == "failed"
}

/// `virtuaos setup` — entry point. See the module doc comment above for the
/// overall design; `once` selects the single-snapshot mode used by scripts.
pub fn run(once: bool) -> Result<()> {
    // ── One-shot snapshot (scripts / non-interactive) ─────────────────────────
    if once {
        return match read_progress() {
            Some(p) => {
                println!(
                    "{} {}/{} ({}%) {} — {}",
                    p.status, p.step, p.total, p.pct, p.label, p.mode
                );
                if p.status == "failed" {
                    std::process::exit(1);
                }
                Ok(())
            }
            None => {
                println!("none (no provisioning in progress)");
                std::process::exit(1);
            }
        };
    }

    let tty = isatty_stdout();
    if tty {
        print!("\x1b[?25l"); // hide cursor
    }

    let mut last_step: Option<u32> = None;
    let mut last_status = String::new();
    let mut waited_no_file = 0u32;

    let result = (|| -> Result<()> {
        loop {
            match read_progress() {
                None => {
                    if tty {
                        print!("\x1b[H\x1b[2J");
                        println!(
                            "{}",
                            "⏳ waiting for Virtua provisioning to start…".dimmed()
                        );
                        println!(
                            "{}",
                            "   (apt install auxinux-virtua kicks it off in the background)"
                                .dimmed()
                        );
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                    } else if waited_no_file == 0 {
                        println!("waiting for Virtua provisioning to start…");
                    }
                    waited_no_file += 1;
                    // Give up after ~30s with no file when not on a TTY.
                    if !tty && waited_no_file > 60 {
                        println!("no provisioning detected.");
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                Some(p) => {
                    if tty {
                        draw_tui(&p);
                    } else if last_step != Some(p.step) || last_status != p.status {
                        // Clean, line-per-change output for journals/pipes.
                        let marker = match p.status.as_str() {
                            "done" => "✓",
                            "failed" => "✗",
                            _ => "▸",
                        };
                        println!("{} [{}/{}] {}% {}", marker, p.step, p.total, p.pct, p.label);
                        last_step = Some(p.step);
                        last_status = p.status.clone();
                    }
                    if is_terminal_state(&p.status) {
                        if p.status == "failed" {
                            if tty {
                                println!(
                                    "{}",
                                    "  Install failed — see: journalctl -u auxinux-virtua-setup"
                                        .red()
                                );
                            }
                            anyhow::bail!("provisioning failed");
                        }
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(300));
                }
            }
        }
    })();

    if tty {
        print!("\x1b[?25h"); // restore cursor
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
    result
}
