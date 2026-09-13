//! grimoire — a terminal writing desk for novels.

mod app;
mod editor;
mod library;
mod manuscript;
mod music;
mod project;
mod scene;
mod theme;
mod ui;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    KeyboardEnhancementFlags, MouseButton, MouseEventKind, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use std::path::{Path, PathBuf};
use std::time::Duration;

use app::{App, Focus, Key, Overlay};
use project::Project;

/// How often we wake to repaint. Also the animation clock.
const TICK: Duration = Duration::from_millis(250);

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let first = args.next();

    if matches!(first.as_deref(), Some("compile") | Some("index")) {
        let want = first.clone().unwrap();
        let (root, _) = resolve_project(args.next().map(PathBuf::from))?;
        let project = Project::load(&root).context("loading project")?;
        if want == "index" {
            let p = manuscript::write_project_file(&project)?;
            println!("Project map written to {}", pretty(&p));
        } else {
            let c = manuscript::compile(&project)?;
            println!("Compiled {}", pretty(&c.path));
            println!(
                "  {} chapters · {} scenes · {} words{}",
                c.chapters,
                c.scenes,
                c.words,
                if c.skipped > 0 {
                    format!(" · {} skipped (compile: false)", c.skipped)
                } else {
                    String::new()
                }
            );
            println!("\nFor DOCX or PDF:");
            println!("  pandoc \"{}\" -o manuscript.docx", c.path.display());
        }
        return Ok(());
    }

    if first.as_deref() == Some("music-setup") {
        return match args.next().as_deref() {
            None => music::setup_for(music::Config::load().source),
            Some(name) => match music::Source::parse(name) {
                Some(s) => music::setup_for(s),
                None => anyhow::bail!(
                    "unknown source '{name}'\n\n  youtube-music · spotify · jellyfin · plex"
                ),
            },
        };
    }

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
        println!("  grimoire                    open your current manuscript");
        println!("  grimoire <dir>              open a specific one");
        println!("  grimoire new <dir>          start a new one");
        println!("  grimoire index              refresh project.md, the project map");
        println!("  grimoire compile            assemble the manuscript");
        println!("  grimoire music-setup <src>  connect music: youtube-music |");
        println!("                              spotify | jellyfin | plex");
        println!("  grimoire music-auth         re-pair only");
        println!();
        println!("With no arguments grimoire opens the current directory if it is a");
        println!("manuscript, otherwise the last one you had open, otherwise it creates");
        println!("{}.", pretty(&default_root()));
        return Ok(());
    }

    if first.as_deref() == Some("new") {
        let dir = args.next().map(PathBuf::from).unwrap_or_else(default_root);
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("creating {}", dir.display()))?;
        project::scaffold(&dir)?;
        println!("Started a manuscript in {}", pretty(&dir));
        println!("Open it with:  grimoire {}", pretty(&dir));
        return Ok(());
    }

    let (root, opening_msg) = resolve_project(first.map(PathBuf::from))?;
    remember(&root);

    let project = Project::load(&root).context("loading project")?;
    let mut app = App::new(project)?;
    if let Some(m) = opening_msg {
        app.msg = m;
    }

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
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
        EnableMouseCapture,
    );
    app.super_keys = cmd_is_reachable();

    let res = run(&mut terminal, &mut app);

    let _ = execute!(std::io::stdout(), DisableMouseCapture, PopKeyboardEnhancementFlags);
    ratatui::restore();
    res
}


/// Where a first-time manuscript goes if the user never names one.
fn default_root() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Documents/Grimoire")
}

fn state_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/grimoire/state.toml")
}

/// Shorten $HOME to ~ so printed paths stay readable.
fn pretty(p: &Path) -> String {
    let s = p.display().to_string();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}

fn is_project(p: &Path) -> bool {
    p.join("manuscript").is_dir()
}

fn last_opened() -> Option<PathBuf> {
    let s = std::fs::read_to_string(state_path()).ok()?;
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("last = ") {
            let v = v.trim().trim_matches('"');
            if !v.is_empty() {
                return Some(PathBuf::from(v));
            }
        }
    }
    None
}

fn remember(root: &Path) {
    let path = state_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("last = \"{}\"\n", root.display()));
}

/// Decide what `grimoire` with no arguments should open. In order: the current
/// directory, the last manuscript you had open, or a fresh one.
fn resolve_project(arg: Option<PathBuf>) -> Result<(PathBuf, Option<String>)> {
    if let Some(p) = arg {
        let abs = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if !abs.exists() {
            anyhow::bail!(
                "{} does not exist\n\n  grimoire new {}   start a manuscript there\n  grimoire                  open your current one",
                pretty(&abs),
                pretty(&p)
            );
        }
        if !is_project(&abs) {
            anyhow::bail!(
                "{} is not a manuscript — no manuscript/ directory inside it\n\n  grimoire new {}   start one there",
                pretty(&abs),
                pretty(&p)
            );
        }
        return Ok((abs, None));
    }

    let cwd = std::env::current_dir()?;
    if is_project(&cwd) {
        return Ok((cwd, None));
    }
    if let Some(last) = last_opened() {
        if is_project(&last) {
            return Ok((last, None));
        }
    }

    let root = default_root();
    if is_project(&root) {
        return Ok((root, None));
    }
    std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
    project::scaffold(&root)?;
    Ok((root.clone(), Some(format!("new manuscript at {}", pretty(&root)))))
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
        let ev = event::read()?;

        if let Event::Mouse(m) = ev {
            match m.kind {
                MouseEventKind::Down(MouseButton::Left) => app.on_click(m.column, m.row),
                MouseEventKind::Drag(MouseButton::Left) => app.on_drag(m.column, m.row),
                MouseEventKind::ScrollDown => app.on_scroll(m.column, m.row, true),
                MouseEventKind::ScrollUp => app.on_scroll(m.column, m.row, false),
                _ => {}
            }
            continue;
        }

        let Event::Key(k) = ev else {
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
                KeyCode::Char('c') => app.copy_selection(),
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
            KeyCode::Tab => Key::Tab,
            KeyCode::BackTab => Key::BackTab,
            KeyCode::F(n) => Key::F(n),
            _ => Key::Other,
        };

        if !matches!(app.overlay, Overlay::None) {
            app.on_overlay_key(key);
            continue;
        }

        if key == Key::F(1) {
            app.open_menu();
            continue;
        }

        if key == Key::F(9) {
            app.open_theme_picker();
            continue;
        }

        // Timer and music work from either pane, so they can't eat keystrokes.
        if let Key::F(n) = key {
            app.on_function_key(n);
            continue;
        }

        if key == Key::Tab || key == Key::BackTab {
            app.cycle_focus(key == Key::Tab);
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
            Focus::Clearing => app.on_clearing_key(key),
            Focus::Music => app.on_music_key(key),
        }

        if app.quit {
            return Ok(());
        }
    }
}
