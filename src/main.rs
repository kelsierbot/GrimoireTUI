//! grimoire — a terminal writing desk for novels.

mod app;
mod theme;
use grimoire_core::paths::home;
mod library;
mod music;
mod palette;
mod scene;
mod scenery;
mod shutdown;
mod ui;
mod visualizer;
mod viz_view;

#[cfg(test)]
mod ui_tests;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, MouseButton,
    MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use app::{App, Focus, Key, Overlay};
use grimoire_core::project::{self, Project};
use grimoire_core::{export, manuscript};

/// How often we wake to repaint. Also the animation clock.
const TICK: Duration = Duration::from_millis(250);
/// Repaint rate while the spectrum is on screen, fast enough to keep up with a beat.
const FAST_TICK: Duration = Duration::from_millis(33);

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
            println!("\nFor DOCX or EPUB:  grimoire export");
        }
        return Ok(());
    }

    if first.as_deref() == Some("export") {
        return export_command(args);
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
        let port = args.next().and_then(|p| p.parse().ok()).unwrap_or(cfg.port);
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
        println!("  grimoire export [dir]       DOCX + EPUB into exports/; pick formats");
        println!("                              with --docx --epub --md, and parts");
        println!("                              with --parts 1,3");
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
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        project::scaffold(&dir)?;
        println!("Started a manuscript in {}", pretty(&dir));
        println!("Open it with:  grimoire {}", pretty(&dir));
        return Ok(());
    }

    let (root, opening_msg) = resolve_project(first.map(PathBuf::from))?;
    remember(&root);

    let stranded = project::restore_stranded(&root);
    let project = Project::load(&root).context("loading project")?;
    let mut app = App::new(project)?;
    if let Some(m) = opening_msg {
        app.msg = m;
    }
    if !stranded.is_empty() {
        app.msg = App::stranded_note(&stranded);
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
    //
    // Mouse capture goes in a call of its own, first. Windows refuses the
    // keyboard flags outright, and inside one execute! that error would stop
    // the mouse being turned on at all.
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    // A paste arrives as one piece instead of a burst of keystrokes, so it
    // lands as one undo step. Windows' legacy console refuses this; that's fine.
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let _ = execute!(
        std::io::stdout(),
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES),
    );
    app.super_keys = cmd_is_reachable();

    shutdown::install();
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(&mut terminal, &mut app)
    }));
    let res = match res {
        Ok(r) => r,
        Err(_) => {
            // ratatui's panic hook has already put the terminal back and
            // printed the panic. Keep every unsaved word before leaving.
            app.rescue();
            let _ = execute!(
                std::io::stdout(),
                DisableBracketedPaste,
                DisableMouseCapture
            );
            ratatui::restore();
            eprintln!("\nGrimoire hit a bug and closed. Anything unsaved was kept, and will be");
            eprintln!("offered back the next time you open this book.");
            std::process::exit(101);
        }
    };

    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    ratatui::restore();
    res
}

const EXPORT_USAGE: &str = "grimoire export [dir] [--docx] [--epub] [--md] [--parts 1,3]";

/// `grimoire export [dir] [--docx] [--epub] [--md] [--parts 1,3]`. With no
/// format named it writes DOCX and EPUB.
fn export_command(mut args: impl Iterator<Item = String>) -> Result<()> {
    let mut dir = None;
    let (mut docx, mut epub, mut markdown) = (false, false, false);
    let mut numbers: Option<Vec<usize>> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--docx" => docx = true,
            "--epub" => epub = true,
            "--md" | "--markdown" => markdown = true,
            "--parts" => {
                let list = args.next().with_context(|| {
                    format!("--parts needs numbers, like --parts 1,3\n\n  {EXPORT_USAGE}")
                })?;
                numbers = Some(part_numbers(&list)?);
            }
            s if s.starts_with("--parts=") => numbers = Some(part_numbers(&s["--parts=".len()..])?),
            s if s.starts_with('-') => anyhow::bail!("unknown option '{s}'\n\n  {EXPORT_USAGE}"),
            _ if dir.is_none() => dir = Some(PathBuf::from(a)),
            _ => anyhow::bail!("one book at a time — '{a}' is a second folder\n\n  {EXPORT_USAGE}"),
        }
    }
    if !(docx || epub || markdown) {
        (docx, epub) = (true, true);
    }

    let (root, _) = resolve_project(dir)?;
    let project = Project::load(&root).context("loading project")?;

    // 1-based on the command line; node indices underneath.
    let parts = match numbers {
        None => None,
        Some(numbers) => {
            let all = export::parts(&project);
            let noun = project.meta.part_noun();
            if all.is_empty() {
                anyhow::bail!("this book has no {noun}s to choose from — leave out --parts");
            }
            let mut chosen = Vec::new();
            for n in numbers {
                match all.get(n.wrapping_sub(1)) {
                    Some((idx, _)) => chosen.push(*idx),
                    None => anyhow::bail!(
                        "there is no {noun} {n} — this book has {}:\n{}",
                        all.len(),
                        all.iter()
                            .enumerate()
                            .map(|(i, (_, t))| format!("  {}  {t}", i + 1))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                }
            }
            Some(chosen)
        }
    };

    let out = export::export(
        &project,
        &export::ExportOptions {
            docx,
            epub,
            markdown,
            parts,
        },
    )?;
    for f in &out.files {
        println!("Wrote {}", pretty(f));
    }
    let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    println!(
        "  {} · about {} words · {}",
        plural(out.chapters, "chapter"),
        manuscript::commas(manuscript::rounded_words(out.words)),
        plural(out.pages, "page")
    );
    Ok(())
}

/// "1,3" or "1-3" into part numbers.
fn part_numbers(list: &str) -> Result<Vec<usize>> {
    let bad = || anyhow::anyhow!("--parts takes numbers like 1,3 or 2-3, not '{list}'");
    let mut out = Vec::new();
    for item in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match item.split_once('-') {
            Some((a, b)) => {
                let (a, b): (usize, usize) = (
                    a.trim().parse().map_err(|_| bad())?,
                    b.trim().parse().map_err(|_| bad())?,
                );
                if a > b {
                    return Err(bad());
                }
                out.extend(a..=b);
            }
            None => out.push(item.parse().map_err(|_| bad())?),
        }
    }
    if out.is_empty() {
        return Err(bad());
    }
    Ok(out)
}

/// Where a first-time manuscript goes if the user never names one.
fn default_root() -> PathBuf {
    home().join("Documents").join("Grimoire")
}

fn state_path() -> PathBuf {
    home().join(".config").join("grimoire").join("state.toml")
}

/// Shorten the home folder to ~ so printed paths stay readable.
fn pretty(p: &Path) -> String {
    let s = p.display().to_string();
    // Windows canonicalises to the \\?\C:\… form; nobody wants to read that.
    let s = s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s);
    let h = home().display().to_string();
    match s.strip_prefix(&h) {
        Some(rest) if h != "." => format!("~{rest}"),
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
/// Find the book to open, and bring it into the current shape first (once).
fn resolve_project(arg: Option<PathBuf>) -> Result<(PathBuf, Option<String>)> {
    let (root, msg) = find_project(arg)?;
    let done = project::upgrade(&root).context("bringing the book up to date")?;
    let msg = match (msg, done.is_empty()) {
        (msg, true) => msg,
        (Some(m), false) => Some(format!("{m} · {}", done.join(" · "))),
        (None, false) => Some(format!("book updated: {}", done.join(" · "))),
    };
    Ok((root, msg))
}

fn find_project(arg: Option<PathBuf>) -> Result<(PathBuf, Option<String>)> {
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
    if let Some(last) = last_opened()
        && is_project(&last)
    {
        return Ok((last, None));
    }

    let root = default_root();
    if is_project(&root) {
        return Ok((root, None));
    }
    std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
    project::scaffold(&root)?;
    Ok((
        root.clone(),
        Some(format!("new manuscript at {}", pretty(&root))),
    ))
}

/// Whether this terminal is one that can actually deliver Cmd, decided from
/// the environment so startup stays instant. If a Super-modified key ever
/// arrives we believe the evidence over this guess.
fn cmd_is_reachable() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    let prog = std::env::var("TERM_PROGRAM")
        .unwrap_or_default()
        .to_lowercase();
    let term = std::env::var("TERM").unwrap_or_default();
    matches!(prog.as_str(), "ghostty" | "wezterm")
        || term.contains("kitty")
        || std::env::var("KITTY_WINDOW_ID").is_ok()
}

fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    let mut confirm_quit = false;
    let mut last_frame = Instant::now();

    // Keys and the mouse are read on a thread of their own. When the terminal
    // window closes, crossterm 0.29's reader spins forever on the dead tty
    // instead of returning; on this thread that can't stop the loop below from
    // seeing the hang-up, saving, and leaving (which ends the spinning thread).
    let (events_tx, events) = std::sync::mpsc::channel::<std::io::Result<Event>>();
    std::thread::spawn(move || {
        loop {
            let ev = event::read();
            let failed = ev.is_err();
            if events_tx.send(ev).is_err() || failed {
                break;
            }
        }
    });

    loop {
        // Closing the window or being told to stop: save, then go.
        if shutdown::requested() {
            save_or_rescue(app);
            shutdown::done();
            return Ok(());
        }
        app.autosave_tick();
        app.sync_tick();
        app.tick_today();
        app.tick_speller();
        app.tick_backup();
        app.music.drain();
        // The player shows its own notes; anywhere else, the status bar does.
        if let Some(n) = app.music.take_fresh_note()
            && !matches!(app.overlay, Overlay::Player { .. })
        {
            app.msg = format!("♪ {n}");
        }
        app.tick_player();
        if app.pomo.tick() {
            app.msg = match app.pomo.phase {
                scene::Phase::Break => "break — go and look at something far away".into(),
                _ => "back to it".into(),
            };
        }
        app.tick_sprint();

        // The spectrum listens only while it's on screen.
        let spectrum = app.pane_mode == scene::Mode::Visualizer && app.scene_visible;
        app.viz.set_active(spectrum);
        if spectrum {
            app.viz.update();
        }

        // If the terminal itself goes away, drawing to it or reading from it
        // fails: save what there is before reporting that.
        if let Err(e) = terminal.draw(|f| ui::draw(f, app)) {
            save_or_rescue(app);
            return Err(e.into());
        }

        let ev = match events.recv_timeout(if spectrum { FAST_TICK } else { TICK }) {
            Ok(Ok(ev)) => ev,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // The scenes animate on TICK whatever the repaint rate.
                if last_frame.elapsed() >= TICK {
                    app.frame = app.frame.wrapping_add(1);
                    last_frame = Instant::now();
                }
                continue;
            }
            Ok(Err(e)) => {
                save_or_rescue(app);
                return Err(e.into());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                save_or_rescue(app);
                return Ok(());
            }
        };

        if let Event::Paste(text) = &ev {
            if matches!(app.overlay, Overlay::None) {
                app.paste(text);
            } else {
                for c in text.chars().filter(|c| !c.is_control()) {
                    app.on_overlay_key(Key::Char(c));
                }
            }
            continue;
        }

        if let Event::Mouse(m) = ev {
            match m.kind {
                MouseEventKind::Up(MouseButton::Left) => app.drop_tree_drag(),
                MouseEventKind::Down(MouseButton::Left) => app.on_click(m.column, m.row),
                MouseEventKind::Down(MouseButton::Right) => app.on_right_click(m.column, m.row),
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

        if on_key(app, k, &mut confirm_quit) {
            return Ok(());
        }
    }
}

/// One key press, routed the way the running app routes it: the find bar's
/// switches, Ctrl shortcuts, the overlay on top, Esc, function keys, Tab, and
/// then the focused pane. True when it means leave (everything is saved by
/// then, or the second Ctrl-Q said to go anyway).
fn on_key(app: &mut App, k: KeyEvent, confirm_quit: &mut bool) -> bool {
    // Either modifier drives the shortcuts: Ctrl everywhere, Cmd where the
    // terminal can actually report it.
    let sup = k.modifiers.contains(KeyModifiers::SUPER);
    if sup {
        // Hard proof this terminal can send Cmd; trust it over the guess.
        app.super_keys = true;
    }
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL) || sup;

    // The find bar's two switches: ^W / Alt-W whole words, ^E / Alt-C case.
    if matches!(app.overlay, Overlay::Find { .. } | Overlay::FindBook { .. })
        && k.modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
        && let KeyCode::Char(c) = k.code
    {
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        match c.to_ascii_lowercase() {
            'w' => {
                app.toggle_find_opt(true);
                return false;
            }
            'e' if !alt => {
                app.toggle_find_opt(false);
                return false;
            }
            'c' if alt => {
                app.toggle_find_opt(false);
                return false;
            }
            _ => {}
        }
    }

    if ctrl {
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        match k.code {
            KeyCode::Char('s') => {
                app.save();
                *confirm_quit = false;
            }
            KeyCode::Char('c') => app.copy_selection(),
            KeyCode::Char('x') => app.cut(),
            KeyCode::Char('k') if matches!(app.overlay, Overlay::None) => app.open_palette(),
            KeyCode::Char('f') => match &app.overlay {
                Overlay::None => app.open_find(),
                // Ctrl-F again widens the search to the whole book.
                Overlay::Find { query, .. } => {
                    let q = query.clone();
                    app.open_find_book(q);
                }
                _ => {}
            },
            KeyCode::Char('r') => app.replace_all_key(),
            KeyCode::Char('o') if matches!(app.overlay, Overlay::None) => app.open_codex(),
            KeyCode::Char('t') if matches!(app.overlay, Overlay::None) => app.open_marks(),
            KeyCode::Char('d') if matches!(app.overlay, Overlay::None) => app.toggle_focus_mode(),
            KeyCode::Char('z') if shift => app.redo(),
            KeyCode::Char('Z') => app.redo(),
            KeyCode::Char('z') => app.undo(),
            KeyCode::Char('y') => app.redo(),
            KeyCode::Char('q') if quit_or_arm(app, confirm_quit) => return true,
            _ => {}
        }
        return false;
    }

    if *confirm_quit {
        *confirm_quit = false;
        app.msg.clear();
    }

    // Alt-↑ / Alt-↓ move the selected scene or folder (or the one being
    // written). Terminals that keep Alt-arrows for themselves: K / J in the tree.
    if k.modifiers.contains(KeyModifiers::ALT)
        && matches!(k.code, KeyCode::Up | KeyCode::Down)
        && matches!(app.overlay, Overlay::None)
        && matches!(app.focus, Focus::Tree | Focus::Editor)
    {
        app.move_selected(k.code == KeyCode::Up);
        return false;
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
        if std::mem::take(&mut app.quit) && quit_or_arm(app, confirm_quit) {
            return true;
        }
        return false;
    }

    // Esc is the menu, from any pane (F1 too, where a keyboard has one).
    if key == Key::Esc || key == Key::F(1) {
        app.escape();
        return false;
    }

    if key == Key::F(9) {
        app.open_theme_picker();
        return false;
    }

    // Timer and music work from either pane, so they can't eat keystrokes.
    if let Key::F(n) = key {
        app.on_function_key(n);
        return false;
    }

    if key == Key::Tab || key == Key::BackTab {
        app.cycle_focus(key == Key::Tab);
        return false;
    }

    match app.focus {
        Focus::Tree => {
            key = match key {
                Key::Char('j') => Key::Down,
                Key::Char('k') => Key::Up,
                Key::Char('h') => Key::Left,
                Key::Char('l') => Key::Right,
                Key::Char('q') => {
                    if quit_or_arm(app, confirm_quit) {
                        return true;
                    }
                    return false;
                }
                other => other,
            };
            app.on_tree_key(key);
        }
        Focus::Editor => app.on_editor_key(key),
        Focus::Codex => app.on_codex_key(key),
        Focus::Beside => app.on_beside_key(key),
        Focus::Clearing => app.on_clearing_key(key),
        Focus::Music => app.on_music_key(key),
    }

    if std::mem::take(&mut app.quit) && quit_or_arm(app, confirm_quit) {
        return true;
    }
    false
}

/// Quit, saving everything first; true to leave. A save that fails keeps
/// Grimoire open and arms `armed`, so the next Ctrl-Q leaves anyway — the
/// words that couldn't be written are in recovery by then.
fn quit_or_arm(app: &mut App, armed: &mut bool) -> bool {
    if *armed || app.try_quit() {
        return true;
    }
    *armed = true;
    let m = app.mod_label();
    app.msg = format!("{} · {m}Q again to quit anyway", app.msg);
    false
}

/// Leaving for any reason but a key (the window closed, the terminal broke):
/// save what can be saved and keep the rest in recovery.
fn save_or_rescue(app: &mut App) {
    if !app.try_quit() {
        app.rescue();
    }
}
