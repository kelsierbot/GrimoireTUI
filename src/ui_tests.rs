//! The whole desk, driven the way a writer drives it: keys go through the same
//! routing as the running app (`on_key`), and every press is followed by a
//! real frame drawn into ratatui's test backend, so what's asserted is what
//! would be on screen. Each test gets a book of its own in a temp dir, and
//! nothing reads or writes ~/.config.

use super::*;
use crate::app::Setup;
use grimoire_core::recovery;
use grimoire_core::settings::Settings;
use grimoire_core::spell;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;
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

// ---- themes ---------------------------------------------------------------

fn fg_colours_in_row(d: &Desk, y: u16) -> Vec<ratatui::style::Color> {
    let buf = d.term.backend().buffer();
    let mut v: Vec<ratatui::style::Color> = (0..buf.area.width).map(|x| buf[(x, y)].fg).collect();
    v.sort_by_key(|c| format!("{c:?}"));
    v.dedup();
    v
}

#[test]
fn the_rainbow_theme_paints_its_accent_as_a_spectrum() {
    let mut d = Desk::open(book("rainbow", true), 120, 35);
    d.app.theme = theme::presets()
        .into_iter()
        .find(|t| t.is_rainbow())
        .unwrap();
    d.draw();
    let accent = d.app.theme.accent;
    let buf = d.term.backend().buffer();
    let flat = (0..buf.area.height)
        .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
        .filter(|&(x, y)| buf[(x, y)].fg == accent || buf[(x, y)].bg == accent)
        .count();
    assert_eq!(flat, 0, "no cell is left in the flat accent");
    // The focused tree's top border runs through several hues.
    assert!(
        fg_colours_in_row(&d, 0).len() >= 5,
        "the border is one colour"
    );
}

#[test]
fn a_flat_theme_keeps_its_accent() {
    let d = Desk::open(book("flat-accent", true), 120, 35);
    let accent = d.app.theme.accent;
    let buf = d.term.backend().buffer();
    assert_eq!(buf[(0, 0)].fg, accent, "the focused border is the accent");
}

#[test]
fn the_theme_picker_scrolls_on_a_short_terminal() {
    let mut d = Desk::open(book("picker", true), 80, 24);
    d.key(KeyCode::F(9));
    assert!(matches!(d.app.overlay, Overlay::Themes { .. }));
    assert!(d.shows("Grimoire"), "starts at the top");
    assert!(d.shows("↓ more"));
    // Up from the first wraps to the last: Custom…, with Rainbow above it.
    d.key(KeyCode::Up);
    assert!(d.shows("Custom…"), "the last row scrolled into view");
    assert!(d.shows("Rainbow"));
    assert!(d.shows("↑ more"));
    assert_eq!(d.app.theme.name, "Custom", "previewing the custom slot");
    d.key(KeyCode::Up);
    assert_eq!(d.app.theme.name, "Rainbow", "previewed as you move");
    d.key(KeyCode::Esc);
    assert_eq!(d.app.theme.name, "Grimoire", "Esc puts the old theme back");
}

// ---- the book at the top of the tree, and where focus mode is ----------------

#[test]
fn a_tall_tree_is_headed_by_the_spellbook_and_clicks_still_land() {
    let mut d = Desk::open(book("art-tall", true), 120, 42);
    assert!(d.shows("`───────────┴───────────'"), "the book is drawn");
    let rows = d.rows();
    let art = rows.iter().position(|r| r.contains("┴")).unwrap();
    let novel = rows
        .iter()
        .position(|r| r.contains("Novel Format"))
        .unwrap();
    assert!(novel > art, "the tree starts below the book");
    // Clicking Scene Two's row opens Scene Two, not whatever is a book's
    // height further down the list.
    let left = |r: &String| r.chars().take(30).collect::<String>();
    let y = rows
        .iter()
        .position(|r| left(r).contains("Scene Two"))
        .unwrap() as u16;
    let x = rows[y as usize].chars().position(|c| c == 'S').unwrap() as u16;
    d.app.on_click(x, y);
    d.draw();
    let open = d.app.open.map(|i| d.app.project.nodes[i].title.clone());
    assert_eq!(open.as_deref(), Some("Scene Two"));
}

#[test]
fn a_short_tree_keeps_its_rows_and_skips_the_book() {
    let d = Desk::open(book("art-short", true), 80, 24);
    assert!(!d.shows("┴───"), "no book on a short terminal");
    assert!(
        d.rows()[1].contains("Novel Format"),
        "the tree starts at the top"
    );
}

#[test]
fn focus_mode_is_named_where_it_can_be_used() {
    let mut d = Desk::open(book("focus-hint", true), 140, 42);
    assert!(
        d.shows("F2 timer · ^D focus mode"),
        "the idle timer says where focus mode is"
    );
    d.key(KeyCode::Tab);
    assert_eq!(d.app.focus, Focus::Editor);
    assert!(
        d.status().contains("Esc menu  ^D focus mode"),
        "{}",
        d.status()
    );
    // Running, the timer is "writing", so "focus" only ever means focus mode.
    d.key(KeyCode::F(2));
    assert!(d.shows("writing · "), "timer label while running");
    assert!(!d.rows().iter().any(|r| r.contains("┌ focus ·")));
}

#[test]
fn the_music_pane_says_its_keys_when_focused() {
    let root = book("music-keys", true);
    let setup = Setup {
        // On but unconfigured: the pane shows, nothing is polled.
        music: music::Config {
            enabled: true,
            ..music::Config::default()
        },
        theme: theme::default_theme(),
        settings: Settings::default(),
        background: false,
    };
    let app = App::with(Project::load(&root).unwrap(), setup).unwrap();
    let mut d = Desk {
        app,
        term: Terminal::new(TestBackend::new(140, 42)).unwrap(),
        armed: false,
        left: false,
        root,
    };
    d.draw();
    assert!(!d.shows("[ prev · space ⏯ · ] next"), "only while focused");
    for _ in 0..5 {
        if d.app.focus == Focus::Music {
            break;
        }
        d.key(KeyCode::Tab);
    }
    assert_eq!(d.app.focus, Focus::Music);
    assert!(
        d.shows("[ prev · space ⏯ · ] next"),
        "the pane's edge names the keys"
    );
    assert!(d.status().contains("[ ] track"), "{}", d.status());
}

// ---- spelling: click a misspelt word ----------------------------------------

/// A book whose first scene reads `body`, open on it with the dictionary
/// loaded (tests start no background work, so it's built here).
fn spelling_desk(tag: &str, body: &str) -> Desk {
    let root = book(tag, true);
    let scene = first_scene(&root);
    let text = fs::read_to_string(&scene).unwrap();
    let front_end = text.find("\n---\n").map(|i| i + 5).unwrap_or(0);
    fs::write(&scene, format!("{}\n{body}\n", &text[..front_end])).unwrap();
    let mut d = Desk::open(root, 120, 35);
    d.app.load_speller_now();
    d.draw();
    d
}

/// Where `needle` is on screen, (x, y), searching right of the tree.
fn at(d: &Desk, needle: &str) -> (u16, u16) {
    for (y, r) in d.rows().iter().enumerate() {
        let chars: Vec<char> = r.chars().collect();
        let n: Vec<char> = needle.chars().collect();
        for x in 31..chars.len().saturating_sub(n.len()) {
            if chars[x..x + n.len()] == n[..] {
                return (x as u16, y as u16);
            }
        }
    }
    panic!("{needle:?} not on screen:\n{}", d.rows().join("\n"));
}

fn click(d: &mut Desk, needle: &str) {
    let (x, y) = at(d, needle);
    d.app.on_click(x + 1, y);
    d.draw();
}

fn popup_open(d: &Desk) -> bool {
    matches!(d.app.overlay, Overlay::Spelling { inline: true, .. })
}

fn scene_text(d: &Desk) -> String {
    d.app.editor.text()
}

#[test]
fn clicking_a_misspelt_word_offers_its_fixes_and_one_undo_takes_one_back() {
    let mut d = spelling_desk("spell-fix", "Teh cat sat.");
    click(&mut d, "Teh cat");
    assert!(popup_open(&d), "the popup opens on the word");
    assert!(d.shows("“Teh”"));
    assert!(d.shows("Always ignore — every book"));
    // "The", in the word's own capitalisation, is on offer; clicking it fixes.
    click(&mut d, "The ");
    assert!(!popup_open(&d));
    assert!(
        scene_text(&d).contains("The cat sat."),
        "{}",
        scene_text(&d)
    );
    let i = d.app.open.unwrap();
    assert!(d.app.project.nodes[i].dirty, "autosave sees the change");
    d.ctrl('z');
    assert!(
        scene_text(&d).contains("Teh cat sat."),
        "one undo takes it back"
    );
}

#[test]
fn right_click_opens_the_same_popup() {
    let mut d = spelling_desk("spell-right", "Teh cat sat.");
    let (x, y) = at(&d, "Teh cat");
    d.app.on_right_click(x + 1, y);
    d.draw();
    assert!(popup_open(&d));
}

#[test]
fn always_ignore_in_this_book_writes_its_list_and_lifts_the_underline() {
    let mut d = spelling_desk("spell-book", "Qwyrk waits.");
    let (x, y) = at(&d, "Qwyrk");
    assert!(
        d.term.backend().buffer()[(x, y)]
            .modifier
            .contains(Modifier::UNDERLINED),
        "underlined to begin with"
    );
    click(&mut d, "Qwyrk");
    click(&mut d, "Always ignore — this book");
    let list = fs::read_to_string(d.root.join("dictionary.txt")).unwrap_or_default();
    assert!(list.lines().any(|l| l == "Qwyrk"), "{list}");
    d.key(KeyCode::End); // off the word, so it would be underlined again
    let (x, y) = at(&d, "Qwyrk");
    assert!(
        !d.term.backend().buffer()[(x, y)]
            .modifier
            .contains(Modifier::UNDERLINED),
        "no longer underlined"
    );
}

#[test]
fn the_every_book_list_is_kept_and_read_by_the_next_launch() {
    let dir = std::env::temp_dir().join(format!("grimoire-ui-userdict-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let list = dir.join("dictionary.txt");
    let mut d = spelling_desk("spell-user", "Zorblat waits.");
    d.app.user_dict = Some(list.clone());
    click(&mut d, "Zorblat");
    click(&mut d, "Always ignore — every book");
    assert!(spell::words_in(&list).contains(&"Zorblat".to_string()));
    // A fresh app — another book, even — accepts it once it reads the list.
    let mut other = spelling_desk("spell-user-2", "Zorblat returns.");
    let line = other
        .app
        .editor
        .lines
        .iter()
        .position(|l| l.contains("Zorblat"))
        .unwrap();
    assert!(
        !other.app.misspellings(line).is_empty(),
        "without the list it's a misspelling"
    );
    other.app.user_dict = Some(list.clone());
    other.app.load_speller_now();
    assert!(
        other.app.misspellings(line).is_empty(),
        "accepted in every book"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_letter_closes_the_popup_and_is_typed() {
    let mut d = spelling_desk("spell-type", "Teh cat sat.");
    click(&mut d, "Teh cat");
    assert!(popup_open(&d));
    d.key(KeyCode::Char('x'));
    assert!(!popup_open(&d), "typing puts the popup away");
    assert!(
        scene_text(&d).contains('x'),
        "and the letter lands in the prose"
    );
}

#[test]
fn clicking_a_correct_word_or_a_note_opens_nothing() {
    let mut d = spelling_desk("spell-none", "%% Teh %% the cat sat.");
    click(&mut d, "cat sat");
    assert!(!popup_open(&d), "a correct word just places the caret");
    click(&mut d, "Teh %%");
    assert!(!popup_open(&d), "notes aren't prose to correct");
}

#[test]
fn f8_offers_the_same_ways_to_ignore() {
    let mut d = spelling_desk("spell-f8", "Teh cat sat.");
    d.key(KeyCode::Tab);
    d.key(KeyCode::F(8));
    assert!(matches!(
        d.app.overlay,
        Overlay::Spelling { inline: false, .. }
    ));
    assert!(d.shows("Ignore for now"));
    assert!(d.shows("Always ignore — this book"));
    assert!(d.shows("Always ignore — every book"));
}

/// Not a test: `GRIMOIRE_SCENE_DUMP=<file> cargo test dump_pomodoro -- --ignored`
/// writes every theme's Pomodoro (idle, midday, night) as JSON cells with
/// their colours, for rendering a contact sheet to look at.
#[test]
#[ignore]
fn dump_pomodoro_previews() {
    let Ok(out) = std::env::var("GRIMOIRE_SCENE_DUMP") else {
        return;
    };
    let frame: u64 = std::env::var("GRIMOIRE_SCENE_FRAME")
        .ok()
        .and_then(|f| f.parse().ok())
        .unwrap_or(0);
    let hex = |c: ratatui::style::Color| match c {
        ratatui::style::Color::Rgb(r, g, b) => format!("{r:02x}{g:02x}{b:02x}"),
        _ => "ffffff".into(),
    };
    let mut themes = Vec::new();
    for t in theme::presets() {
        let mut shots = Vec::new();
        for (label, phase, p) in [
            ("idle", scene::Phase::Idle, 0.0),
            ("writing", scene::Phase::Focus, 0.42),
            ("break", scene::Phase::Break, 0.55),
        ] {
            let g = scene::render(scene::Scenery::for_theme(&t.name), phase, p, false, frame);
            let night = phase == scene::Phase::Break;
            let rows: Vec<Vec<(String, String)>> = g
                .iter()
                .enumerate()
                .map(|(y, row)| {
                    row.iter()
                        .enumerate()
                        .map(|(x, &(ch, ink))| {
                            let mut c = ui::scene_colour(ink, &t, night);
                            if t.is_rainbow() && c == t.accent {
                                c = theme::hue(x as f32 * 3.0 + y as f32 * 7.0);
                            }
                            (ch.to_string(), hex(c))
                        })
                        .collect()
                })
                .collect();
            shots.push(serde_json::json!({ "label": label, "rows": rows }));
        }
        themes.push(serde_json::json!({ "name": t.name, "border": hex(t.border), "shots": shots }));
    }
    fs::write(out, serde_json::to_string(&themes).unwrap()).unwrap();
}

// ---- the Pomodoro and the Visualizer ---------------------------------------

#[test]
fn every_theme_draws_its_own_pomodoro_through_the_real_desk() {
    let mut d = Desk::open(book("pomodoro-worlds", true), 140, 42);
    let mut seen: Vec<Vec<String>> = Vec::new();
    for t in theme::presets() {
        d.app.theme = t.clone();
        d.app.pomo.reset();
        d.draw();
        let pane = |d: &Desk| -> Vec<String> {
            let r = d.app.rect_scene;
            let rows = d.rows();
            (r.y..r.y + r.height)
                .map(|y| {
                    rows[y as usize]
                        .chars()
                        .skip(r.x as usize)
                        .take(r.width as usize)
                        .collect()
                })
                .collect()
        };
        let idle = pane(&d);
        assert!(
            !seen.contains(&idle),
            "{} looks like another theme's world",
            t.name
        );
        seen.push(idle);
        // Running, and on to night, across a spread of frames: no panics, and
        // the title keeps its words.
        d.key(KeyCode::F(2));
        for f in (0..300).step_by(37) {
            d.app.frame = f;
            d.draw();
        }
        assert!(d.shows("writing · "), "{}", t.name);
        d.app.pomo.phase = scene::Phase::Break;
        for f in (0..300).step_by(41) {
            d.app.frame = f;
            d.draw();
        }
        assert!(d.shows("break · "), "{}", t.name);
    }
}

#[test]
fn the_pane_switches_between_pomodoro_and_visualizer_and_there_is_no_garden() {
    let mut d = Desk::open(book("two-views", true), 140, 42);
    assert!(
        d.shows("pomodoro visualizer"),
        "the switcher names both views"
    );
    assert_eq!(d.app.pane_mode, scene::Mode::Pomodoro);
    for _ in 0..6 {
        if d.app.focus == Focus::Clearing {
            break;
        }
        d.key(KeyCode::Tab);
    }
    assert_eq!(d.app.focus, Focus::Clearing);
    d.key(KeyCode::Right);
    assert_eq!(d.app.pane_mode, scene::Mode::Visualizer);
    d.key(KeyCode::Right);
    assert_eq!(
        d.app.pane_mode,
        scene::Mode::Pomodoro,
        "two views, round and back"
    );
    d.key(KeyCode::Left);
    assert_eq!(d.app.pane_mode, scene::Mode::Visualizer);
    for word in ["garden", "clearing", "spectrum"] {
        assert!(!d.shows(word), "{word} is still on screen");
    }
}
