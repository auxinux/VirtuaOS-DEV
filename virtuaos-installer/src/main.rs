//! VirtuaOS installer — ratatui TUI front-end.
//!
//! Collects every answer in a polished wizard, writes them to the backend conf,
//! then launches `virtuaos-install --backend` and renders the live progress it
//! streams. The heavy lifting (partition/copy/chroot/grub) stays in the proven
//! bash backend; this binary only owns the look-and-feel.

mod app;
mod install;
mod model;
mod theme;

use anyhow::{Context, Result};
use app::{Action, App, Step};
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, cursor};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Stdout};
use std::process::Command;
use std::time::Duration;

type Term = Terminal<CrosstermBackend<Stdout>>;

fn main() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        eprintln!("virtuaos-installer must run as root (e.g. sudo virtuaos-installer).");
        std::process::exit(1);
    }

    // Restore the terminal even if a panic unwinds through the draw loop.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
        default_hook(info);
    }));

    let mut terminal = setup_terminal().context("initialising terminal")?;
    let result = run(&mut terminal);
    restore_terminal(&mut terminal).ok();

    match result {
        Ok(true) => {
            // User asked to reboot from the success screen.
            Command::new("reboot").status().ok();
            Ok(())
        }
        Ok(false) => Ok(()),
        Err(e) => Err(e),
    }
}

/// Enter raw mode + the alternate screen and hide the cursor, ready for
/// ratatui to take over the whole terminal.
fn setup_terminal() -> Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, cursor::Hide)?;
    let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    Ok(terminal)
}

/// Undo [`setup_terminal`] so the shell prompt is left in a usable state.
fn restore_terminal(terminal: &mut Term) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, cursor::Show)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Returns Ok(true) if the user chose to reboot.
fn run(terminal: &mut Term) -> Result<bool> {
    let mut app = App::new();
    loop {
        terminal.draw(|f| app.draw(f))?;

        if app.step == Step::Installing {
            app.poll_install();
        }

        // Short timeout so the install screen animates even with no key events.
        if event::poll(Duration::from_millis(120))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match app.on_key(key) {
                        Action::Quit => return Ok(false),
                        Action::Reboot => return Ok(true),
                        Action::None => {}
                    }
                }
            }
        }
    }
}
