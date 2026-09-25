//! The whole desk, driven the way a writer drives it: keys go through the same
//! routing as the running app (`on_key`), and every press is followed by a
//! real frame drawn into ratatui's test backend, so what's asserted is what
//! would be on screen. Each test gets a book of its own in a temp dir, and
//! nothing reads or writes ~/.config.

use super::*;
use crate::app::Setup;
use grimoire_core::recovery;
use grimoire_core::settings::Settings;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::fs;

struct Desk {
    app: App,
    term: Terminal<TestBackend>,
    armed: bool,
    left: bool,
    root: PathBuf,
}

/// A new book, scaffolded like `grimoire new`. `seen` makes it look opened
/// before, so it launches on its first scene rather than the guide.
fn book(tag: &str, seen: bool) -> PathBuf {
    let root = std::env::temp_dir().join(format!("grimoire-ui-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    project::scaffold(&root).unwrap();
    if seen {
        fs::create_dir_all(root.join(".grimoire")).unwrap();
        fs::write(root.join(".grimoire/progress.toml"), "").unwrap();
    }
    root
}

fn first_scene(root: &Path) -> PathBuf {
    let mut ch = root.join("manuscript");
    for _ in 0..2 {
        let mut dirs: Vec<PathBuf> = fs::read_dir(&ch)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        ch = dirs[0].clone();
    }
    ch.join("01-Scene-One.md")
}

impl Desk {
    fn open(root: PathBuf, w: u16, h: u16) -> Desk {
        let setup = Setup {
            music: music::Config::default(),
            theme: theme::default_theme(),
            settings: Settings::default(),
            background: false,
        };
        let app = App::with(Project::load(&root).unwrap(), setup).unwrap();
        let mut d = Desk {
            app,
            term: Terminal::new(TestBackend::new(w, h)).unwrap(),
            armed: false,
            left: false,
            root,
        };
        d.draw();
        d
    }

    fn draw(&mut self) {
        let app = &mut self.app;
        self.term.draw(|f| ui::draw(f, app)).unwrap();
    }

    fn press(&mut self, k: KeyEvent) {
        if on_key(&mut self.app, k, &mut self.armed) {
            self.left = true;
        }
        self.draw();
    }

    fn key(&mut self, code: KeyCode) {
        self.press(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn keys(&mut self, codes: &[KeyCode]) {
        for &c in codes {
            self.key(c);
        }
    }

    fn ctrl(&mut self, c: char) {
        self.press(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }

    fn typed(&mut self, s: &str) {
        for c in s.chars() {
            self.key(KeyCode::Char(c));
        }
    }

    fn rows(&self) -> Vec<String> {
        let buf = self.term.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect()
    }

    fn shows(&self, s: &str) -> bool {
        self.rows().iter().any(|r| r.contains(s))
    }

    fn status(&self) -> String {
        self.rows().last().cloned().unwrap_or_default()
    }

    fn menu_open(&self) -> bool {
        matches!(self.app.overlay, Overlay::Menu { .. })
    }
}

impl Drop for Desk {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

// ---- Esc and the menu ---------------------------------------------------

#[test]
fn esc_from_the_tree_opens_the_menu_and_esc_closes_it() {
    let mut d = Desk::open(book("esc-tree", true), 120, 35);
    assert_eq!(d.app.focus, Focus::Tree);
    d.key(KeyCode::Esc);
    assert!(d.menu_open());
    assert!(d.shows("Quit Grimoire"));
    d.key(KeyCode::Esc);
    assert_eq!(d.app.overlay, Overlay::None);
    assert!(!d.shows("Quit Grimoire"));
    assert_eq!(d.app.focus, Focus::Tree);
}

#[test]
fn closing_the_menu_leaves_you_in_the_editor() {
    let mut d = Desk::open(book("esc-editor", true), 120, 35);
    d.key(KeyCode::Tab);
    assert_eq!(d.app.focus, Focus::Editor);
    d.key(KeyCode::Esc);
    assert!(d.menu_open());
    d.key(KeyCode::Esc);
    assert_eq!(d.app.overlay, Overlay::None);
    assert_eq!(d.app.focus, Focus::Editor);
    assert!(d.status().contains("F8 spelling"), "{}", d.status());
}

#[test]
fn f1_opens_the_menu_too() {
    let mut d = Desk::open(book("f1", true), 120, 35);
    d.key(KeyCode::F(1));
    assert!(d.menu_open());
}

#[test]
fn esc_clears_a_selection_before_it_opens_the_menu() {
    let mut d = Desk::open(book("esc-sel", true), 120, 35);
    d.key(KeyCode::Tab);
    d.typed("Some words");
    // Drag across the first word, as with the mouse.
    let r = d.app.rect_editor;
    d.app.on_click(r.x, r.y);
    d.app.on_drag(r.x + 4, r.y);
    d.draw();
    assert!(d.app.editor.has_selection());
    d.key(KeyCode::Esc);
    assert!(!d.app.editor.has_selection());
    assert!(!d.menu_open());
    d.key(KeyCode::Esc);
    assert!(d.menu_open());
}

#[test]
fn quit_grimoire_saves_and_leaves() {
    let root = book("quit", true);
    let scene = first_scene(&root);
    let mut d = Desk::open(root, 120, 35);
    d.key(KeyCode::Tab);
    d.typed("Zyx marker");
    d.key(KeyCode::Esc);
    // Up from the first row wraps to the last: Quit Grimoire.
    d.key(KeyCode::Up);
    assert!(d.shows("▸ Quit Grimoire"));
    d.key(KeyCode::Enter);
    assert!(d.left, "Enter on Quit Grimoire leaves");
    let saved = fs::read_to_string(&scene).unwrap();
    assert!(saved.contains("Zyx marker"), "{saved}");
}

#[test]
fn the_menu_acts_on_the_scene_being_written() {
    let mut d = Desk::open(book("menu-target", true), 120, 35);
    // The tree's cursor wanders off to Scene Three; then back to writing.
    d.keys(&[KeyCode::Down, KeyCode::Down]);
    d.key(KeyCode::Tab);
    d.key(KeyCode::Esc);
    let at = d
        .app
        .menu()
        .iter()
        .position(|(_, a)| *a == palette::Action::Delete)
        .unwrap();
    for _ in 0..at {
        d.key(KeyCode::Down);
    }
    d.key(KeyCode::Enter);
    match &d.app.overlay {
        Overlay::Confirm { name, .. } => assert_eq!(name, "Scene One"),
        other => panic!("expected the delete prompt, got {other:?}"),
    }
}

#[test]
fn the_menu_offers_the_writing_tools_and_hides_what_cant_act() {
    let d = Desk::open(book("menu-rows", true), 120, 35);
    let labels: Vec<String> = d.app.menu().into_iter().map(|(l, _)| l).collect();
    for want in [
        "Focus mode",
        "Open a scene beside…",
        "Notes & TKs…",
        "Next scene still in draft",
        "Turn echo words on",
        "Start a sprint…",
        "Export…",
        "Settings…",
    ] {
        assert!(
            labels.iter().any(|l| l.starts_with(want)),
            "no {want} in {labels:?}"
        );
    }
    assert!(
        !labels.iter().any(|l| l.contains("Music player")),
        "music is off"
    );
    assert!(labels.last().unwrap().starts_with("Quit Grimoire"));
    assert!(labels[labels.len() - 2].starts_with("Close"));
}

#[test]
fn settings_and_back_again() {
    let mut d = Desk::open(book("settings", true), 120, 35);
    d.key(KeyCode::Esc);
    let at = d
        .app
        .menu()
        .iter()
        .position(|(_, a)| *a == palette::Action::Settings)
        .unwrap();
    for _ in 0..at {
        d.key(KeyCode::Down);
    }
    d.key(KeyCode::Enter);
    assert!(matches!(d.app.overlay, Overlay::Settings { .. }));
    assert!(d.shows("Line width: 72 columns"));
    assert!(d.shows("typewriter scrolling"));
    d.key(KeyCode::Esc);
    assert_eq!(d.app.overlay, Overlay::Menu { sel: at });
}

#[test]
fn a_short_terminal_scrolls_the_menu_to_quit() {
    let mut d = Desk::open(book("short", true), 80, 16);
    d.key(KeyCode::Esc);
    assert!(d.shows("Find anything"));
    assert!(d.shows("esc close"), "the key hints stay in view");
    d.key(KeyCode::Up);
    assert!(d.shows("▸ Quit Grimoire"));
}

#[test]
fn the_palette_runs_the_same_actions() {
    let mut d = Desk::open(book("palette", true), 120, 35);
    d.ctrl('k');
    assert!(matches!(d.app.overlay, Overlay::Palette { .. }));
    d.typed("focus mode");
    d.key(KeyCode::Enter);
    assert!(d.app.focus_mode);
}

// ---- the status bar ------------------------------------------------------

#[test]
fn at_80_columns_the_status_bar_leads_with_esc() {
    let mut d = Desk::open(book("status80", true), 80, 24);
    assert!(d.status().contains("Esc menu"), "tree: {}", d.status());
    d.key(KeyCode::Tab);
    assert!(d.status().contains("Esc menu"), "editor: {}", d.status());
}

// ---- notes beside the scene ------------------------------------------------

fn with_wren(tag: &str) -> Desk {
    let root = book(tag, true);
    fs::write(
        root.join("characters/wren.md"),
        "---\ntitle: Wren\n---\nA courier.\n",
    )
    .unwrap();
    let mut d = Desk::open(root, 120, 35);
    d.key(KeyCode::Tab);
    d.typed("Wren");
    d.key(KeyCode::Left);
    d.ctrl('o');
    assert!(d.app.codex.is_some(), "Ctrl-O on a name opens its note");
    assert!(d.shows("A courier."));
    d
}

#[test]
fn esc_closes_an_open_note_from_the_editor() {
    let mut d = with_wren("note-esc");
    assert_eq!(d.app.focus, Focus::Editor);
    assert!(d.status().contains("Esc close note"), "{}", d.status());
    d.key(KeyCode::Esc);
    assert!(d.app.codex.is_none());
    assert!(!d.menu_open(), "the first Esc only closes the note");
    d.key(KeyCode::Esc);
    assert!(d.menu_open());
}

#[test]
fn ctrl_o_again_puts_the_note_away() {
    let mut d = with_wren("note-toggle");
    d.ctrl('o');
    assert!(d.app.codex.is_none());
}

// ---- focus mode ------------------------------------------------------------

#[test]
fn focus_mode_hides_the_panes_and_comes_back() {
    let mut d = Desk::open(book("focus", true), 120, 35);
    assert!(d.shows("Manuscript"), "the tree is up");
    d.key(KeyCode::Tab);
    d.ctrl('d');
    assert!(d.app.focus_mode);
    assert!(!d.shows("Manuscript"), "the tree is hidden");
    assert!(!d.shows("clearing"), "so is the clearing");
    // Tab can't reach what isn't there.
    d.key(KeyCode::Tab);
    assert_eq!(d.app.focus, Focus::Editor);
    // Esc still opens the menu, and closing it goes back to focus mode.
    d.key(KeyCode::Esc);
    assert!(d.menu_open());
    d.key(KeyCode::Esc);
    assert!(d.app.focus_mode);
    d.ctrl('d');
    assert!(!d.app.focus_mode);
    assert!(d.shows("Manuscript"));
}

// ---- a new book --------------------------------------------------------------

#[test]
fn a_book_never_opened_starts_on_its_guide() {
    let d = Desk::open(book("guide", false), 120, 35);
    let open = d.app.open.expect("something is open");
    assert_eq!(d.app.project.nodes[open].area, project::Area::Format);
    assert!(d.status().contains("a new book"), "{}", d.status());
}

#[test]
fn a_book_opened_before_starts_on_its_first_scene() {
    let d = Desk::open(book("no-guide", true), 120, 35);
    let open = d.app.open.expect("something is open");
    assert_eq!(d.app.project.nodes[open].title, "Scene One");
}

#[test]
fn an_empty_scene_says_how_to_start() {
    let d = Desk::open(book("cue", true), 120, 35);
    assert!(d.shows("Tab or click to write"), "cue on an empty scene");
}

// ---- notes in the prose ----------------------------------------------------

#[test]
fn notes_and_tks_are_left_out_of_the_count() {
    let root = book("notes-count", true);
    let scene = first_scene(&root);
    let text = fs::read_to_string(&scene).unwrap();
    fs::write(
        &scene,
        format!("{text}One two %% three four %% five TK six.\n"),
    )
    .unwrap();
    let d = Desk::open(root, 120, 35);
    assert_eq!(d.app.project.total_words(), 4);
}

// ---- recovered words ---------------------------------------------------------

#[test]
fn letters_never_decide_what_happens_to_recovered_words() {
    let root = book("recover", true);
    let scene = first_scene(&root);
    recovery::keep(
        &root,
        &scene,
        "---\ntitle: Scene One\n---\nWords from before.\n",
    )
    .unwrap();
    let mut d = Desk::open(root.clone(), 120, 35);
    assert!(matches!(d.app.overlay, Overlay::Recover { .. }));
    for c in ['n', 'N', 'y', 'q', 'x'] {
        d.key(KeyCode::Char(c));
        assert!(
            matches!(d.app.overlay, Overlay::Recover { .. }),
            "{c} must not decide"
        );
    }
    let project = Project::load(&root).unwrap();
    assert_eq!(recovery::pending(&project).len(), 1, "still recoverable");
    // Enter on the first choice restores them.
    d.key(KeyCode::Enter);
    assert_eq!(d.app.overlay, Overlay::None);
    let saved = fs::read_to_string(&scene).unwrap();
    assert!(saved.contains("Words from before."), "{saved}");
}

#[test]
fn deciding_later_keeps_the_recovered_words() {
    let root = book("recover-later", true);
    let scene = first_scene(&root);
    recovery::keep(&root, &scene, "---\ntitle: Scene One\n---\nLater words.\n").unwrap();
    let mut d = Desk::open(root.clone(), 120, 35);
    d.key(KeyCode::Esc);
    assert_eq!(d.app.overlay, Overlay::None);
    let project = Project::load(&root).unwrap();
    assert_eq!(recovery::pending(&project).len(), 1);
}

// ---- lists ------------------------------------------------------------------

#[test]
fn moving_through_themes_previews_each_and_esc_puts_yours_back() {
    let mut d = Desk::open(book("themes", true), 120, 35);
    let mine = d.app.theme.name.clone();
    d.key(KeyCode::F(9));
    assert!(matches!(d.app.overlay, Overlay::Themes { .. }));
    d.key(KeyCode::Down);
    assert_ne!(d.app.theme.name, mine, "the next theme shows at once");
    d.key(KeyCode::Char('k'));
    assert_eq!(d.app.theme.name, mine);
    d.key(KeyCode::Up);
    assert_ne!(d.app.theme.name, mine, "and it goes round the list");
    d.key(KeyCode::Esc);
    assert_eq!(d.app.theme.name, mine);
}
