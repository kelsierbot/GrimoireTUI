//! grimoire — a terminal writing desk for novels.

mod app;
mod editor;
mod project;
mod ui;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::path::PathBuf;
use std::time::Duration;

use app::{App, Focus, Key};
use project::Project;

fn main() -> Result<()> {
    let root = match std::env::args().nth(1) {
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

    let mut terminal = ratatui::init();
    let res = run(&mut terminal, &mut app);
    ratatui::restore();
    res
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let mut confirm_quit = false;

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(k) = event::read()? else {
            continue;
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }

        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);

        if ctrl {
            match k.code {
                KeyCode::Char('s') => {
                    app.save();
                    confirm_quit = false;
                }
                KeyCode::Char('q') => {
                    if app.project.dirty_count() > 0 && !confirm_quit {
                        confirm_quit = true;
                        app.msg = "unsaved — ^Q again to discard, ^S to save".into();
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
            _ => Key::Other,
        };

        if key == Key::Tab {
            app.toggle_focus();
            continue;
        }

        match app.focus {
            Focus::Tree => {
                // vim-ish aliases, only where they can't collide with typing
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
