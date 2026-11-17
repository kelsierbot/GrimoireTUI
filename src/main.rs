//! grimoire — a terminal writing desk for novels.

mod app;
mod books;
mod help;
mod library;
mod music;
mod palette;
mod scene;
mod scenery;
mod shutdown;
mod theme;
mod ui;
mod visualizer;
mod viz_view;
mod voice;

#[cfg(test)]
mod shots;
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
use grimoire_core::{export, export_print, manuscript};

/// How often we wake to repaint. Also the animation clock.
const TICK: Duration = Duration::from_millis(250);
/// Repaint rate while the spectrum is on screen, fast enough to keep up with a beat.
const FAST_TICK: Duration = Duration::from_millis(33);
/// How often to redraw while the Rainbow theme's colours flow: often enough
/// that the sweep glides rather than steps.
const RAINBOW_TICK: Duration = Duration::from_millis(80);

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

    if matches!(first.as_deref(), Some("-V") | Some("--version")) {
        println!(
            "grimoire {} · made by Catfinity Studios, makers of Catfinity · https://catfinity.com",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }

    // `grimoire help` too, unless there's a book in a folder called that.
    let asked_for_help = match first.as_deref() {
        Some("-h") | Some("--help") => true,
        Some("help") => !Path::new("help").exists(),
        _ => false,
    };
    if asked_for_help {
        println!("grimoire: a terminal writing desk for novels\n");
        println!("usage:");
        println!("  grimoire                    open your current manuscript");
        println!("  grimoire <dir>              open a specific one");
        println!("  grimoire new <dir>          start a new one: a novel, or add");
        println!("                              --short-story or --blank");
        println!("  grimoire import <draft>     bring in a draft (Word, Markdown or");
        println!("                              text) as a new book beside it");
        println!("  grimoire where              say where your book is kept");
        println!("  grimoire copy [dir]         the whole book as one .zip, to keep or");
        println!("                              send; add --to <folder> to choose where");
        println!("  grimoire <copy.zip>         open a copy as a book of its own");
        println!("  grimoire index              refresh project.md, the project map");
        println!("  grimoire compile            assemble the manuscript");
        println!("  grimoire export [dir]       DOCX + EPUB into exports/; pick formats");
        println!("                              with --docx --pdf --epub --paperback");
        println!("                              --md, a trim with --trim 6x9, parts");
        println!("                              with --parts 1,3, or a sample with");
        println!("                              --chapters 1-3 or --words 10000");
        println!("  grimoire --version          which version, and who made it");
        println!("  grimoire music-setup <src>  connect music: youtube-music |");
        println!("                              spotify | jellyfin | plex");
        println!("  grimoire music-auth         re-pair only");
        println!();
        println!("With no arguments grimoire opens the book in the folder you're in, if");
        println!("you're in one, otherwise the last one you had open. On a computer's");
        println!("first start it opens the newest book in your books folder,");
        println!("{},", pretty(&grimoire_core::shelf::home()));
        println!("or makes your first book there.");
        return Ok(());
    }

    // Where the book is, without opening it: the answer to "where did I
    // put that?" from any terminal.
    if first.as_deref() == Some("where") {
        let Some(root) = book_in_hand(args.next().map(PathBuf::from)) else {
            println!("No book has been opened on this computer yet.");
            println!(
                "New books are kept in {}.",
                pretty(&grimoire_core::shelf::home())
            );
            return Ok(());
        };
        let facts = grimoire_core::shelf::facts(&root);
        let title = books::title(&root);
        println!("{title}");
        println!("  {}", root.display());
        println!(
            "  a folder of plain text files: {} files, {} KB",
            facts.files,
            facts.bytes.div_ceil(1024)
        );
        let copies = grimoire_core::shelf::copies_dir(&grimoire_core::shelf::home());
        match grimoire_core::shelf::newest_copy(&copies, &title, &root) {
            Some((file, _)) => println!("  newest copy: {}", file.display()),
            None => println!("  no copy saved yet · grimoire copy makes one"),
        }
        return Ok(());
    }

    // The whole book as one file, to keep somewhere safe or to send.
    if first.as_deref() == Some("copy") {
        let (mut dir, mut to) = (None, None);
        while let Some(a) = args.next() {
            match a.as_str() {
                "--to" => {
                    to = Some(PathBuf::from(args.next().with_context(|| {
                        "--to needs a folder, like --to ~/Desktop\n\n  grimoire copy [book folder] [--to folder]"
                    })?))
                }
                s if s.starts_with('-') => anyhow::bail!(
                    "unknown option '{s}'\n\n  grimoire copy [book folder] [--to folder]"
                ),
                _ => dir = Some(PathBuf::from(a)),
            }
        }
        let root = book_in_hand(dir).context(
            "which book? No book has been opened here yet\n\n  grimoire copy <book folder>",
        )?;
        let into =
            to.unwrap_or_else(|| grimoire_core::shelf::copies_dir(&grimoire_core::shelf::home()));
        let file = grimoire_core::shelf::save_copy(
            &root,
            &books::title(&root),
            &into,
            chrono::Local::now().date_naive(),
        )?;
        println!("Saved a copy of {}:", books::title(&root));
        println!("  {}", file.display());
        println!(
            "Open it on any computer with:  grimoire \"{}\"",
            pretty(&file)
        );
        return Ok(());
    }

    if first.as_deref() == Some("import") {
        let Some(draft) = args.next().map(PathBuf::from) else {
            anyhow::bail!("which draft?\n\n  grimoire import <draft.docx|.md|.txt> [book folder]");
        };
        let book = grimoire_core::import::read(&draft)?;
        let dir = match args.next() {
            Some(d) => PathBuf::from(d),
            None => books::beside(draft.parent().unwrap_or(Path::new(".")), &book.title),
        };
        if dir.exists() && std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_some()) {
            anyhow::bail!(
                "{} already has things in it\n\n  grimoire import {} <another folder>",
                pretty(&dir),
                pretty(&draft)
            );
        }
        grimoire_core::import::make_book(&dir, &book)?;
        println!("Brought in {}: {}", book.title, books::tally(&book));
        if !book.before.trim().is_empty() {
            println!("What came before the first chapter is in Notes.");
        }
        println!("Open it with:  grimoire {}", pretty(&dir));
        return Ok(());
    }

    if first.as_deref() == Some("new") {
        // `--short-story` or `--blank`, before or after the folder.
        let mut shape = project::Template::Novel;
        let mut dir = None;
        for a in args {
            match project::Template::parse(&a).filter(|_| a.starts_with("--")) {
                Some(t) => shape = t,
                None if a.starts_with("--") => anyhow::bail!(
                    "unknown shape {a}\n\n  grimoire new [--short-story | --blank] <dir>"
                ),
                None => dir = Some(PathBuf::from(a)),
            }
        }
        // No folder named: in the books folder, never over a book there.
        let dir = dir.unwrap_or_else(|| {
            grimoire_core::shelf::unique(&grimoire_core::shelf::home(), "New Book")
        });
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        project::scaffold_with(&dir, None, &shape.shape())?;
        println!("Started a manuscript in {}", pretty(&dir));
        println!("Open it with:  grimoire {}", pretty(&dir));
        return Ok(());
    }

    // A mistyped option isn't a book to open (or to offer to make).
    if let Some(option) = first.as_deref().filter(|a| a.starts_with('-')) {
        anyhow::bail!("there's no option '{option}' · grimoire --help lists what there is");
    }
    // A copy of a book (one .zip) opens as a book of its own, in the books
    // folder; the copy is left as it was.
    let mut from_copy = None;
    let first = match first.map(PathBuf::from) {
        Some(p) if grimoire_core::shelf::is_copy(&p) => {
            let root = grimoire_core::shelf::open_copy(&p, &grimoire_core::shelf::home())?;
            from_copy = Some(format!(
                "opened the copy as a book of its own · it lives in {}",
                pretty(&root)
            ));
            Some(root)
        }
        other => other,
    };
    let (root, opening_msg) = resolve_project(first)?;
    let opening_msg = from_copy.or(opening_msg);
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
        .context("this needs a real terminal, because grimoire cannot run in a pipe")?;

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

const EXPORT_USAGE: &str = "grimoire export [dir] [--docx] [--pdf] [--paperback [--trim 6x9]] \
[--epub] [--md] [--parts 1,3] [--chapters 1-3 | --words 10000]";

/// `grimoire export [dir] [--docx] [--pdf] [--paperback [--trim 6x9]] [--epub]
/// [--md] [--parts 1,3] [--chapters 1-3 | --words 10000]`. With no format
/// named it writes DOCX and EPUB. A sample (`--chapters`, `--words`) is what
/// an agent asks for; `--trim` alone implies `--paperback`.
fn export_command(mut args: impl Iterator<Item = String>) -> Result<()> {
    let mut dir = None;
    let (mut docx, mut epub, mut markdown, mut pdf) = (false, false, false, false);
    let mut paperback = false;
    let mut trim = None;
    let mut scope = export::Scope::Whole;
    let mut numbers: Option<Vec<usize>> = None;
    let parse_trim = |s: &str| {
        export_print::Trim::parse(s).with_context(|| {
            format!("--trim takes 5x8, 5.25x8, 5.5x8.5 or 6x9, not '{s}'\n\n  {EXPORT_USAGE}")
        })
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--docx" => docx = true,
            "--pdf" => pdf = true,
            "--chapters" => {
                let list = args.next().with_context(|| {
                    format!("--chapters needs a range, like --chapters 1-3\n\n  {EXPORT_USAGE}")
                })?;
                scope = chapter_range(&list)?;
            }
            "--words" => {
                let n = args.next().with_context(|| {
                    format!("--words needs a number, like --words 10000\n\n  {EXPORT_USAGE}")
                })?;
                scope = export::Scope::Words(
                    n.replace(',', "")
                        .parse()
                        .map_err(|_| anyhow::anyhow!("--words takes a number, not '{n}'"))?,
                );
            }
            "--epub" => epub = true,
            "--md" | "--markdown" => markdown = true,
            "--paperback" | "--print" => paperback = true,
            "--trim" => {
                let t = args.next().with_context(|| {
                    format!("--trim needs a size, like --trim 6x9\n\n  {EXPORT_USAGE}")
                })?;
                trim = Some(parse_trim(&t)?);
            }
            s if s.starts_with("--trim=") => trim = Some(parse_trim(&s["--trim=".len()..])?),
            "--parts" => {
                let list = args.next().with_context(|| {
                    format!("--parts needs numbers, like --parts 1,3\n\n  {EXPORT_USAGE}")
                })?;
                numbers = Some(part_numbers(&list)?);
            }
            s if s.starts_with("--parts=") => numbers = Some(part_numbers(&s["--parts=".len()..])?),
            s if s.starts_with('-') => anyhow::bail!("unknown option '{s}'\n\n  {EXPORT_USAGE}"),
            _ if dir.is_none() => dir = Some(PathBuf::from(a)),
            _ => anyhow::bail!(
                "one book at a time, and '{a}' is a second folder\n\n  {EXPORT_USAGE}"
            ),
        }
    }
    // A trim size is only for the paperback, so asking for one asks for it.
    paperback |= trim.is_some();
    if !(docx || epub || markdown || pdf || paperback) {
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
                anyhow::bail!("this book has no {noun}s to choose from, so leave out --parts");
            }
            let mut chosen = Vec::new();
            for n in numbers {
                match all.get(n.wrapping_sub(1)) {
                    Some((idx, _)) => chosen.push(*idx),
                    None => anyhow::bail!(
                        "there is no {noun} {n}. This book has {}:\n{}",
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
            pdf,
            epub,
            markdown,
            paperback,
            paperback_pdf: true,
            trim,
            parts,
            scope,
        },
    )?;
    for f in &out.files {
        println!("Wrote {}", pretty(f));
    }
    if let Some(why) = &out.pdf_error {
        eprintln!("No PDF: {why}");
    }
    for n in &out.notes {
        eprintln!("{n}");
    }
    if !out.tks.is_empty() {
        eprintln!(
            "Note: {} TK{} still in the text:",
            out.tks.len(),
            if out.tks.len() == 1 { " is" } else { "s are" }
        );
        for t in out.tks.iter().take(10) {
            eprintln!("  {t}");
        }
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

/// "1-3", "3" or "2–5" into a chapter range.
fn chapter_range(list: &str) -> Result<export::Scope> {
    let bad = || anyhow::anyhow!("--chapters takes a range like 1-3, not '{list}'");
    let norm = list.replace('–', "-");
    let (a, b) = match norm.split_once('-') {
        Some((a, b)) => (a.trim(), b.trim()),
        None => (norm.trim(), norm.trim()),
    };
    let (from, to): (usize, usize) = (a.parse().map_err(|_| bad())?, b.parse().map_err(|_| bad())?);
    if from == 0 || to < from {
        return Err(bad());
    }
    Ok(export::Scope::Chapters { from, to })
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

use books::{default_root, is_book as is_project, pretty};
use grimoire_core::paths::home;

fn last_opened() -> Option<PathBuf> {
    books::last(&books::state_path())
}

fn remember(root: &Path) {
    books::remember(&books::state_path(), root);
}

/// Decide what `grimoire` with no arguments should open. In order: the current
/// directory, the last manuscript you had open, or a fresh one.
/// Find the book to open, and bring it into the current shape first (once).
/// The book a command means when it's given a folder, or none: that
/// folder, else the folder you're standing in, else the one last open.
/// Never makes a book; `None` if there isn't one.
fn book_in_hand(arg: Option<PathBuf>) -> Option<PathBuf> {
    let real = |p: PathBuf| std::fs::canonicalize(&p).unwrap_or(p);
    if let Some(p) = arg {
        return is_project(&p).then(|| real(p));
    }
    let cwd = std::env::current_dir().ok()?;
    if is_project(&cwd) {
        return Some(real(cwd));
    }
    books::last(&books::state_path()).filter(|p| is_project(p))
}

fn resolve_project(arg: Option<PathBuf>) -> Result<(PathBuf, Option<String>)> {
    let (root, msg) = find_project(arg)?;
    // Something the upgrade can't reach (a folder not downloaded yet) mustn't
    // keep the book shut: open it as it is and try again next time.
    let done = project::upgrade(&root).unwrap_or_else(|e| {
        vec![format!(
            "not all of it could be brought up to date yet ({e:#})"
        )]
    });
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
                "{} is not a manuscript (no manuscript/ directory inside it)\n\n  grimoire new {}   start one there",
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

    first_start(&grimoire_core::shelf::home(), &default_root())
}

/// Nothing to go on: no book here, none remembered. A computer with books
/// in its books folder already (carried over from another one, say) opens
/// the one worked on last. A book made before there was a books folder
/// still opens where it is. Otherwise this is the very first start, and
/// the first book is made in the books folder, where every later one goes.
fn first_start(home: &Path, older: &Path) -> Result<(PathBuf, Option<String>)> {
    if let Some(book) = grimoire_core::shelf::books_in(home).into_iter().next() {
        let said = format!("opened {} from your books folder", books::title(&book));
        return Ok((book, Some(said)));
    }
    if is_project(older) {
        return Ok((older.to_path_buf(), None));
    }
    let root = home.join(grimoire_core::shelf::FIRST_BOOK);
    std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
    project::scaffold(&root)?;
    Ok((
        root.clone(),
        Some(format!(
            "your first book is ready, in {} · this page shows how it's laid out",
            pretty(&root)
        )),
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

/// One frame, painted where the writer can't see it happen. The terminal's
/// cursor is what paints: it visits every cell that changed before it comes
/// back to the page, and in a theme that's always moving (Rainbow's colours
/// flow along the title line twelve times a second) it was seen flashing
/// up there. So the frame is handed over whole (terminals that know how
/// show nothing until it's finished), and the cursor is put away while it's
/// painted for the ones that don't.
fn draw_frame(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> Result<()> {
    use ratatui::crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
    execute!(terminal.backend_mut(), BeginSynchronizedUpdate)?;
    terminal.hide_cursor()?;
    let drawn = terminal.draw(|f| ui::draw(f, app)).map(|_| ());
    // Always handed back, or a terminal would go on holding the screen.
    execute!(terminal.backend_mut(), EndSynchronizedUpdate)?;
    drawn?;
    Ok(())
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
        app.tick_pdf();
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
                scene::Phase::Break => "break · go and look at something far away".into(),
                _ => "back to it".into(),
            };
        }
        app.tick_sprint();
        app.tick_reading();

        // The spectrum listens only while it's on screen.
        let spectrum = app.pane_mode == scene::Mode::Visualizer && app.scene_visible;
        app.viz.set_active(spectrum);
        if spectrum {
            app.viz.update();
        }

        // If the terminal itself goes away, drawing to it or reading from it
        // fails: save what there is before reporting that.
        if let Err(e) = draw_frame(terminal, app) {
            save_or_rescue(app);
            return Err(e);
        }

        let tick = if spectrum || app.reading.is_some() {
            FAST_TICK
        } else if app.theme.is_rainbow() {
            RAINBOW_TICK
        } else {
            TICK
        };
        let ev = match events.recv_timeout(tick) {
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
            } else if app.overlay_takes_text() {
                // A tab is a space in a one-line box, as it is on the page.
                for c in text
                    .chars()
                    .map(|c| if c == '\t' { ' ' } else { c })
                    .filter(|c| !c.is_control())
                {
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
    // AltGr arrives as Ctrl+Alt on Windows; what it makes (@, €, ł…) is
    // typing, not a shortcut.
    let k = match k.code {
        KeyCode::Char(c)
            if k.modifiers
                .contains(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && !c.is_ascii_alphanumeric() =>
        {
            KeyEvent::new(k.code, KeyModifiers::NONE)
        }
        _ => k,
    };
    // The page's own moves with Shift, Ctrl and Alt: selecting from the
    // keyboard, and going (or deleting) a word at a time.
    if matches!(app.overlay, Overlay::None) && app.focus == Focus::Editor && app.open.is_some() {
        let m = k.modifiers;
        let shift = m.contains(KeyModifiers::SHIFT);
        let alt = m.contains(KeyModifiers::ALT);
        let ctrl = m.contains(KeyModifiers::CONTROL) || m.contains(KeyModifiers::SUPER);
        let by_word = ctrl || alt;
        let arrow = |code: KeyCode| match code {
            KeyCode::Left => Some(Key::Left),
            KeyCode::Right => Some(Key::Right),
            KeyCode::Up => Some(Key::Up),
            KeyCode::Down => Some(Key::Down),
            KeyCode::Home => Some(Key::Home),
            KeyCode::End => Some(Key::End),
            KeyCode::PageUp => Some(Key::PageUp),
            KeyCode::PageDown => Some(Key::PageDown),
            _ => None,
        };
        // Alt-↑ / Alt-↓ alone still move the scene (below).
        let moves_scene = alt && !shift && !ctrl && matches!(k.code, KeyCode::Up | KeyCode::Down);
        if !moves_scene
            && (shift || by_word)
            && let Some(key) = arrow(k.code)
        {
            app.editor_move(key, shift, by_word);
            return false;
        }
        match k.code {
            KeyCode::Backspace if by_word => {
                app.delete_word(true);
                return false;
            }
            KeyCode::Delete if by_word => {
                app.delete_word(false);
                return false;
            }
            // A Mac's Option-← and Option-→ arrive as Alt-b and Alt-f.
            KeyCode::Char('b') if alt && !ctrl => {
                app.editor_move(Key::Left, shift, true);
                return false;
            }
            KeyCode::Char('f') if alt && !ctrl => {
                app.editor_move(Key::Right, shift, true);
                return false;
            }
            KeyCode::Char('d') if alt && !ctrl => {
                app.delete_word(false);
                return false;
            }
            // Any other Alt-letter is a shortcut the page doesn't have: it
            // mustn't type the bare letter into the prose.
            KeyCode::Char(_) if alt && !ctrl => return false,
            _ => {}
        }
    }
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
            // Cutting and undoing belong to the desk: with a box up they'd
            // change the page behind it, unseen. (Find keeps undo, for a
            // replace you want back.)
            KeyCode::Char('x') if matches!(app.overlay, Overlay::None) => app.cut(),
            // Paste: into the page, or into whatever box is being typed in.
            KeyCode::Char('v') if matches!(app.overlay, Overlay::None) => app.paste_clipboard(),
            KeyCode::Char('v') if app.overlay_takes_text() => {
                for c in app
                    .clipboard_text()
                    .chars()
                    .map(|c| if c == '\t' { ' ' } else { c })
                    .filter(|c| !c.is_control())
                {
                    app.on_overlay_key(Key::Char(c));
                }
            }
            KeyCode::Char('a') if matches!(app.overlay, Overlay::None) => app.select_all(),
            // Bold and italics. Ctrl-I reaches Grimoire only where the
            // terminal tells it apart from Tab (Ghostty, Kitty, WezTerm,
            // foot); everywhere, the menu's Writing tools have both.
            KeyCode::Char('b') if matches!(app.overlay, Overlay::None) => app.emphasis("**"),
            KeyCode::Char('i') if matches!(app.overlay, Overlay::None) => app.emphasis("*"),
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
            KeyCode::Char('z' | 'Z' | 'y')
                if !matches!(app.overlay, Overlay::None | Overlay::Find { .. }) => {}
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

    // F1 is the help from anywhere; `?` too, wherever it wouldn't be typed.
    let in_help = matches!(app.overlay, Overlay::Help { .. });
    if (key == Key::F(1) && !in_help) || (key == Key::Char('?') && app.question_opens_help()) {
        app.open_help(None);
        return false;
    }

    if !matches!(app.overlay, Overlay::None) {
        app.on_overlay_key(key);
        if std::mem::take(&mut app.quit) && quit_or_arm(app, confirm_quit) {
            return true;
        }
        return false;
    }

    // Esc is the menu, from any pane.
    if key == Key::Esc {
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

    std::mem::take(&mut app.quit) && quit_or_arm(app, confirm_quit)
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

#[cfg(test)]
mod first_start_tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grimoire-first-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_very_first_book_is_made_in_the_books_folder() {
        let d = tmp("new");
        let home = d.join("Grimoire Books");
        let (root, said) = first_start(&home, &d.join("Grimoire")).unwrap();
        assert_eq!(root, home.join("My First Book"));
        assert!(is_project(&root));
        assert!(said.unwrap().starts_with("your first book is ready"));
        // The next start with nothing remembered opens it; it makes no second.
        let (again, said) = first_start(&home, &d.join("Grimoire")).unwrap();
        assert_eq!(again, root);
        assert!(said.unwrap().contains("from your books folder"));
        assert_eq!(grimoire_core::shelf::books_in(&home).len(), 1);
    }

    #[test]
    fn a_book_from_before_the_books_folder_still_opens_where_it_is() {
        let d = tmp("older");
        let older = d.join("Grimoire");
        std::fs::create_dir_all(&older).unwrap();
        project::scaffold(&older).unwrap();
        let home = d.join("Grimoire Books");
        let (root, said) = first_start(&home, &older).unwrap();
        assert_eq!(root, older);
        assert_eq!(said, None);
        assert!(!home.exists(), "nothing new was made");
    }
}
