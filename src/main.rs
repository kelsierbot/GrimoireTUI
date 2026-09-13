//! grimoire — a terminal writing desk for novels.

mod app;
mod editor;
mod music;
mod project;
mod scene;
mod ui;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use std::path::PathBuf;
use std::time::Duration;

use app::{App, Focus, Key};
use project::Project;

/// How often we wake to repaint. Also the animation clock.
const TICK: Duration = Duration::from_millis(250);

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let first = args.next();

    if first.as_deref() == Some("music-auth") {
        let cfg = music::Config::load();
        let host = args.next().unwrap_or(cfg.host);
        let port = args
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or(cfg.port);
        return music::authenticate(&host, port);
    }

    if matches!(first.as_deref(), Some("-h") | Some("--help")) {
        println!("grimoire — a terminal writing desk for novels\n");
        println!("usage:");
        println!("  grimoire <project-dir>      open a manuscript");
        println!("  grimoire music-auth         pair with YTMDesktop");
        return Ok(());
    }

    let root = match first {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir()?,
    };

    if !root.join("manuscript").is_dir() {
        anyhow::bail!(
            "no manuscript/ directory found in {}\n\nusage: grimoire <project-dir>",
            root.display()
        );
    }

    let project = Project::load(&root).context("loading project")?;
    let mut app = App::new(project)?;

    let mut terminal = ratatui::try_init()
        .context("this needs a real terminal — grimoire cannot run in a pipe")?;

    // Cmd/Super only reaches a TUI when the terminal speaks the Kitty keyboard
    // protocol. Ghostty, Kitty, WezTerm and foot do; Apple Terminal does not.
    // Ctrl is always accepted, so this only ever adds a second option.
    //
    // We push the flags unconditionally — terminals that don't understand the
    // sequence ignore it. We deliberately do NOT call
    // supports_keyboard_enhancement(): it queries the terminal and blocks for a
    // full 2s when nothing answers, which is most of them.
    let _ = execute!(
        std::io::stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    );
    app.super_keys = cmd_is_reachable();

    let res = run(&mut terminal, &mut app);

    let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    ratatui::restore();
    res
}

/// Whether this terminal is one that can actually deliver Cmd, decided from
/// the environment so startup stays instant. If a Super-modified key ever
/// arrives we believe the evidence over this guess.
fn cmd_is_reachable() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    let prog = std::env::var("TERM_PROGRAM").unwrap_or_default().to_lowercase();
    let term = std::env::var("TERM").unwrap_or_default();
    matches!(prog.as_str(), "ghostty" | "wezterm")
        || term.contains("kitty")
        || std::env::var("KITTY_WINDOW_ID").is_ok()
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let mut confirm_quit = false;

    loop {
        app.music.drain();
        if app.pomo.tick() {
            app.msg = match app.pomo.phase {
                scene::Phase::Break => "break — go and look at something far away".into(),
                _ => "back to it".into(),
            };
        }

        terminal.draw(|f| ui::draw(f, app))?;

        if !event::poll(TICK)? {
            app.frame = app.frame.wrapping_add(1);
            continue;
        }
        let Event::Key(k) = event::read()? else {
            continue;
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }

        // Either modifier drives the shortcuts: Ctrl everywhere, Cmd where the
        // terminal can actually report it.
        let sup = k.modifiers.contains(KeyModifiers::SUPER);
        if sup {
            // Hard proof this terminal can send Cmd; trust it over the guess.
            app.super_keys = true;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL) || sup;

        if ctrl {
            match k.code {
                KeyCode::Char('s') => {
                    app.save();
                    confirm_quit = false;
                }
                KeyCode::Char('q') => {
                    if app.project.dirty_count() > 0 && !confirm_quit {
                        confirm_quit = true;
                        let m = app.mod_label();
                        app.msg = format!("unsaved — {m}Q again to discard, {m}S to save");
                    } else {
                        return Ok(());
                    }
                }
                _ => {}
            }
            continue;
        }

        if confirm_quit {
            confirm_quit = false;
            app.msg.clear();
        }

        let mut key = match k.code {
            KeyCode::Char(c) => Key::Char(c),
            KeyCode::Enter => Key::Enter,
            KeyCode::Backspace => Key::Backspace,
            KeyCode::Delete => Key::Delete,
            KeyCode::Left => Key::Left,
            KeyCode::Right => Key::Right,
            KeyCode::Up => Key::Up,
            KeyCode::Down => Key::Down,
            KeyCode::Home => Key::Home,
            KeyCode::End => Key::End,
            KeyCode::PageUp => Key::PageUp,
            KeyCode::PageDown => Key::PageDown,
            KeyCode::Esc => Key::Esc,
            KeyCode::Tab | KeyCode::BackTab => Key::Tab,
            KeyCode::F(n) => Key::F(n),
            _ => Key::Other,
        };

        // Timer and music work from either pane, so they can't eat keystrokes.
        if let Key::F(n) = key {
            app.on_function_key(n);
            continue;
        }

        if key == Key::Tab {
            app.toggle_focus();
            continue;
        }

        match app.focus {
            Focus::Tree => {
                key = match key {
                    Key::Char('j') => Key::Down,
                    Key::Char('k') => Key::Up,
                    Key::Char('h') => Key::Left,
                    Key::Char('l') => Key::Right,
                    Key::Char('q') => return Ok(()),
                    other => other,
                };
                app.on_tree_key(key);
            }
            Focus::Editor => app.on_editor_key(key),
        }

        if app.quit {
            return Ok(());
        }
    }
}
