//! Application state and key handling.

use anyhow::Result;
use ratatui::layout::Rect;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::music::{self, Music};
use crate::palette::{self, Action};
use crate::scene::{Mode, Pomodoro};
use crate::theme::{self, Theme};
use crate::visualizer::Visualizer;
use grimoire_core::codex;
use grimoire_core::cork;
use grimoire_core::create::{self, New, Plan};
use grimoire_core::editor::{self, Editor};
use grimoire_core::export;
use grimoire_core::history;
use grimoire_core::manuscript;
use grimoire_core::project::{self, DiskChange, Kind, Project};
use grimoire_core::recovery;
use grimoire_core::resume;
use grimoire_core::search;
use grimoire_core::sessions;
use grimoire_core::settings::Settings;
use grimoire_core::spell;

mod aids;
pub(crate) mod overlays;
pub use aids::{MarkRow, Sprint, filter_marks as aids_filter};

/// Save a couple of seconds after typing stops…
const AUTOSAVE_IDLE: Duration = Duration::from_secs(2);
/// …and never let unsaved words sit longer than this, however fast you type.
const AUTOSAVE_MAX: Duration = Duration::from_secs(20);
/// After a failed save, try again this often rather than on every tick.
const RETRY: Duration = Duration::from_secs(10);
/// How often to look for changes made to the book by something else — a
/// sync client, Obsidian, the phone. A `stat` per scene; files are only read
/// when their size or time moved.
const DISK_CHECK: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq)]
pub enum SaveState {
    Clean,
    Saved(Instant),
    Failed(String),
    /// Words that can't be written yet — the file can't be read right now,
    /// or went missing for a moment — held in memory and recovery until it
    /// can be checked. Not a failure; retried on the same backoff.
    Waiting(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Editor,
    /// The note open beside the scene.
    Codex,
    /// Another scene, read-only, beside the one being written.
    Beside,
    Clearing,
    Music,
}

/// Something done to the book's files from the tree, kept so it can be taken
/// back.
/// The recovered-words choices, in the order they're listed.
pub const RECOVER_CHOICES: [&str; 3] = [
    "Restore them — the saved versions go into each scene's history",
    "Keep the saved versions — the recovered words go to the trash",
    "Decide later — offered again next time",
];

#[derive(Debug, Clone)]
enum TreeStep {
    /// Things moved on disk: a delete (to the trash) or a move. Each batch was
    /// one rename-all; a drag is several. `links` if `[[links]]` followed.
    Moved {
        what: String,
        batches: Vec<Vec<(PathBuf, PathBuf)>>,
        links: bool,
    },
    /// A rename, by name, so a scene's `title:` goes back too.
    Renamed {
        from: PathBuf,
        to: PathBuf,
        old: String,
        new: String,
    },
    /// The sections were put in a new order.
    Sections {
        what: String,
        before: Vec<grimoire_core::project::Area>,
        after: Vec<grimoire_core::project::Area>,
    },
    /// Something new. Undoing it sends it to the trash (where it waits, words
    /// and all), and redoing brings it back from there.
    Created {
        what: String,
        path: PathBuf,
        trashed: Option<PathBuf>,
    },
    /// A replace across the book: each changed scene's text before and after,
    /// so one Ctrl-Z takes the whole thing back.
    Replaced {
        what: String,
        scenes: Vec<(PathBuf, String, String)>,
    },
}

/// What an App starts from that doesn't live in the book: the music, theme
/// and settings files in ~/.config, and whether to start its background
/// work (loading the dictionary, backing up writing sessions).
pub struct Setup {
    pub music: music::Config,
    pub theme: Theme,
    pub settings: Settings,
    pub background: bool,
}

impl Setup {
    /// What a real launch uses: this user's own config, background work on.
    pub fn from_config() -> Setup {
        Setup {
            music: music::Config::load(),
            theme: theme::load(),
            settings: Settings::load(),
            background: true,
        }
    }
}

pub struct App {
    pub project: Project,
    pub visible: Vec<usize>,
    pub parents: Vec<Option<usize>>,
    pub sel: usize,
    pub tree_scroll: usize,
    pub open: Option<usize>,
    pub editor: Editor,
    pub focus: Focus,
    /// The manuscript's word count at the start of today: "today" is what it
    /// has grown by since. See [`App::tick_today`].
    pub baseline: usize,
    baseline_date: String,
    pub msg: String,
    pub quit: bool,
    pub edit_width: usize,
    pub edit_height: usize,
    pub pomo: Pomodoro,
    /// What the small pane under the tree is showing. ←/→ cycles it.
    pub pane_mode: Mode,
    pub music: Music,
    /// Listens to system audio, but only while the spectrum view is showing.
    pub viz: Visualizer,
    /// When the player last asked for the queue, so it stays current while open.
    pub player_fetched: Option<std::time::Instant>,
    /// Animation counter, bumped once per event-loop tick.
    pub frame: u64,
    /// True when the terminal can actually report Cmd/Super — which needs the
    /// Kitty keyboard protocol. Ctrl always works regardless.
    pub super_keys: bool,
    /// Set during draw: short terminals drop these panes, and Tab must not
    /// focus something that isn't on screen.
    pub scene_visible: bool,
    pub music_visible: bool,
    /// Inner rects recorded during draw, so clicks can be routed to a pane.
    pub rect_tree: Rect,
    pub rect_editor: Rect,
    pub rect_scene: Rect,
    pub rect_music: Rect,
    /// Where the create keys on the tree's bottom edge were drawn, so a
    /// click on one does what pressing it would.
    pub create_hits: Vec<(Rect, New)>,
    /// Where the view switcher on the scene pane's bottom edge was drawn:
    /// each view's name, and the arrows either side.
    pub view_hits: Vec<(Rect, Mode)>,
    pub theme: Theme,
    pub overlay: Overlay,
    /// Autosave bookkeeping: the last keystroke that changed text, when the
    /// oldest unsaved change was made, and how the last save went.
    pub last_edit: Option<Instant>,
    pub unsaved_since: Option<Instant>,
    pub save_state: SaveState,
    last_attempt: Option<Instant>,
    /// Scenes whose pre-session text is already in history.
    pre_snapshotted: HashSet<PathBuf>,
    /// Undo for scenes that aren't open, so switching back still undoes.
    undo_stash: HashMap<PathBuf, editor::History>,
    /// What was done in the tree — deletes, renames, moves, new things — so
    /// Ctrl-Z outside the editor can take it back, and Ctrl-Y put it again.
    tree_undo: Vec<TreeStep>,
    tree_redo: Vec<TreeStep>,
    /// How the find bar matches (whole words, exact case unless loosened).
    pub find_opts: search::Opts,
    /// A replace across the book is the last thing done (or undone), so Ctrl-Z
    /// (or Ctrl-Y) in the editor means all of it, not just this scene.
    replace_undoable: bool,
    replace_redoable: bool,
    /// Spellcheck underlines are showing.
    pub spell_on: bool,
    /// The tree shows a symbol beside each row.
    pub icons_on: bool,
    /// The dictionary, once it has loaded in the background.
    pub speller: Option<spell::Speller>,
    /// The writer's own word list, accepted in every book. None in tests, so
    /// nothing ever reads or writes the real one there.
    pub user_dict: Option<PathBuf>,
    speller_rx: Option<std::sync::mpsc::Receiver<spell::Speller>>,
    /// A tree row being dragged to a new place: (row it started on, row now under the pointer).
    pub tree_drag: Option<(usize, usize)>,
    /// The whole terminal's size at the last draw, for layouts that depend on it.
    pub screen: (u16, u16),
    /// Every notebook note and the names that mean it.
    pub codex_index: Vec<codex::Entry>,
    /// A note open beside the scene.
    pub codex: Option<CodexPane>,
    pub rect_codex: Rect,
    /// A scene open read-only beside the one being written. It takes the same
    /// place as a note, so only one of them shows at a time.
    pub beside: Option<BesidePane>,
    pub rect_beside: Rect,
    /// Focus mode: only the prose, centred, and a quiet status line.
    pub focus_mode: bool,
    /// The widest a line of prose runs (0: the whole pane), and whether focus
    /// mode keeps the line being written mid-screen. From settings.toml.
    pub line_width: usize,
    pub typewriter: bool,
    /// The editor's whole pane, for clicks: `rect_editor` is only the column
    /// the prose is set in.
    pub rect_prose: Rect,
    /// Set during draw: the writing area is wide enough for a pane beside it.
    pub side_room: bool,
    /// Where the cursor was at the last draw, so the view only follows it
    /// when it moves (and a wheel scroll isn't undone).
    last_caret: Option<(usize, usize, usize, usize, usize, bool)>,
    /// The last place written to resume.md, so it's only rewritten on a move.
    last_resume: Option<(PathBuf, usize)>,
    /// Session history is on for this book (checked once at launch).
    pub sessions_on: bool,
    /// A background push of saved sessions, and how the last one went.
    backup_rx: Option<std::sync::mpsc::Receiver<sessions::PushOutcome>>,
    pub backup_note: Option<String>,
    /// Words repeated close together are lit in the open scene.
    pub echo_on: bool,
    /// A writing sprint under way: a word goal while the timer's focus runs.
    pub sprint: Option<Sprint>,
    /// When the disk was last checked for changes made elsewhere.
    last_disk_check: Option<Instant>,
    /// Files were added, removed or parked on disk: re-read the tree as soon
    /// as nothing unsaved is at risk.
    tree_stale: bool,
    /// When re-reading the tree last failed; it isn't tried again until
    /// [`RETRY`] has passed.
    reload_failed: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodexPane {
    pub entry: codex::Entry,
    pub appears: Vec<codex::Appearance>,
    pub sel: usize,
    pub scroll: usize,
}

/// A second scene shown read-only beside the one being written, for
/// continuity. Held by path so it survives the tree being re-read.
#[derive(Debug, Clone, PartialEq)]
pub struct BesidePane {
    pub path: PathBuf,
    pub scroll: usize,
}

/// What's being typed on a corkboard card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardField {
    Synopsis,
    Pov,
}

/// Width of one index card, border included, plus the gap after it.
pub const CARD_W: u16 = 30;

/// Modal state. Only one can be up at a time.
#[derive(Debug, Clone, PartialEq)]
pub enum Overlay {
    None,
    /// The main menu — the discoverable way to reach everything.
    Menu {
        sel: usize,
    },
    /// The menu's Settings, one level down: themes, music, spellcheck, icons.
    Settings {
        sel: usize,
    },
    /// Browsing presets. `restore` is put back if you press Esc.
    Themes {
        sel: usize,
        restore: Theme,
    },
    /// Choosing where music comes from.
    Sources {
        sel: usize,
    },
    /// Editing the custom theme swatch by swatch.
    Custom {
        field: usize,
        buf: String,
    },
    /// Naming a new scene, chapter, part or folder. The name starts as the
    /// plan's suggestion, selected, so typing replaces it and ↵ accepts it.
    Create {
        plan: Plan,
        buf: String,
        /// Still showing the suggestion untouched.
        fresh: bool,
    },
    /// Renaming whatever the tree has selected. The current name starts
    /// selected, so typing replaces it and ↵ keeps it.
    Rename {
        path: PathBuf,
        buf: String,
        fresh: bool,
        /// What it is, in the book's own words: "chapter", "act", "note".
        noun: String,
    },
    /// Deleting is one keypress from gone and there is no undo, so it asks —
    /// by name, with the word count it's about to take with it.
    Confirm {
        path: PathBuf,
        name: String,
        noun: String,
        words: usize,
        /// It's already in the trash, so this one really is the end of it.
        permanent: bool,
    },
    /// Words that couldn't be saved last time, offered back on launch.
    Recover {
        items: Vec<recovery::Pending>,
        /// Which choice is highlighted: see [`RECOVER_CHOICES`]. Only Enter
        /// acts, so a stray key at launch can't decide for you.
        sel: usize,
    },
    /// Ctrl-T: every `%% note %%` and TK in the book, to jump to.
    Marks {
        query: String,
        sel: usize,
        rows: Vec<MarkRow>,
    },
    /// Starting a sprint: how many words, how many minutes.
    Sprint {
        words: String,
        minutes: String,
        on_minutes: bool,
        /// The field still shows its suggestion; the first digit replaces it.
        fresh: bool,
    },
    /// Ctrl-K: find any action, scene, note or theme by name.
    Palette {
        query: String,
        sel: usize,
        entries: Vec<palette::Entry>,
    },
    /// Find (and replace) in the open scene: a bar under the prose, which
    /// stays visible so the matches light up in place.
    Find {
        query: String,
        with: Option<String>,
        on_with: bool,
        from: (usize, usize),
    },
    /// Find (and replace) across the whole book.
    FindBook {
        query: String,
        with: Option<String>,
        on_with: bool,
        hits: Vec<search::Hit>,
        sel: usize,
        /// Showing "replace N in M scenes? y/n".
        confirm: bool,
    },
    /// Near-miss spellings of notebook names.
    Names {
        drifts: Vec<search::Drift>,
        sel: usize,
    },
    /// Session history isn't on yet: offer to turn it on.
    SessionsOff,
    /// Every saved writing session, newest first; Enter shows what changed.
    Sessions {
        list: Vec<sessions::Session>,
        sel: usize,
        /// What the next session would be saved as, if anything has changed.
        pending: Option<String>,
        backup: String,
        /// The scenes one session changed, and which is selected.
        changes: Option<(usize, Vec<sessions::Change>, usize)>,
    },
    /// One scene's changes in one session, as a word diff.
    SessionDiff {
        title: String,
        label: String,
        before: String,
        after: String,
        scroll: usize,
    },
    /// Export for readers: formats, which acts, then the result.
    Export {
        /// Word, EPUB, Markdown.
        formats: [bool; 3],
        /// Each act by path, with its title and whether it's included.
        parts: Vec<(PathBuf, String, bool)>,
        sel: usize,
        /// What happened, once it has run: lines to show.
        done: Option<Vec<String>>,
    },
    /// Suggestions for one misspelt word, plus adding it to the book.
    Spelling {
        line: usize,
        start: usize,
        end: usize,
        word: String,
        suggestions: Vec<String>,
        sel: usize,
        /// Opened by clicking the word, not F8: keys it doesn't use go on to
        /// the editor, so clicking a word to fix it by hand still works.
        inline: bool,
    },
    /// Index cards for one act, chapter by chapter.
    Cork {
        /// The act shown, by path (indices move when the tree reloads);
        /// None for a book without acts.
        scope: Option<PathBuf>,
        sel: usize,
        /// Only this POV's cards are lit.
        pov: Option<String>,
        typing: Option<(CardField, String)>,
    },
    /// A scene's kept versions, with what changed since each.
    History {
        scene: PathBuf,
        title: String,
        versions: Vec<history::Version>,
        sel: usize,
        scroll: usize,
    },
    /// The music player: what's playing, the queue, playlists, and search.
    Player {
        tab: Tab,
        sel: usize,
        /// Keep the selection on the playing track until you move it yourself.
        follow: bool,
        /// Typed into search, and into the playlist finder.
        query: String,
        find: String,
        typing: bool,
    },
}

/// The player's three lists, in Tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Queue,
    Playlists,
    Search,
}

impl Tab {
    pub fn next(self) -> Tab {
        match self {
            Tab::Queue => Tab::Playlists,
            Tab::Playlists => Tab::Search,
            Tab::Search => Tab::Queue,
        }
    }
    pub fn prev(self) -> Tab {
        self.next().next()
    }
}

fn hit(r: Rect, x: u16, y: u16) -> bool {
    r.width > 0 && r.height > 0 && x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
}

impl App {
    /// What to print in front of a shortcut, given what this terminal can send.
    pub fn mod_label(&self) -> &'static str {
        if self.super_keys { "⌘" } else { "^" }
    }

    pub fn new(project: Project) -> Result<Self> {
        Self::with(project, Setup::from_config())
    }

    /// An App from what [`Setup`] hands it: nothing read from ~/.config, and
    /// with `background` off no threads started — how the tests build one.
    pub fn with(mut project: Project, setup: Setup) -> Result<Self> {
        let parents = project.parents();
        // A book no one has opened yet: no day's baseline, nowhere to resume,
        // no words. It opens on the Novel Format guide instead of an empty
        // scene. Checked before load_baseline writes the first baseline.
        let never_opened = !project.root.join(".grimoire/progress.toml").exists()
            && resume::read(&project.root).is_none()
            && project.total_words() == 0;
        let guide = project
            .nodes
            .iter()
            .position(|n| n.area == project::Area::Format && n.kind == Kind::Scene)
            .filter(|_| never_opened);
        let baseline = load_baseline(&project)?;
        // Open the first scene straight away so launching lands you on prose
        // rather than an empty pane. Focus stays on the tree, so a stray
        // keystroke can't wander into the manuscript.
        let first_scene = (0..project.nodes.len())
            .find(|&i| project.nodes[i].kind == Kind::Scene && project.nodes[i].in_manuscript);
        // A three-act book is a hundred rows fully open. Fold the manuscript
        // and then open only the way down to that first scene, so the shape of
        // the book is legible above the prose instead of burying it. A small
        // book is under the threshold and behaves as it always did. From here
        // on, folding is remembered across reloads.
        let scenes = project
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::Scene && n.in_manuscript)
            .count();
        if scenes > 12 {
            for n in &mut project.nodes {
                if n.kind == Kind::Container && n.in_manuscript {
                    n.expanded = false;
                }
            }
            if let Some(i) = first_scene {
                let mut up = parents[i];
                while let Some(pi) = up {
                    project.nodes[pi].expanded = true;
                    up = parents[pi];
                }
            }
        }
        let visible = project.visible();

        Ok(Self {
            project,
            visible,
            parents,
            sel: 0,
            tree_scroll: 0,
            open: None,
            editor: Editor::from_text(""),
            focus: Focus::Tree,
            baseline,
            baseline_date: today_string(),
            msg: String::new(),
            quit: false,
            edit_width: 60,
            edit_height: 20,
            pomo: Pomodoro::default(),
            pane_mode: Mode::Pomodoro,
            music: Music::spawn(setup.music),
            viz: Visualizer::new(),
            player_fetched: None,
            frame: 0,
            super_keys: false,
            scene_visible: true,
            music_visible: true,
            rect_tree: Rect::default(),
            rect_editor: Rect::default(),
            rect_scene: Rect::default(),
            rect_music: Rect::default(),
            create_hits: Vec::new(),
            view_hits: Vec::new(),
            theme: setup.theme,
            overlay: Overlay::None,
            last_edit: None,
            unsaved_since: None,
            save_state: SaveState::Clean,
            last_attempt: None,
            pre_snapshotted: HashSet::new(),
            undo_stash: HashMap::new(),
            tree_undo: Vec::new(),
            tree_redo: Vec::new(),
            find_opts: search::Opts::default(),
            replace_undoable: false,
            replace_redoable: false,
            spell_on: setup.settings.spellcheck,
            icons_on: setup.settings.icons,
            speller: None,
            user_dict: setup.background.then(spell::user_dictionary),
            speller_rx: None,
            tree_drag: None,
            screen: (120, 40),
            codex_index: Vec::new(),
            codex: None,
            rect_codex: Rect::default(),
            beside: None,
            rect_beside: Rect::default(),
            focus_mode: false,
            line_width: setup.settings.line_width,
            typewriter: setup.settings.typewriter,
            rect_prose: Rect::default(),
            side_room: true,
            last_caret: None,
            last_resume: None,
            sessions_on: false,
            backup_rx: None,
            backup_note: None,
            echo_on: false,
            sprint: None,
            last_disk_check: None,
            tree_stale: false,
            reload_failed: None,
        })
        .map(|mut app: App| {
            if setup.background {
                app.load_speller();
            }
            app.rebuild_codex();
            app.sessions_on = setup.background
                && sessions::git_available()
                && sessions::is_enabled(&app.project.root);
            if app.sessions_on {
                app.back_up();
            }
            let items = recovery::pending(&app.project);
            if !items.is_empty() {
                app.overlay = Overlay::Recover { items, sel: 0 };
            }
            if let Some(i) = guide.or(first_scene) {
                app.editor = Editor::from_text(&app.project.nodes[i].body);
                app.open = Some(i);
                if let Some(pos) = app.visible.iter().position(|&v| v == i) {
                    app.sel = pos;
                }
            }
            if guide.is_some() {
                app.msg =
                    "a new book — this is how it's laid out · Tab to read, Esc for the menu".into();
            }
            app.resume_where_left();
            // Say what couldn't be read, once, so an empty row isn't a mystery.
            let unreadable = app
                .project
                .nodes
                .iter()
                .filter(|n| n.disk.unavailable.is_some())
                .count();
            if let Some(why) = &app.project.meta_unreadable {
                app.msg = format!("{why} — opened with the defaults, and it won't be rewritten");
            } else if unreadable > 0 {
                app.msg = format!(
                    "{unreadable} file{} can't be read right now (not downloaded, or offline?) — shown, kept, and never saved over",
                    if unreadable == 1 { "" } else { "s" }
                );
            }
            app
        })
    }

    /// Function keys drive the timer and the music, from either pane, so they
    /// never collide with typing.
    pub fn on_function_key(&mut self, n: u8) {
        if matches!(n, 4..=7) && !self.music.enabled {
            self.msg = "music is off — turn it on from the menu (Esc › Settings)".into();
            return;
        }
        match n {
            2 => self.pomo.toggle(),
            3 => {
                self.pomo.reset();
                self.msg = "timer reset".into();
                self.end_sprint("timer reset, sprint stopped");
            }
            4 => self.music.send(music::Cmd::Prev),
            5 => self.music.send(music::Cmd::PlayPause),
            6 => self.music.send(music::Cmd::Next),
            7 => self.open_player(),
            8 => self.spelling(),
            _ => {}
        }
    }

    pub fn refresh_visible(&mut self) {
        self.visible = self.project.visible();
        if self.sel >= self.visible.len() {
            self.sel = self.visible.len().saturating_sub(1);
        }
    }

    /// Breadcrumb for the editor pane title.
    pub fn open_title(&self) -> String {
        let Some(i) = self.open else {
            return "no scene".into();
        };
        let scene = &self.project.nodes[i].title;
        match self.parents[i] {
            Some(p) => format!("{} / {}", self.project.nodes[p].title, scene),
            None => scene.clone(),
        }
    }

    /// Push editor contents back into the open node.
    pub fn flush(&mut self) {
        if let Some(i) = self.open {
            let text = self.editor.text();
            if text != self.project.nodes[i].body {
                self.project.nodes[i].body = text;
                self.mark_changed(i);
                // Typed since: Ctrl-Z is about this scene again.
                self.replace_undoable = false;
                self.replace_redoable = false;
            }
        }
    }

    /// A scene's text changed in memory; autosave will pick it up.
    fn mark_changed(&mut self, i: usize) {
        if self.project.nodes[i].read_only {
            self.refuse_read_only(i);
            return;
        }
        self.project.nodes[i].dirty = true;
        let now = Instant::now();
        self.last_edit = Some(now);
        self.unsaved_since.get_or_insert(now);
    }

    /// A file that isn't UTF-8 is shown, never changed: whatever just
    /// happened to it in memory is put back from disk, and the writer is told
    /// why. Writing it would replace the bytes that couldn't be read.
    fn refuse_read_only(&mut self, i: usize) {
        if let Some(why) = self.project.nodes[i].disk.unavailable.clone()
            && self.project.nodes[i].never_read()
        {
            if self.open == Some(i) {
                self.editor = Editor::from_text("");
            }
            self.msg = format!(
                "{} can't be edited yet — {}",
                self.project.nodes[i].title, why.why
            );
            return;
        }
        let _ = self.project.nodes[i].read_disk();
        if self.open == Some(i) {
            let (cy, cx) = (self.editor.cy, self.editor.cx);
            self.editor = Editor::from_text(&self.project.nodes[i].body);
            self.editor.place(cy, cx);
        }
        self.msg = format!(
            "{} isn't UTF-8 text, so Grimoire shows it but won't change it — resave it as UTF-8 to edit it here",
            self.project.nodes[i].title
        );
    }

    fn open_scene(&mut self, idx: usize) {
        self.flush();
        // Always the file as it is now: something else may have written it
        // since the book was read.
        match self.project.check_disk(idx, true) {
            Ok(change @ (DiskChange::Gone | DiskChange::GoneParked(_))) => {
                self.take_disk_change(idx, change);
                return;
            }
            Ok(change) => self.take_disk_change(idx, change),
            Err(_) => {}
        }
        self.save_resume(false);
        // Park this scene's undo so coming back to it still undoes.
        if let Some(i) = self.open {
            let path = self.project.nodes[i].path.clone();
            self.undo_stash.insert(path, self.editor.take_history());
        }
        self.editor = Editor::from_text(&self.project.nodes[idx].body);
        if let Some(h) = self.undo_stash.remove(&self.project.nodes[idx].path) {
            self.editor.set_history(h);
        }
        self.open = Some(idx);
        self.focus = Focus::Editor;
        self.msg.clear();
    }

    /// Ctrl-S. Autosave does this on its own; pressing it is never wrong.
    pub fn save(&mut self) {
        let before = self.project.dirty_count();
        self.flush();
        let pending = self.project.dirty_count().max(before);
        if self.commit_saves() {
            self.msg = match pending {
                0 => "all saved".into(),
                1 => "saved".into(),
                n => format!("saved {n} scenes"),
            };
        }
    }

    /// Write every changed scene, keeping history as it goes. On a failure
    /// the words go to recovery and the status bar says so. True when
    /// everything that needed saving was saved.
    pub fn commit_saves(&mut self) -> bool {
        self.flush();
        self.last_attempt = Some(Instant::now());
        let root = self.project.root.clone();
        let dirty: Vec<usize> = (0..self.project.nodes.len())
            .filter(|&i| self.project.nodes[i].dirty && self.project.nodes[i].kind == Kind::Scene)
            .collect();
        if dirty.is_empty() {
            self.unsaved_since = None;
            return true;
        }
        // The version from before this session first touched a scene always
        // goes into history, so "how it was this morning" is never lost.
        for &i in &dirty {
            let path = self.project.nodes[i].path.clone();
            if self.pre_snapshotted.insert(path.clone())
                && let Ok(old) = fs::read_to_string(&path)
            {
                let _ = history::snapshot(&root, &path, &old, None);
            }
        }
        let report = self.project.save_dirty();
        for (i, copy) in &report.parked {
            self.take_disk_change(*i, DiskChange::Parked(copy.clone()));
        }
        for (i, copy) in &report.gone {
            self.take_disk_change(*i, DiskChange::GoneParked(copy.clone()));
        }
        // Waiting on a file that can't be read or checked yet: the words stay
        // unsaved in memory, and a copy goes to recovery in case Grimoire
        // closes before the file comes back.
        for &i in &report.waiting {
            let n = &self.project.nodes[i];
            let _ = recovery::keep(&root, &n.path, &n.file_text());
        }
        let notes_changed = report
            .saved
            .iter()
            .any(|&i| !self.project.nodes[i].in_manuscript);
        for &i in &report.saved {
            let n = &self.project.nodes[i];
            let _ = history::snapshot(&root, &n.path, &n.file_text(), Some(history::GAP));
            recovery::clear(&root, &n.path);
        }
        if notes_changed || (self.codex.is_some() && !report.saved.is_empty()) {
            self.rebuild_codex();
        }
        if report.failed.is_empty() {
            for (i, _) in report.parked.iter().chain(&report.gone) {
                recovery::clear(&root, &self.project.nodes[*i].path);
            }
            if let Some(&i) = report.waiting.first() {
                let n = &self.project.nodes[i];
                let why = n
                    .disk
                    .unavailable
                    .as_ref()
                    .map(|u| u.why.clone())
                    .unwrap_or_else(|| "its file is missing just now".into());
                let wait = format!("{} can't be saved yet — {why}", n.title);
                if !matches!(&self.save_state, SaveState::Waiting(w) if *w == wait) {
                    self.msg =
                        format!("{wait}. Your words are kept and will be saved once it can be.");
                }
                self.save_state = SaveState::Waiting(wait);
                self.save_resume(false);
                return true;
            }
            self.save_state = SaveState::Saved(Instant::now());
            self.unsaved_since = None;
            self.save_resume(false);
            return true;
        }
        let mut kept = true;
        for (i, _) in &report.failed {
            let n = &self.project.nodes[*i];
            kept &= recovery::keep(&root, &n.path, &n.file_text()).is_ok();
        }
        let (i, why) = &report.failed[0];
        let title = self.project.nodes[*i].title.clone();
        self.save_state = SaveState::Failed(why.clone());
        self.msg = if kept {
            format!(
                "couldn't save {title} ({why}) — your words are kept safe and will be offered back"
            )
        } else {
            format!(
                "couldn't save {title} ({why}) — and couldn't keep a copy either; copy your text somewhere"
            )
        };
        false
    }

    /// Called every tick: save once typing has paused, or once changes have
    /// waited too long.
    /// "Today" starts again at midnight, even with Grimoire left open.
    pub fn tick_today(&mut self) {
        let date = today_string();
        if date != self.baseline_date {
            self.baseline_date = date;
            let total = self.project.total_words();
            self.set_baseline(total);
        }
    }

    /// The day's net change in words: below zero after cutting more than has
    /// been written, so every word typed still moves it. Cut and paste
    /// elsewhere in the book comes out even.
    pub fn today_words(&self) -> i64 {
        self.project.total_words() as i64 - self.baseline as i64
    }

    /// Deleting a scene, or bringing one back, isn't writing: it moves the
    /// day's starting point by the same amount, so "today" doesn't change.
    fn keep_today(&mut self, before: usize) {
        let after = self.project.total_words();
        if after != before {
            self.set_baseline((self.baseline + after).saturating_sub(before));
            if let Some(s) = &mut self.sprint {
                s.start = (s.start + after).saturating_sub(before);
            }
        }
    }

    fn set_baseline(&mut self, n: usize) {
        if n == self.baseline {
            return;
        }
        self.baseline = n;
        let dir = self.project.root.join(".grimoire");
        write_baseline(&dir, &dir.join("progress.toml"), &self.baseline_date, n);
    }

    /// After a failed or waiting save, the next try waits out [`RETRY`] —
    /// however it's asked for.
    fn may_retry(&self) -> bool {
        match self.save_state {
            SaveState::Failed(_) | SaveState::Waiting(_) => {
                self.last_attempt.is_none_or(|t| t.elapsed() >= RETRY)
            }
            _ => true,
        }
    }

    pub fn autosave_tick(&mut self) {
        if self.project.dirty_count() == 0 {
            self.unsaved_since = None;
            return;
        }
        let idle = self.last_edit.is_none_or(|t| t.elapsed() >= AUTOSAVE_IDLE);
        let overdue = self
            .unsaved_since
            .is_some_and(|t| t.elapsed() >= AUTOSAVE_MAX);
        if (idle || overdue) && self.may_retry() {
            self.commit_saves();
        }
    }

    /// Called every tick. Every couple of seconds, look for scenes changed,
    /// added or removed on disk by something else and take them in; see
    /// [`Project::check_disk`]. Nothing written elsewhere is overwritten, and
    /// nothing unsaved here is dropped: it's parked beside the scene instead.
    pub fn sync_tick(&mut self) {
        let due = self
            .last_disk_check
            .is_none_or(|t| t.elapsed() >= DISK_CHECK);
        if due {
            self.last_disk_check = Some(Instant::now());
            self.flush();
            let mut notes_changed = false;
            for i in 0..self.project.nodes.len() {
                let before = self.project.total_words();
                // An error is a file caught mid-write by a sync client: next time.
                if let Ok(change) = self.project.check_disk(i, false) {
                    notes_changed |=
                        change != DiskChange::Same && !self.project.nodes[i].in_manuscript;
                    // A scene downloading isn't writing: "today" doesn't move.
                    if change == DiskChange::Available {
                        self.keep_today(before);
                    }
                    self.take_disk_change(i, change);
                }
            }
            if notes_changed {
                // A note's names feed the codex and the spellchecker.
                self.refresh_names();
            }
            if !self.tree_stale && self.project.tree_is_stale() {
                self.tree_stale = true;
            }
        }
        if self.tree_stale {
            self.reload_after_sync();
        }
    }

    /// Say what a change on disk meant, and show it if it's the open scene.
    fn take_disk_change(&mut self, i: usize, change: DiskChange) {
        let title = self.project.nodes[i].title.clone();
        let open = self.open == Some(i);
        let name = |p: &Path| {
            p.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        };
        match change {
            DiskChange::Same => {}
            DiskChange::Adopted => {
                if open {
                    let body = self.project.nodes[i].body.clone();
                    self.editor.set_text(&body);
                    let m = self.mod_label();
                    self.msg = format!(
                        "{title} was changed outside Grimoire — showing that version ({m}Z goes back)"
                    );
                }
            }
            DiskChange::Parked(copy) => {
                if open {
                    let body = self.project.nodes[i].body.clone();
                    self.editor.set_text(&body);
                }
                self.msg = format!(
                    "{title} was changed elsewhere while you were writing — that version is the scene now, and yours is beside it as “{}”",
                    name(&copy)
                );
                self.tree_stale = true;
            }
            DiskChange::Gone => {
                if open {
                    self.msg = format!("{title} was moved or deleted outside Grimoire");
                }
                self.tree_stale = true;
            }
            DiskChange::GoneParked(copy) => {
                self.msg = format!(
                    "{title} was moved or deleted outside Grimoire — your unsaved words are in the Trash as “{}”",
                    name(&copy)
                );
                self.tree_stale = true;
            }
            DiskChange::Unavailable(why) => {
                if open || self.project.nodes[i].dirty {
                    self.msg = format!(
                        "{title}: {} — what's here is kept, and nothing is saved over it until it can be read",
                        why.why
                    );
                }
            }
            DiskChange::Available => {
                if open {
                    let body = self.project.nodes[i].body.clone();
                    self.editor.set_text(&body);
                    self.msg = format!("{title} can be read again");
                }
            }
        }
    }

    /// Take in files added or removed elsewhere. Unsaved words are saved (or
    /// parked) first, so re-reading the tree can't drop them; if something
    /// can't be saved yet, this waits for a later tick.
    fn reload_after_sync(&mut self) {
        // A save that failed waits out its backoff here too, and a re-read
        // that failed isn't tried again every frame.
        if !self.may_retry() || self.reload_failed.is_some_and(|t| t.elapsed() < RETRY) {
            return;
        }
        if !self.commit_saves() {
            return;
        }
        self.tree_stale = false;
        let open_was = self.open.map(|i| {
            (
                self.project.nodes[i].path.clone(),
                self.project.nodes[i].seen,
            )
        });
        if let Err(e) = self.reload_tree() {
            self.msg = format!("couldn't re-read the book: {e}");
            self.reload_failed = Some(Instant::now());
            self.tree_stale = true;
            return;
        }
        self.reload_failed = None;
        let Some((path, seen)) = open_was else {
            return;
        };
        if self.open.is_some() {
            return;
        }
        // The open scene went. If the same words turn up somewhere else, it
        // was moved or renamed there: follow it.
        let moved = seen.and_then(|fp| {
            self.project
                .nodes
                .iter()
                .position(|n| n.kind == Kind::Scene && !n.parked && n.seen == Some(fp))
        });
        match moved {
            Some(j) => {
                self.open = Some(j);
                self.reveal(j);
                self.msg = format!(
                    "{} was moved outside Grimoire — followed it",
                    self.project.nodes[j].title
                );
            }
            None => {
                self.undo_stash.remove(&path);
                self.editor = Editor::from_text("");
                if self.focus == Focus::Editor {
                    self.focus = Focus::Tree;
                }
            }
        }
    }

    /// Save everything before quitting — and where you were, and the session
    /// if session history is on. False (with a message) if something couldn't
    /// be saved; its words are in recovery either way.
    pub fn try_quit(&mut self) -> bool {
        if !self.commit_saves() {
            return false;
        }
        self.save_resume(true);
        if self.sessions_on && sessions::has_writing(&self.project.root) {
            // Local and quick; backing up happens next launch.
            let _ = sessions::commit_session(&self.project.root, chrono::Local::now());
        }
        true
    }

    /// Last resort after a crash: keep every unsaved scene in recovery rather
    /// than trusting a half-broken state to write the real files.
    pub fn rescue(&mut self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.flush()));
        let root = self.project.root.clone();
        for n in self
            .project
            .nodes
            .iter()
            .filter(|n| n.dirty && n.kind == Kind::Scene)
        {
            let _ = recovery::keep(&root, &n.path, &n.file_text());
        }
    }

    // ---- undo, cut and paste -------------------------------------------

    pub fn undo(&mut self) {
        if self.focus != Focus::Editor || self.open.is_none() || self.replace_undoable {
            self.tree_step(true);
            return;
        }
        if self.editor.undo() {
            self.flush();
        } else {
            self.msg = "nothing to undo".into();
        }
    }

    pub fn redo(&mut self) {
        if self.focus != Focus::Editor || self.open.is_none() || self.replace_redoable {
            self.tree_step(false);
            return;
        }
        if self.editor.redo() {
            self.flush();
        } else {
            self.msg = "nothing to redo".into();
        }
    }

    pub fn cut(&mut self) {
        if self.focus != Focus::Editor || self.open.is_none() {
            return;
        }
        match self.editor.selected_text() {
            Some(text) if !text.is_empty() => {
                if copy_to_clipboard(&text) {
                    self.editor.delete_selection();
                    self.flush();
                    self.msg = "cut — undo puts it back".into();
                } else {
                    self.msg = "no clipboard tool found, so nothing was cut".into();
                }
            }
            _ => self.msg = "nothing selected".into(),
        }
    }

    /// Text pasted into the terminal arrives in one piece.
    pub fn paste(&mut self, text: &str) {
        if self.focus != Focus::Editor || self.open.is_none() {
            return;
        }
        self.editor.delete_selection();
        self.editor.insert_str(text);
        self.flush();
    }

    // ---- the command palette -------------------------------------------

    pub fn open_palette(&mut self) {
        self.flush();
        let entries = palette::entries(self);
        self.overlay = Overlay::Palette {
            query: String::new(),
            sel: 0,
            entries,
        };
    }

    /// Point the tree at a node: unfold the way down to it and select it.
    pub fn reveal(&mut self, idx: usize) {
        let mut up = self.parents[idx];
        while let Some(pi) = up {
            self.project.nodes[pi].expanded = true;
            up = self.parents[pi];
        }
        self.refresh_visible();
        if let Some(pos) = self.visible.iter().position(|&v| v == idx) {
            self.sel = pos;
        }
    }

    /// Tree actions act on the tree's selection. From the editor, that should
    /// be the scene you're writing, not wherever the tree was left.
    fn select_open_scene(&mut self) {
        if self.focus == Focus::Editor
            && let Some(i) = self.open
        {
            self.reveal(i);
        }
    }

    pub fn run_action(&mut self, action: Action) {
        self.overlay = Overlay::None;
        match action {
            Action::NewScene | Action::NewChapter | Action::NewPart | Action::NewFolder => {
                self.select_open_scene();
                self.start_create(match action {
                    Action::NewScene => New::Scene,
                    Action::NewChapter => New::Chapter,
                    Action::NewPart => New::Part,
                    _ => New::Folder,
                });
            }
            Action::Rename => {
                self.select_open_scene();
                self.start_rename();
            }
            Action::Delete => {
                self.select_open_scene();
                self.start_delete();
            }
            Action::History => self.open_history(),
            Action::Save => self.save(),
            Action::Undo | Action::Redo => {
                // The tree's own actions are undone from anywhere but the
                // editor; from the editor, it's the typing.
                if action == Action::Undo {
                    self.undo()
                } else {
                    self.redo()
                }
            }
            Action::ProjectMap => self.write_project_map(),
            Action::Compile => self.compile_markdown(),
            Action::Themes => self.open_theme_picker(),
            Action::Theme(name) => {
                if let Some(th) = theme::presets().into_iter().find(|t| t.name == name) {
                    self.theme = th;
                    let _ = theme::save(&self.theme);
                    self.msg = format!("theme: {name}");
                }
            }
            Action::MusicToggle => self.set_music(!self.music.enabled),
            Action::MusicPlayer => self.on_function_key(7),
            Action::MusicSource => {
                let cur = self.music.source;
                let sel = music::Source::ALL
                    .iter()
                    .position(|s| *s == cur)
                    .unwrap_or(0);
                self.overlay = Overlay::Sources { sel };
            }
            Action::PlayPause => self.on_function_key(5),
            Action::NextTrack => self.on_function_key(6),
            Action::PrevTrack => self.on_function_key(4),
            Action::Timer => self.on_function_key(2),
            Action::TimerReset => self.on_function_key(3),
            Action::Menu => self.open_menu(),
            Action::Quit => self.quit = true,
            Action::FindAnything => self.open_palette(),
            Action::Settings => self.overlay = Overlay::Settings { sel: 0 },
            Action::MenuBack => {
                let sel = self
                    .menu()
                    .iter()
                    .position(|(_, a)| *a == Action::Settings)
                    .unwrap_or(0);
                self.overlay = Overlay::Menu { sel };
            }
            // run_action has already put the menu away.
            Action::CloseMenu => {}
            Action::Open(path) => {
                if let Some(i) = self.project.nodes.iter().position(|n| n.path == path) {
                    self.reveal(i);
                    self.open_scene(i);
                }
            }
            Action::FocusMode => self.toggle_focus_mode(),
            Action::BesidePicker => self.open_beside_picker(),
            Action::Beside(path) => {
                if let Some(i) = self.project.nodes.iter().position(|n| n.path == path) {
                    self.show_beside(i);
                }
            }
            Action::LineWidth => self.cycle_line_width(),
            Action::Typewriter => self.toggle_typewriter(),
            other => self.run_feature(other),
        }
    }

    /// Actions that belong to a feature with its own section below.
    fn run_feature(&mut self, action: Action) {
        match action {
            Action::Corkboard => self.open_cork(),
            Action::NotesList => self.open_marks(),
            Action::NextTk => self.next_tk(),
            Action::NextDraft => self.next_draft(),
            Action::EchoWords => self.toggle_echoes(),
            Action::StartSprint => self.start_sprint_dialog(),
            Action::EndSprint => {
                self.pomo.reset();
                self.end_sprint("sprint stopped");
            }
            Action::OpenCodex => self.open_codex(),
            Action::Export => self.open_export(),
            Action::Sessions => self.open_sessions(),
            Action::SaveSession => {
                if !self.sessions_on {
                    self.open_sessions();
                } else if let Some(label) = self.save_session() {
                    self.msg = format!("session saved: {label}");
                }
            }
            Action::Spellcheck => self.toggle_spellcheck(),
            Action::Icons => self.toggle_icons(),
            Action::SpellingSuggestions => self.spelling(),
            Action::MoveUp => self.move_selected(true),
            Action::MoveDown => self.move_selected(false),
            Action::FindInScene => self.open_find(),
            Action::FindInBook => self.open_find_book(String::new()),
            Action::CheckNames => self.check_names(),
            other => self.msg = format!("{other:?} isn't available yet"),
        }
    }

    // ---- find and replace ------------------------------------------------

    /// Ctrl-F in a scene. A selected phrase becomes the search.
    pub fn open_find(&mut self) {
        let Some(_) = self.open else {
            self.open_find_book(String::new());
            return;
        };
        self.flush();
        self.focus = Focus::Editor;
        let query = self
            .editor
            .selected_text()
            .filter(|s| !s.contains('\n'))
            .unwrap_or_default();
        let from = self
            .editor
            .selection()
            .map(|(a, _)| a)
            .unwrap_or((self.editor.cy, self.editor.cx));
        self.overlay = Overlay::Find {
            query,
            with: None,
            on_with: false,
            from,
        };
    }

    /// Every match in the open scene, in order.
    fn scene_matches(&self, query: &str) -> Vec<(usize, usize, usize)> {
        self.editor
            .lines
            .iter()
            .enumerate()
            .flat_map(|(l, line)| {
                search::matches_with(line, query, self.find_opts)
                    .into_iter()
                    .map(move |(s, e)| (l, s, e))
            })
            .collect()
    }

    /// Select the next match after the cursor (or the previous one before the
    /// selection), wrapping at the ends. Returns "3 of 9" style position.
    fn find_step(&mut self, query: &str, forward: bool, from: Option<(usize, usize)>) {
        let all = self.scene_matches(query);
        if all.is_empty() {
            self.editor.clear_selection();
            return;
        }
        let here = from.unwrap_or_else(|| {
            if forward {
                (self.editor.cy, self.editor.cx)
            } else {
                self.editor
                    .selection()
                    .map(|(a, _)| a)
                    .unwrap_or((self.editor.cy, self.editor.cx))
            }
        });
        let pick = if forward {
            all.iter()
                .find(|&&(l, s, _)| (l, s) >= here)
                .or(all.first())
        } else {
            all.iter()
                .rev()
                .find(|&&(l, s, _)| (l, s) < here)
                .or(all.last())
        };
        if let Some(&(l, s, e)) = pick {
            self.editor.select((l, s), (l, e));
        }
    }

    /// "3 of 9" for the find bar, or "no matches".
    pub fn find_position(&self, query: &str) -> String {
        if query.is_empty() {
            return String::new();
        }
        let all = self.scene_matches(query);
        if all.is_empty() {
            return "no matches".into();
        }
        let cur = self
            .editor
            .selection()
            .and_then(|(a, b)| all.iter().position(|&(l, s, e)| (l, s) == a && (l, e) == b));
        match cur {
            Some(i) => format!("{} of {}", i + 1, all.len()),
            None => format!("{} matches", all.len()),
        }
    }

    fn replace_current(&mut self, query: &str, with: &str) {
        let is_match = self.editor.selected_text().is_some_and(|t| {
            search::matches_with(&t, query, self.find_opts) == vec![(0, t.chars().count())]
        });
        if is_match {
            self.editor.delete_selection();
            self.editor.insert_str(with);
            self.flush();
        }
        self.find_step(query, true, None);
    }

    /// Ctrl-R while finding: replace every match in this scene, or ask before
    /// replacing across the book.
    pub fn replace_all_key(&mut self) {
        match &mut self.overlay {
            Overlay::Find {
                query,
                with: Some(with),
                ..
            } if !query.is_empty() => {
                let (query, with) = (query.clone(), with.clone());
                let (text, n) =
                    search::replace_all(&self.editor.text(), &query, &with, self.find_opts);
                if n > 0 {
                    self.editor.set_text(&text);
                    self.flush();
                }
                self.msg = match n {
                    0 => "no matches".into(),
                    1 => "replaced 1 — Ctrl-Z undoes it".into(),
                    n => format!("replaced {n} — Ctrl-Z undoes them"),
                };
            }
            Overlay::FindBook {
                query,
                with: Some(_),
                hits,
                confirm,
                ..
            } if !query.is_empty() && !hits.is_empty() => {
                *confirm = true;
            }
            Overlay::Find { .. } | Overlay::FindBook { .. } => {
                self.msg = "Tab to type a replacement first".into();
            }
            _ => {}
        }
    }

    /// ^W / Alt-W in the find bar: whole words only, or inside words too.
    /// ^E / Alt-C: exact case, or any case. What's found is what's replaced.
    pub fn toggle_find_opt(&mut self, whole_words: bool) {
        if whole_words {
            self.find_opts.whole_words = !self.find_opts.whole_words;
        } else {
            self.find_opts.match_case = !self.find_opts.match_case;
        }
        match &mut self.overlay {
            Overlay::Find { query, from, .. } => {
                let (q, from) = (query.clone(), *from);
                self.find_step(&q, true, Some(from));
            }
            Overlay::FindBook {
                query, hits, sel, ..
            } => {
                *hits = search::book(&self.project, &self.parents, query, self.find_opts);
                *sel = 0;
            }
            _ => {}
        }
    }

    /// The find bar's matching, as it reads on screen.
    pub fn find_opts_label(&self) -> String {
        format!(
            "{} · {}",
            if self.find_opts.whole_words {
                "whole words"
            } else {
                "inside words too"
            },
            if self.find_opts.match_case {
                "exact case"
            } else {
                "any case"
            },
        )
    }

    pub fn open_find_book(&mut self, query: String) {
        self.flush();
        let hits = search::book(&self.project, &self.parents, &query, self.find_opts);
        self.overlay = Overlay::FindBook {
            query,
            with: None,
            on_with: false,
            hits,
            sel: 0,
            confirm: false,
        };
    }

    /// Open the scene a hit is in, with the match selected.
    fn go_to_hit(&mut self, path: &Path, line: usize, start: usize, end: usize) {
        self.overlay = Overlay::None;
        let Some(i) = self.project.nodes.iter().position(|n| n.path == path) else {
            return;
        };
        self.reveal(i);
        if self.open != Some(i) {
            self.open_scene(i);
        }
        self.focus = Focus::Editor;
        self.editor.select((line, start), (line, end));
    }

    fn replace_in_book(&mut self, query: &str, with: &str) {
        self.flush();
        let root = self.project.root.clone();
        let mut changed = Vec::new();
        let mut total = 0;
        for i in 0..self.project.nodes.len() {
            if self.project.nodes[i].kind != Kind::Scene
                || self.project.in_trash(i)
                || self.project.nodes[i].read_only
            {
                continue;
            }
            let before = self.project.nodes[i].body.clone();
            let (text, n) = search::replace_all(&before, query, with, self.find_opts);
            if n == 0 {
                continue;
            }
            // Every scene's version from before goes into its history.
            let path = self.project.nodes[i].path.clone();
            let _ = history::snapshot(&root, &path, &self.project.nodes[i].file_text(), None);
            if self.open == Some(i) {
                self.editor.set_text(&text);
            }
            self.project.nodes[i].body = text.clone();
            self.mark_changed(i);
            changed.push((path, before, text));
            total += n;
        }
        let scenes = changed.len();
        self.commit_saves();
        if scenes > 0 {
            self.record(TreeStep::Replaced {
                what: format!("replace “{query}” with “{with}”"),
                scenes: changed,
            });
        }
        let m = self.mod_label();
        self.msg = format!(
            "replaced {total} in {scenes} scene{} — {m}Z takes back all of it",
            if scenes == 1 { "" } else { "s" }
        );
    }

    /// Put each scene a book-wide replace changed back to `to` (from `from`).
    /// A scene edited since is left alone. Returns how many were left.
    fn swap_replaced(&mut self, scenes: &[(PathBuf, String, String)], back: bool) -> usize {
        let mut skipped = 0;
        for (path, before, after) in scenes {
            let (from, to) = if back {
                (after, before)
            } else {
                (before, after)
            };
            let Some(i) = self.project.nodes.iter().position(|n| &n.path == path) else {
                skipped += 1;
                continue;
            };
            if &self.project.nodes[i].body != from {
                skipped += 1;
                continue;
            }
            if self.open == Some(i) {
                // The editor holds the replace as a step of its own: step
                // through it, so its undo and redo stay in order.
                let stepped = if back {
                    self.editor.undo()
                } else {
                    self.editor.redo()
                };
                if !stepped || &self.editor.text() != to {
                    self.editor.set_text(to);
                }
            }
            self.project.nodes[i].body = to.clone();
            self.mark_changed(i);
        }
        skipped
    }

    pub fn check_names(&mut self) {
        self.flush();
        let names = search::names(&self.project, &self.parents);
        if names.is_empty() {
            self.msg = "no notes in the notebook to check names against yet".into();
            return;
        }
        let known = |w: &str| self.is_dictionary_word(w);
        let drifts = search::drift(&self.project, &self.parents, &names, &known);
        if drifts.is_empty() {
            self.msg = format!("every name matches the notebook ({} checked)", names.len());
            return;
        }
        self.overlay = Overlay::Names { drifts, sel: 0 };
    }

    /// Whether a word is ordinary English, so it isn't mistaken for a
    /// misspelt name. Without a dictionary loaded, nothing is. Lowercased, so
    /// "Reach" counts as the word "reach"; a name the writer has added to the
    /// book's own list counts too, since it's meant.
    fn is_dictionary_word(&self, word: &str) -> bool {
        self.speller
            .as_ref()
            .is_some_and(|s| s.is_correct(&word.to_lowercase()))
    }

    // ---- export ----------------------------------------------------------

    pub fn open_export(&mut self) {
        self.flush();
        let parts = export::parts(&self.project)
            .into_iter()
            .map(|(i, title)| (self.project.nodes[i].path.clone(), title, true))
            .collect();
        self.overlay = Overlay::Export {
            formats: [true, true, false],
            parts,
            sel: 0,
            done: None,
        };
    }

    fn run_export(&mut self, formats: [bool; 3], parts: &[(PathBuf, String, bool)]) -> Vec<String> {
        // Export what's on screen, saved or not; saving first keeps the files
        // and the export in agreement.
        self.commit_saves();
        let chosen: Vec<usize> = parts
            .iter()
            .filter(|(_, _, on)| *on)
            .filter_map(|(p, _, _)| self.project.nodes.iter().position(|n| &n.path == p))
            .collect();
        let whole = parts.is_empty() || chosen.len() == parts.len();
        let opts = export::ExportOptions {
            docx: formats[0],
            epub: formats[1],
            markdown: formats[2],
            parts: if whole { None } else { Some(chosen) },
        };
        match export::export(&self.project, &opts) {
            Ok(done) => {
                let mut lines: Vec<String> = done
                    .files
                    .iter()
                    .map(|f| {
                        let size = fs::metadata(f).map(|m| m.len()).unwrap_or(0);
                        let rel = f.strip_prefix(&self.project.root).unwrap_or(f);
                        format!("✓ {}  {}", rel.display(), human_size(size))
                    })
                    .collect();
                lines.push(String::new());
                // Shunn rounds, which makes a short book "about 0 words".
                let rounded = manuscript::rounded_words(done.words);
                let words = if rounded == 0 {
                    format!("{} words", done.words)
                } else {
                    format!("about {rounded} words")
                };
                lines.push(format!(
                    "{} chapter{} · {words} · {} manuscript page{}",
                    done.chapters,
                    if done.chapters == 1 { "" } else { "s" },
                    done.pages,
                    if done.pages == 1 { "" } else { "s" },
                ));
                self.msg = format!(
                    "exported {} file{} to exports/",
                    done.files.len(),
                    if done.files.len() == 1 { "" } else { "s" }
                );
                lines
            }
            Err(e) => vec![format!("couldn't export: {e}")],
        }
    }

    // ---- spelling --------------------------------------------------------

    /// Build the dictionary off the UI thread, with the notebook's names and
    /// the book's own word list already accepted.
    fn load_speller(&mut self) {
        let words = self.speller_words();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut s = spell::Speller::new();
            s.add_words(words);
            let _ = tx.send(s);
        });
        self.speller_rx = Some(rx);
    }

    /// Everything the dictionary should accept beyond English: the
    /// notebook's names, this book's list and the writer's own.
    fn speller_words(&self) -> Vec<String> {
        let mut words = spell::names_from_titles(&self.note_titles());
        words.extend(spell::book_words(&self.project.root));
        if let Some(p) = &self.user_dict {
            words.extend(spell::words_in(p));
        }
        words
    }

    /// The dictionary, built right now on this thread — for tests, which
    /// start no background work.
    #[cfg(test)]
    pub(crate) fn load_speller_now(&mut self) {
        let mut s = spell::Speller::new();
        s.add_words(self.speller_words());
        self.speller = Some(s);
    }

    fn note_titles(&self) -> Vec<String> {
        self.project
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, n)| {
                n.kind == Kind::Scene && n.area.is_notebook() && !self.project.in_trash(*i)
            })
            .map(|(_, n)| n.title.clone())
            .collect()
    }

    /// Called every tick: pick up the dictionary when it's ready.
    pub fn tick_speller(&mut self) {
        if let Some(rx) = &self.speller_rx
            && let Ok(s) = rx.try_recv()
        {
            self.speller = Some(s);
            self.speller_rx = None;
            // Now ordinary words can be told apart from names.
            self.rebuild_codex();
        }
    }

    /// New notes may have added names.
    fn refresh_names(&mut self) {
        let words = spell::names_from_titles(&self.note_titles());
        if let Some(s) = &mut self.speller {
            s.add_words(words);
        }
        self.rebuild_codex();
    }

    // ---- picking up where you left off ----------------------------------

    /// Land on the sentence recorded in resume.md, if its scene still exists.
    fn resume_where_left(&mut self) {
        let root = self.project.root.clone();
        let Some(r) = resume::read(&root) else { return };
        let scene = root.join(&r.scene);
        let Some(i) = self
            .project
            .nodes
            .iter()
            .position(|n| n.kind == Kind::Scene && n.path == scene)
        else {
            return;
        };
        self.reveal(i);
        self.editor = Editor::from_text(&self.project.nodes[i].body);
        self.open = Some(i);
        self.editor.place(r.line, r.column);
        self.focus = Focus::Editor;
        self.last_resume = Some((scene, r.line));
        let place = self.parents[i]
            .map(|p| search::place_of(&self.project, &self.parents, p))
            .unwrap_or_default();
        let here = resume::machine_name();
        let who = if r.machine == here {
            "here".to_string()
        } else {
            format!("on {}", r.machine)
        };
        let when = r.when.format("%a %-I:%M %P");
        // A scene with nothing in it was only ever open, never written in.
        // The scene and when come first, so a narrow status line keeps them.
        let body = &self.project.nodes[i].body;
        let (did, at) = if body.split_whitespace().next().is_some() {
            ("last written", format!(" · paragraph {}", r.line + 1))
        } else {
            ("left open", String::new())
        };
        self.msg = format!(
            "resuming {} · {did} {who}, {when}{at}{}{place}",
            self.project.nodes[i].title,
            if place.is_empty() { "" } else { " · " },
        );
    }

    /// Record where the cursor is, if it has moved to another paragraph or
    /// scene since the last time (or always, when leaving).
    fn save_resume(&mut self, force: bool) {
        let Some(i) = self.open else { return };
        let n = &self.project.nodes[i];
        if !n.in_manuscript && !n.front_matter {
            return;
        }
        let here = (n.path.clone(), self.editor.cy);
        if !force && self.last_resume.as_ref() == Some(&here) {
            return;
        }
        let root = self.project.root.clone();
        let r = resume::Resume {
            scene: resume::relative(&root, &n.path),
            line: self.editor.cy,
            column: self.editor.cx,
            machine: resume::machine_name(),
            when: chrono::Local::now(),
        };
        let place = self.parents[i]
            .map(|p| search::place_of(&self.project, &self.parents, p))
            .unwrap_or_default();
        if resume::write(&root, &r, &n.title, &place).is_ok() {
            self.last_resume = Some(here);
        }
    }

    // ---- writing sessions ------------------------------------------------

    /// Push saved sessions off-site in the background, if there's a remote.
    fn back_up(&mut self) {
        if self.backup_rx.is_some() {
            return;
        }
        let root = self.project.root.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // No upstream yet (a remote just connected) counts as "not pushed".
            if sessions::has_remote(&root) && sessions::unpushed(&root).is_none_or(|n| n > 0) {
                let _ = tx.send(sessions::push(&root));
            }
        });
        self.backup_rx = Some(rx);
    }

    pub fn tick_backup(&mut self) {
        if let Some(rx) = &self.backup_rx {
            match rx.try_recv() {
                Ok(outcome) => {
                    self.backup_note = Some(match outcome {
                        sessions::PushOutcome::Pushed => "backed up ✓".into(),
                        sessions::PushOutcome::NoRemote => {
                            "no remote — sessions stay on this computer".into()
                        }
                        sessions::PushOutcome::Failed(why) => format!("couldn't back up: {why}"),
                    });
                    self.backup_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.backup_rx = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    /// Save this session as a snapshot now. Returns its label.
    fn save_session(&mut self) -> Option<String> {
        if !self.commit_saves() {
            return None;
        }
        self.save_resume(true);
        let root = self.project.root.clone();
        // Moving the cursor isn't a session; it rides along with the next one.
        if !sessions::has_writing(&root) {
            self.msg = "nothing new since the last session".into();
            return None;
        }
        let label = sessions::pending_label(&root, chrono::Local::now())
            .ok()
            .flatten();
        match sessions::commit_session(&root, chrono::Local::now()) {
            Ok(Some(_)) => {
                self.back_up();
                label
            }
            Ok(None) => {
                self.msg = "nothing new since the last session".into();
                None
            }
            Err(e) => {
                self.msg = format!("couldn't save the session: {e}");
                None
            }
        }
    }

    /// Back from a diff to the sessions list (the list is re-read; it's cheap).
    fn open_sessions_keeping_place(&mut self) {
        self.overlay = Overlay::None;
        self.open_sessions();
    }

    pub fn open_sessions(&mut self) {
        if !sessions::git_available() {
            self.msg =
                "writing sessions need Git — install it from git-scm.com, then try again".into();
            return;
        }
        if !self.sessions_on {
            self.overlay = Overlay::SessionsOff;
            return;
        }
        self.commit_saves();
        let root = self.project.root.clone();
        match sessions::sessions(&root, 200) {
            Ok(list) => {
                let pending = sessions::pending_label(&root, chrono::Local::now())
                    .ok()
                    .flatten()
                    .filter(|l| l.contains(" · "));
                let backup = if !sessions::has_remote(&root) {
                    "only on this computer — connect a remote (git remote add origin …) to back sessions up".to_string()
                } else if self.backup_rx.is_some() {
                    "backing up…".to_string()
                } else {
                    match (sessions::unpushed(&root), &self.backup_note) {
                        (Some(0), _) => "backed up ✓".to_string(),
                        (_, Some(note)) if note.starts_with("couldn't") => note.clone(),
                        (None, _) => "not backed up yet — it happens in the background".to_string(),
                        (Some(1), _) => "1 session waiting to back up".to_string(),
                        (Some(n), _) => format!("{n} sessions waiting to back up"),
                    }
                };
                self.overlay = Overlay::Sessions {
                    list,
                    sel: 0,
                    pending,
                    backup,
                    changes: None,
                };
            }
            Err(e) => self.msg = format!("couldn't read the sessions: {e}"),
        }
    }

    // ---- the codex -------------------------------------------------------

    pub fn rebuild_codex(&mut self) {
        let speller = self.speller.as_ref();
        let ordinary = |w: &str| speller.is_some_and(|s| s.is_correct(w));
        self.codex_index = codex::index(&self.project, &self.parents, &ordinary);
        // Keep an open note current.
        if let Some(pane) = &mut self.codex {
            match self.codex_index.iter().find(|e| e.note == pane.entry.note) {
                Some(e) => {
                    pane.entry = e.clone();
                    pane.appears = codex::appearances(&self.project, &self.parents, e);
                }
                None => self.codex = None,
            }
        }
    }

    /// Ctrl-O: open the note for the name (or [[link]]) under the cursor
    /// beside the scene. From the tree, a selected note opens directly.
    pub fn open_codex(&mut self) {
        self.flush();
        let entry = if self.focus == Focus::Tree {
            self.visible.get(self.sel).and_then(|&i| {
                self.codex_index
                    .iter()
                    .position(|e| e.note == self.project.nodes[i].path)
            })
        } else if self.open.is_some() {
            let (cy, cx) = (self.editor.cy, self.editor.cx);
            let line = &self.editor.lines[cy];
            codex::link_at(line, cx, &self.codex_index).or_else(|| {
                codex::spans(line, &self.codex_index)
                    .into_iter()
                    .find(|&(s, e, _)| cx >= s && cx <= e)
                    .map(|(_, _, i)| i)
            })
        } else {
            None
        };
        // Ctrl-O again, on the same name or on none, puts the note away.
        let showing = self.codex.as_ref().map(|c| c.entry.note.clone());
        if showing.is_some()
            && entry.is_none_or(|i| Some(&self.codex_index[i].note) == showing.as_ref())
        {
            self.close_codex();
            return;
        }
        let Some(i) = entry else {
            self.msg = if self.codex_index.is_empty() {
                "no notes yet — add one under Characters or Places".into()
            } else {
                "put the cursor on a name from your notebook".into()
            };
            return;
        };
        let e = self.codex_index[i].clone();
        let appears = codex::appearances(&self.project, &self.parents, &e);
        self.msg = format!(
            "{} · appears in {} scene{}",
            e.title,
            appears.len(),
            if appears.len() == 1 { "" } else { "s" }
        );
        self.close_beside();
        self.codex = Some(CodexPane {
            entry: e,
            appears,
            sel: 0,
            scroll: 0,
        });
    }

    pub fn on_codex_key(&mut self, key: Key) {
        let Some(pane) = &mut self.codex else {
            self.focus = Focus::Editor;
            return;
        };
        match key {
            Key::Down | Key::Char('j') => {
                pane.sel = (pane.sel + 1).min(pane.appears.len().saturating_sub(1))
            }
            Key::Up | Key::Char('k') => pane.sel = pane.sel.saturating_sub(1),
            Key::PageDown | Key::Char(' ') => pane.scroll += 5,
            Key::PageUp => pane.scroll = pane.scroll.saturating_sub(5),
            Key::Enter => {
                if let Some(a) = pane.appears.get(pane.sel).cloned()
                    && let Some(i) = self.project.nodes.iter().position(|n| n.path == a.scene)
                {
                    self.reveal(i);
                    self.open_scene(i);
                    self.focus = Focus::Codex;
                }
            }
            Key::Char('o') => {
                // Open the note itself for editing.
                let note = pane.entry.note.clone();
                if let Some(i) = self.project.nodes.iter().position(|n| n.path == note) {
                    self.reveal(i);
                    self.open_scene(i);
                }
            }
            Key::Esc | Key::Char('q') => self.close_codex(),
            _ => {}
        }
    }

    /// Change one setting and save, keeping every other one as it is on disk.
    fn save_setting(change: impl FnOnce(&mut Settings)) {
        let mut s = Settings::load();
        change(&mut s);
        let _ = s.save();
    }

    pub fn toggle_icons(&mut self) {
        self.icons_on = !self.icons_on;
        let on = self.icons_on;
        Self::save_setting(|s| s.icons = on);
        self.msg = if self.icons_on {
            "tree icons on".into()
        } else {
            "tree icons off".into()
        };
    }

    pub fn toggle_spellcheck(&mut self) {
        self.spell_on = !self.spell_on;
        let on = self.spell_on;
        Self::save_setting(|s| s.spellcheck = on);
        self.msg = if self.spell_on {
            "spellcheck on".into()
        } else {
            "spellcheck off".into()
        };
    }

    /// Misspelt words in one paragraph of the open scene, leaving out the word
    /// the cursor is in the middle of typing.
    pub fn misspellings(&self, line: usize) -> Vec<(usize, usize)> {
        let (Some(s), true) = (&self.speller, self.spell_on) else {
            return Vec::new();
        };
        let Some(text) = self.editor.lines.get(line) else {
            return Vec::new();
        };
        let typing = self.focus == Focus::Editor && self.editor.cy == line;
        s.misspellings(text)
            .into_iter()
            .filter(|&(a, b)| !(typing && self.editor.cx >= a && self.editor.cx <= b))
            .collect()
    }

    /// F8: suggestions for the misspelt word at the cursor, or, if the cursor
    /// isn't on one, jump to the next misspelling and offer those.
    pub fn spelling(&mut self) {
        if self.open.is_none() {
            self.msg = "open a scene to check its spelling".into();
            return;
        }
        let Some(speller) = &self.speller else {
            self.msg = "the dictionary is still loading".into();
            return;
        };
        self.focus = Focus::Editor;
        let (cy, cx) = (self.editor.cy, self.editor.cx);
        // Notes and TKs are the writer's own shorthand, not prose to correct.
        let marks = grimoire_core::notes::line_spans(&self.editor.lines);
        let bad = |l: usize| {
            App::misspellings_outside_marks(speller.misspellings(&self.editor.lines[l]), &marks[l])
        };
        let here = bad(cy)
            .into_iter()
            .find(|&(a, b)| cx >= a && cx <= b)
            .map(|(a, b)| (cy, a, b));
        let target = here.or_else(|| {
            let lines = &self.editor.lines;
            (0..lines.len())
                .map(|k| (cy + k) % lines.len())
                .flat_map(|l| bad(l).into_iter().map(move |(a, b)| (l, a, b)))
                .find(|&(l, a, _)| (l, a) > (cy, cx) || l < cy)
        });
        let Some((line, start, end)) = target else {
            self.msg = "no misspellings in this scene".into();
            return;
        };
        let word: String = self.editor.lines[line]
            .chars()
            .skip(start)
            .take(end - start)
            .collect();
        let suggestions = suggestions_for(speller, &word);
        self.editor.select((line, start), (line, end));
        self.overlay = Overlay::Spelling {
            line,
            start,
            end,
            word,
            suggestions,
            sel: 0,
            inline: false,
        };
    }

    /// After a click in the prose: if the caret landed on a misspelt word
    /// (not inside a `%% note %%`), offer its suggestions right under it.
    /// True if it did.
    pub fn offer_spelling_at_caret(&mut self) -> bool {
        let (Some(speller), true, Some(_)) = (&self.speller, self.spell_on, self.open) else {
            return false;
        };
        let (line, cx) = (self.editor.cy, self.editor.cx);
        let Some(text) = self.editor.lines.get(line) else {
            return false;
        };
        let marks = grimoire_core::notes::line_spans(&self.editor.lines);
        let Some((start, end)) =
            App::misspellings_outside_marks(speller.misspellings(text), &marks[line])
                .into_iter()
                .find(|&(a, b)| cx >= a && cx < b)
        else {
            return false;
        };
        let word: String = text.chars().skip(start).take(end - start).collect();
        let suggestions = suggestions_for(speller, &word);
        self.overlay = Overlay::Spelling {
            line,
            start,
            end,
            word,
            suggestions,
            sel: 0,
            inline: true,
        };
        true
    }

    /// Put `with` where the misspelt word was — one undoable edit — unless
    /// the text has moved under the popup since it opened.
    fn apply_spelling(&mut self, line: usize, start: usize, end: usize, word: &str, with: &str) {
        let still: Option<String> = self
            .editor
            .lines
            .get(line)
            .map(|l| l.chars().skip(start).take(end - start).collect());
        if still.as_deref() != Some(word) {
            self.msg = "the text changed under it — try again".into();
            return;
        }
        self.editor.replace_in_line(line, start, end, with);
        self.flush();
    }

    /// Accept a word from now on: until Grimoire closes, in this book
    /// (dictionary.txt), or in every book (the writer's own list).
    fn ignore_word(&mut self, word: &str, scope: Ignore) {
        match scope {
            Ignore::Now => {
                if let Some(s) = &mut self.speller {
                    s.add_words([word.to_string()]);
                }
                self.msg = format!("“{word}” ignored until you close Grimoire");
            }
            Ignore::Book => self.add_to_book(word),
            Ignore::Everywhere => {
                let Some(path) = self.user_dict.clone() else {
                    self.msg = "there's no word list for every book here".into();
                    return;
                };
                match spell::add_user_word(&path, word) {
                    Ok(()) => {
                        if let Some(s) = &mut self.speller {
                            s.add_words([word.to_string()]);
                        }
                        self.msg = format!("“{word}” is now a word in every book");
                    }
                    Err(e) => self.msg = format!("couldn't add it: {e}"),
                }
            }
        }
    }

    fn add_to_book(&mut self, word: &str) {
        match spell::add_book_word(&self.project.root, word) {
            Ok(()) => {
                if let Some(s) = &mut self.speller {
                    s.add_words([word.to_string()]);
                }
                self.msg = format!("“{word}” added to this book's dictionary (dictionary.txt)");
            }
            Err(e) => self.msg = format!("couldn't add it: {e}"),
        }
    }

    fn fix_name(&mut self, variant: &str, name: &str) {
        let root = self.project.root.clone();
        let mut total = 0;
        for i in 0..self.project.nodes.len() {
            let n = &self.project.nodes[i];
            if n.kind != Kind::Scene || !n.in_manuscript || self.project.in_trash(i) || n.read_only
            {
                continue;
            }
            let (text, count) = search::replace_word(&n.body, variant, name);
            if count == 0 {
                continue;
            }
            let path = n.path.clone();
            let _ = history::snapshot(&root, &path, &n.file_text(), None);
            if self.open == Some(i) {
                self.editor.set_text(&text);
            }
            self.project.nodes[i].body = text;
            self.mark_changed(i);
            total += count;
        }
        self.commit_saves();
        self.msg = format!(
            "{variant} → {name} in {total} place{}",
            if total == 1 { "" } else { "s" }
        );
    }

    // ---- scene history -------------------------------------------------

    pub fn open_history(&mut self) {
        let idx = match self.focus {
            Focus::Editor => self.open,
            _ => self.visible.get(self.sel).copied(),
        };
        let Some(idx) = idx.filter(|&i| self.project.nodes[i].kind == Kind::Scene) else {
            self.msg = "pick a scene to see its history".into();
            return;
        };
        // Keep what's there now, so the list always starts from here.
        self.commit_saves();
        let n = &self.project.nodes[idx];
        let versions = history::versions(&self.project.root, &n.path);
        if versions.is_empty() {
            self.msg = format!("no history for {} yet — it's kept as you write", n.title);
            return;
        }
        self.overlay = Overlay::History {
            scene: n.path.clone(),
            title: n.title.clone(),
            versions,
            sel: 0,
            scroll: 0,
        };
    }

    fn restore_version(&mut self, scene: &Path, text: &str) {
        let root = self.project.root.clone();
        let Some(idx) = self.project.nodes.iter().position(|n| n.path == scene) else {
            return;
        };
        // What's there now goes into history first, so restoring is undoable
        // from the history list as well as with Ctrl-Z.
        let _ = history::snapshot(&root, scene, &self.project.nodes[idx].file_text(), None);
        let (front, body) = grimoire_core::project::split_frontmatter(text);
        if self.open != Some(idx) {
            self.open_scene(idx);
        }
        self.project.nodes[idx].front = front;
        self.editor.set_text(&body);
        self.flush();
        self.mark_changed(idx);
        self.focus = Focus::Editor;
        self.msg = "restored — Ctrl-Z undoes it".into();
    }

    // ---- creating scenes, chapters and parts -------------------------------

    /// Open the naming prompt for `n`, `c`, `p` or `N`, or say why not.
    /// Where things go is `create::plan`'s job.
    pub fn start_create(&mut self, want: New) {
        let sel = self.visible.get(self.sel).copied();
        match create::plan(&self.project, &self.parents, sel, want) {
            Ok(plan) => {
                let fresh = !plan.name.is_empty();
                self.overlay = Overlay::Create {
                    buf: plan.name.clone(),
                    plan,
                    fresh,
                };
            }
            Err(why) => {
                self.overlay = Overlay::None;
                self.msg = why;
            }
        }
    }

    fn finish_create(&mut self, plan: Plan, name: String) {
        // Save first, so re-reading the tree can't lose an unsaved sentence.
        self.flush();
        let saved = self.project.dirty_count();
        if !self.commit_saves() {
            return;
        }
        let path = match project::create(&plan.dir, &name, plan.folder) {
            Ok(p) => p,
            Err(e) => {
                self.msg = format!("couldn't create it: {e}");
                return;
            }
        };
        let what = if name.trim().is_empty() {
            plan.noun.clone()
        } else {
            name.trim().to_string()
        };
        // Brand new: nothing parked or kept for an older thing at this path
        // (deleted before its history followed it) is this one's.
        self.undo_stash.retain(|p, _| !p.starts_with(&path));
        history::set_aside(&self.project.root, &path);
        self.record(TreeStep::Created {
            what,
            path: path.clone(),
            trashed: None,
        });
        if let Err(e) = self.reload_tree() {
            self.msg = format!("created, but couldn't re-read the tree: {e}");
            return;
        }
        let mut made = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if let Some(i) = self.project.nodes.iter().position(|n| n.path == path) {
            made = self.project.nodes[i].title.clone();
            let mut p = self.parents[i];
            while let Some(pi) = p {
                self.project.nodes[pi].expanded = true;
                p = self.parents[pi];
            }
            self.refresh_visible();
            if let Some(pos) = self.visible.iter().position(|&v| v == i) {
                self.sel = pos;
            }
            if plan.folder {
                // The next key is n or c, which only the tree hears.
                self.focus = Focus::Tree;
            } else {
                self.open_scene(i);
            }
        }
        // Say what comes next, so an empty chapter isn't a dead end.
        let next = match plan.made {
            New::Chapter => " · n adds a scene to it",
            New::Part => " · c adds a chapter to it",
            _ => "",
        };
        let also = if saved > 0 {
            format!(" · saved {saved} first")
        } else {
            String::new()
        };
        self.msg = format!("made {made}{next}{also}");
    }

    // ---- renaming and deleting ---------------------------------------------

    /// What the selected row is, in the book's own words.
    fn noun_of(&self, idx: usize) -> String {
        let n = &self.project.nodes[idx];
        if n.kind == Kind::Category {
            return "section".into();
        }
        if self.project.in_trash(idx) {
            return if n.kind == Kind::Container {
                "folder".into()
            } else {
                "file".into()
            };
        }
        if !n.in_manuscript {
            if n.kind == Kind::Container {
                return "folder".into();
            }
            return match n.area {
                grimoire_core::project::Area::FrontMatter
                | grimoire_core::project::Area::Format => "document",
                grimoire_core::project::Area::Templates => "sheet",
                _ => "note",
            }
            .into();
        }
        match n.kind {
            Kind::Scene => "scene".into(),
            _ => match manuscript::section_of(&self.project, idx) {
                manuscript::Section::Part => self.project.meta.part_noun(),
                _ => "chapter".into(),
            },
        }
    }

    /// The row the tree is on, unless it's one of the section headings.
    fn selected_file(&mut self) -> Option<usize> {
        let idx = self.visible.get(self.sel).copied()?;
        if self.project.nodes[idx].kind == Kind::Category {
            self.msg = "that's a section — pick what's inside it".into();
            return None;
        }
        Some(idx)
    }

    pub fn start_rename(&mut self) {
        let Some(idx) = self.selected_file() else {
            return;
        };
        let n = &self.project.nodes[idx];
        self.overlay = Overlay::Rename {
            path: n.path.clone(),
            buf: n.title.clone(),
            fresh: true,
            noun: self.noun_of(idx),
        };
    }

    fn finish_rename(&mut self, path: PathBuf, name: String) {
        self.flush();
        if !self.commit_saves() {
            return;
        }
        let old = self
            .project
            .nodes
            .iter()
            .find(|n| n.path == path)
            .map(|n| n.title.clone())
            .unwrap_or_default();
        let to = match project::rename(&path, &name) {
            Ok(p) => p,
            Err(e) => {
                self.msg = format!("couldn't rename it: {e}");
                return;
            }
        };
        self.record(TreeStep::Renamed {
            from: path.clone(),
            to: to.clone(),
            old,
            new: name.trim().to_string(),
        });
        self.follow_paths(&path, &to);
        if let Err(e) = self.reload_tree() {
            self.msg = format!("renamed, but couldn't re-read the tree: {e}");
            return;
        }
        if let Some(i) = self.project.nodes.iter().position(|n| n.path == to) {
            if let Some(pos) = self.visible.iter().position(|&v| v == i) {
                self.sel = pos;
            }
            self.msg = format!("renamed to {}", self.project.nodes[i].title);
        }
    }

    pub fn start_delete(&mut self) {
        let Some(idx) = self.selected_file() else {
            return;
        };
        let n = &self.project.nodes[idx];
        self.overlay = Overlay::Confirm {
            path: n.path.clone(),
            name: n.title.clone(),
            noun: self.noun_of(idx),
            words: self.project.subtree_words(idx),
            permanent: self.project.in_trash(idx),
        };
    }

    fn finish_delete(&mut self, path: PathBuf, name: String, permanent: bool) {
        let before = self.project.total_words();
        self.delete_path(path, name, permanent);
        self.keep_today(before);
    }

    fn delete_path(&mut self, path: PathBuf, name: String, permanent: bool) {
        self.flush();
        if !self.commit_saves() {
            return;
        }
        let root = self.project.root.clone();
        // Close the editor if what's going is the scene it's showing, or holds it.
        let open_path = self.open.map(|i| self.project.nodes[i].path.clone());
        if let Some(open) = open_path.filter(|p| p.starts_with(&path)) {
            // Its undo goes with it, so Ctrl-Z on the delete brings that back too.
            self.undo_stash.insert(open, self.editor.take_history());
            self.open = None;
            self.editor = Editor::from_text("");
            self.focus = Focus::Tree;
        }
        let done = if permanent {
            project::destroy(&path).map(|_| {
                self.undo_stash.retain(|p, _| !p.starts_with(&path));
                String::new()
            })
        } else {
            let m = self.mod_label();
            project::trash(&root, &path).map(|to| {
                // Undo and history follow it into the trash, so nothing made
                // later at the same path inherits them.
                self.follow_paths(&path, &to);
                self.record(TreeStep::Moved {
                    what: format!("delete {name}"),
                    batches: vec![vec![(path.clone(), to)]],
                    links: false,
                });
                format!(" — {m}Z puts it back")
            })
        };
        match done {
            Ok(where_to) => {
                if let Err(e) = self.reload_tree() {
                    self.msg = format!("deleted, but couldn't re-read the tree: {e}");
                    return;
                }
                self.msg = format!("deleted {name}{where_to}");
            }
            Err(e) => self.msg = format!("couldn't delete it: {e}"),
        }
    }

    /// Something on disk moved from `from` to `to` (a scene or a whole folder):
    /// its history, its parked undo and the session bookkeeping go with it.
    fn follow_paths(&mut self, from: &Path, to: &Path) {
        self.follow_many(&[(from.to_path_buf(), to.to_path_buf())]);
    }

    /// Several things moved at once (a swap, a shift along). Every path is
    /// mapped from where it was before any of them moved, so a swap can't map
    /// something twice.
    fn follow_many(&mut self, renames: &[(PathBuf, PathBuf)]) {
        let root = self.project.root.clone();
        history::follow_all(&root, renames);
        let moved = |p: &PathBuf| {
            renames.iter().find_map(|(from, to)| {
                p.strip_prefix(from).ok().map(|rest| {
                    if rest.as_os_str().is_empty() {
                        to.to_path_buf()
                    } else {
                        to.join(rest)
                    }
                })
            })
        };
        self.undo_stash = std::mem::take(&mut self.undo_stash)
            .into_iter()
            .map(|(p, h)| (moved(&p).unwrap_or(p), h))
            .collect();
        self.pre_snapshotted = std::mem::take(&mut self.pre_snapshotted)
            .into_iter()
            .map(|p| moved(&p).unwrap_or(p))
            .collect();
        // Every node follows, not just the open one: a scene with unsaved
        // words is found again by its new path when the tree is re-read.
        for n in &mut self.project.nodes {
            if let Some(p) = moved(&n.path) {
                n.path = p;
            }
        }
    }

    /// Re-read the tree from disk, keeping what's folded, which scene is open,
    /// and the editor exactly as it is. Words not on disk yet ride across,
    /// still guarded against the version they were written over: the new tree
    /// comes from the files, and anything unsaved would otherwise go with the
    /// old one.
    fn reload_tree(&mut self) -> Result<()> {
        self.flush();
        let unsaved: Vec<project::Node> = self
            .project
            .nodes
            .iter()
            .filter(|n| n.dirty && n.kind == Kind::Scene)
            .cloned()
            .collect();
        let collapsed: Vec<(PathBuf, bool)> = self
            .project
            .nodes
            .iter()
            .filter(|n| n.kind != Kind::Scene)
            .map(|n| (n.path.clone(), n.expanded))
            .collect();
        let open_path = self.open.map(|i| self.project.nodes[i].path.clone());
        let root = self.project.root.clone();
        let stranded = project::restore_stranded(&root);
        if !stranded.is_empty() {
            self.msg = Self::stranded_note(&stranded);
        }
        self.project = Project::load(&root)?;
        for n in &mut self.project.nodes {
            if let Some(&(_, open)) = collapsed.iter().find(|(p, _)| *p == n.path) {
                n.expanded = open;
            }
        }
        self.parents = self.project.parents();
        self.open = open_path.and_then(|p| self.project.nodes.iter().position(|n| n.path == p));
        for old in unsaved {
            match self.project.nodes.iter().position(|n| n.path == old.path) {
                Some(i) => {
                    let n = &mut self.project.nodes[i];
                    n.front = old.front;
                    n.body = old.body;
                    n.seen = old.seen;
                    n.stat = None;
                    self.mark_changed(i);
                }
                // Gone from where it was: keep its words where the next launch
                // looks for them rather than let them drop.
                None => {
                    let _ = recovery::keep(&root, &old.path, &old.file_text());
                }
            }
        }
        // Nothing unsaved in the editor, and the file says something else (a
        // link rewritten by a move or its undo, a change from elsewhere): show
        // the file, or the next keystroke would save the old words over it.
        if let Some(i) = self.open
            && !self.project.nodes[i].dirty
            && self.editor.text() != self.project.nodes[i].body
        {
            let body = self.project.nodes[i].body.clone();
            self.editor.set_text(&body);
        }
        self.refresh_visible();
        self.refresh_names();
        Ok(())
    }

    /// Said when [`project::restore_stranded`] brought something back.
    pub fn stranded_note(back: &[PathBuf]) -> String {
        let names: Vec<String> = back
            .iter()
            .take(3)
            .map(|p| {
                let stem = p
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                stem.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-')
                    .replace('-', " ")
            })
            .collect();
        format!(
            "brought back {} left hidden by a move that didn't finish: {}{}",
            back.len(),
            names.join(", "),
            if back.len() > 3 { ", …" } else { "" }
        )
    }

    // ---- the corkboard --------------------------------------------------

    pub fn cork_cols(&self) -> usize {
        ((self.screen.0.saturating_sub(4)) / CARD_W).max(1) as usize
    }

    pub fn cork_scope(&self, scope: &Option<PathBuf>) -> Option<usize> {
        scope
            .as_ref()
            .and_then(|p| self.project.nodes.iter().position(|n| &n.path == p))
    }

    pub fn open_cork(&mut self) {
        self.flush();
        let at = match self.focus {
            Focus::Editor => self.open,
            _ => self.visible.get(self.sel).copied().or(self.open),
        };
        let parts = cork::parts(&self.project);
        let part = at
            .and_then(|i| cork::part_of(&self.project, &self.parents, i))
            .or_else(|| parts.first().copied());
        let groups = cork::board(&self.project, part);
        if groups.is_empty() {
            self.msg = "no scenes to put on the board yet".into();
            return;
        }
        let sel = groups
            .iter()
            .flat_map(|g| &g.cards)
            .position(|c| Some(c.idx) == at)
            .unwrap_or(0);
        self.overlay = Overlay::Cork {
            scope: part.map(|i| self.project.nodes[i].path.clone()),
            sel,
            pov: None,
            typing: None,
        };
    }

    // ---- undoing what the tree did ---------------------------------------

    fn record(&mut self, step: TreeStep) {
        self.replace_undoable = matches!(step, TreeStep::Replaced { .. });
        self.replace_redoable = false;
        self.tree_undo.push(step);
        if self.tree_undo.len() > 100 {
            self.tree_undo.remove(0);
        }
        self.tree_redo.clear();
    }

    /// Ctrl-Z (`back`) or Ctrl-Y outside the editor: take back the last thing
    /// done in the tree, or do again the last thing taken back.
    fn tree_step(&mut self, back: bool) {
        let before = self.project.total_words();
        self.take_step(back);
        self.keep_today(before);
    }

    fn take_step(&mut self, back: bool) {
        let Some(step) = (if back {
            self.tree_undo.pop()
        } else {
            self.tree_redo.pop()
        }) else {
            self.msg = if back {
                "nothing to undo".into()
            } else {
                "nothing to redo".into()
            };
            return;
        };
        self.flush();
        if !self.commit_saves() {
            // Nothing was touched; keep the step for when saving works.
            if back {
                self.tree_undo.push(step)
            } else {
                self.tree_redo.push(step)
            }
            return;
        }
        let root = self.project.root.clone();
        let (result, step, show, what) = match step {
            TreeStep::Moved {
                what,
                batches,
                links,
            } => {
                let order: Vec<Vec<(PathBuf, PathBuf)>> = if back {
                    batches
                        .iter()
                        .rev()
                        .map(|b| b.iter().map(|(f, t)| (t.clone(), f.clone())).collect())
                        .collect()
                } else {
                    batches.clone()
                };
                let mut result = Ok(());
                let mut show = None;
                for batch in &order {
                    self.close_if_trashed(batch);
                    if let Err(e) = project::apply_moves(&root, batch, links) {
                        result = Err(e);
                        break;
                    }
                    self.follow_many(batch);
                    show = batch.first().map(|(_, t)| t.clone());
                }
                (
                    result,
                    TreeStep::Moved {
                        what: what.clone(),
                        batches,
                        links,
                    },
                    show,
                    what,
                )
            }
            TreeStep::Renamed { from, to, old, new } => {
                let (at, name) = if back { (&to, &old) } else { (&from, &new) };
                let result = project::rename(at, name).map(|now| {
                    self.follow_paths(at, &now);
                });
                let show = Some(if back { from.clone() } else { to.clone() });
                let what = format!("rename to {new}");
                (result, TreeStep::Renamed { from, to, old, new }, show, what)
            }
            TreeStep::Sections {
                what,
                before,
                after,
            } => {
                let order = if back { &before } else { &after };
                let result = project::save_section_order(&root, order);
                (
                    result,
                    TreeStep::Sections {
                        what: what.clone(),
                        before,
                        after,
                    },
                    None,
                    what,
                )
            }
            TreeStep::Created {
                what,
                path,
                trashed,
            } => {
                let (result, trashed, show) = if back {
                    let pair = project::trash(&root, &path).map(|t| vec![(path.clone(), t)]);
                    match pair {
                        Ok(batch) => {
                            self.close_if_trashed(&batch);
                            self.follow_many(&batch);
                            let t = batch[0].1.clone();
                            (Ok(()), Some(t), None)
                        }
                        Err(e) => (Err(e), trashed, None),
                    }
                } else {
                    match &trashed {
                        Some(t) => {
                            let batch = vec![(t.clone(), path.clone())];
                            let result = project::apply_moves(&root, &batch, false);
                            if result.is_ok() {
                                self.follow_many(&batch);
                            }
                            (result, None, Some(path.clone()))
                        }
                        None => (Ok(()), None, Some(path.clone())),
                    }
                };
                let label = format!("create {what}");
                (
                    result,
                    TreeStep::Created {
                        what,
                        path,
                        trashed,
                    },
                    show,
                    label,
                )
            }
            TreeStep::Replaced { what, scenes } => {
                let skipped = self.swap_replaced(&scenes, back);
                let result = if self.commit_saves() {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("a scene couldn't be saved"))
                };
                let label = if skipped > 0 {
                    format!(
                        "{what} ({skipped} scene{} changed since, left as {})",
                        if skipped == 1 { "" } else { "s" },
                        if skipped == 1 { "it is" } else { "they are" }
                    )
                } else {
                    what.clone()
                };
                self.replace_undoable = !back;
                self.replace_redoable = back;
                (result, TreeStep::Replaced { what, scenes }, None, label)
            }
        };
        match result {
            Ok(()) => {
                if back {
                    self.tree_redo.push(step)
                } else {
                    self.tree_undo.push(step)
                }
                if let Err(e) = self.reload_tree() {
                    self.msg = format!("couldn't re-read the tree: {e}");
                    return;
                }
                if let Some(i) =
                    show.and_then(|p| self.project.nodes.iter().position(|n| n.path == p))
                {
                    self.reveal(i);
                }
                let m = self.mod_label();
                self.msg = if back {
                    format!("undid: {what} · {m}Y redoes it")
                } else {
                    format!("redid: {what}")
                };
            }
            Err(e) => {
                // It can't be taken back now (something else changed on disk);
                // drop it rather than leave a step that will never work.
                self.msg = format!(
                    "couldn't {} {what}: {e}",
                    if back { "undo" } else { "redo" }
                );
            }
        }
    }

    /// If a move sends the open scene (or what holds it) to the trash, close it.
    fn close_if_trashed(&mut self, batch: &[(PathBuf, PathBuf)]) {
        let bin = project::trash_dir(&self.project.root);
        let Some(open) = self.open.map(|i| self.project.nodes[i].path.clone()) else {
            return;
        };
        if batch
            .iter()
            .any(|(from, to)| to.starts_with(&bin) && open.starts_with(from))
        {
            self.open = None;
            self.editor = Editor::from_text("");
            self.focus = Focus::Tree;
        }
    }

    // ---- moving scenes and chapters -------------------------------------

    /// Alt-↑ / Alt-↓: move the selected scene or folder one place.
    pub fn move_selected(&mut self, up: bool) {
        self.select_open_scene();
        if let Some(idx) = self.visible.get(self.sel).copied()
            && self.project.roots.contains(&idx)
        {
            self.move_section(idx, up);
            return;
        }
        let Some(idx) = self.selected_file() else {
            return;
        };
        if self.project.in_trash(idx) {
            self.msg = "things in the trash stay where they are".into();
            return;
        }
        let name = self.project.nodes[idx].title.clone();
        let noun = self.noun_of(idx);
        if !self.commit_saves() {
            return;
        }
        let root = self.project.root.clone();
        let path = self.project.nodes[idx].path.clone();
        let moved = match project::move_item(&root, &path, up) {
            Ok(m) => m,
            Err(e) => {
                self.msg = format!("couldn't move {name}: {e}");
                return;
            }
        };
        self.follow_many(&moved.renames);
        self.record(TreeStep::Moved {
            what: format!("move {name}"),
            batches: vec![moved.renames.clone()],
            links: true,
        });
        if let Err(e) = self.reload_tree() {
            self.msg = format!("moved, but couldn't re-read the tree: {e}");
            return;
        }
        if let Some(i) = self
            .project
            .nodes
            .iter()
            .position(|n| n.path == moved.target)
        {
            self.reveal(i);
        }
        let links = match moved.links {
            0 => String::new(),
            1 => " · links in 1 file updated".into(),
            n => format!(" · links in {n} files updated"),
        };
        self.msg = format!(
            "moved {noun} {name} {}{links}",
            if up { "up" } else { "down" }
        );
    }

    /// Move a whole section one place up or down among the others. The order
    /// is the book's, kept in novel.toml; the trash stays at the bottom.
    fn move_section(&mut self, idx: usize, up: bool) {
        use grimoire_core::project::Area;
        let area = self.project.nodes[idx].area;
        let name = self.project.nodes[idx].title.clone();
        if area == Area::Trash {
            self.msg = "the trash stays at the bottom".into();
            return;
        }
        let shown: Vec<Area> = self
            .project
            .roots
            .iter()
            .map(|&r| self.project.nodes[r].area)
            .collect();
        let at = shown.iter().position(|&a| a == area).unwrap_or(0);
        let other =
            if up { at.checked_sub(1) } else { Some(at + 1) }.and_then(|i| shown.get(i).copied());
        let Some(other) = other.filter(|&o| o != Area::Trash) else {
            self.msg = format!("{name} is already {}", if up { "first" } else { "last" });
            return;
        };
        let before = Area::ordered(&self.project.meta);
        let mut after = before.clone();
        let (i, j) = (
            after.iter().position(|&a| a == area).unwrap(),
            after.iter().position(|&a| a == other).unwrap(),
        );
        after.swap(i, j);
        if let Err(e) = self.set_section_order(&after, Some(area)) {
            self.msg = format!("couldn't move {name}: {e}");
            return;
        }
        self.record(TreeStep::Sections {
            what: format!("move {name}"),
            before,
            after,
        });
        self.msg = format!("moved section {name} {}", if up { "up" } else { "down" });
    }

    fn set_section_order(
        &mut self,
        order: &[grimoire_core::project::Area],
        show: Option<grimoire_core::project::Area>,
    ) -> Result<()> {
        // Save first, like every other change to the tree.
        self.commit_saves();
        project::save_section_order(&self.project.root, order)?;
        self.reload_tree()?;
        if let Some(i) = show.and_then(|a| {
            self.project
                .roots
                .iter()
                .copied()
                .find(|&r| self.project.nodes[r].area == a)
        }) {
            self.reveal(i);
        }
        Ok(())
    }

    /// A drag in the tree ended on another row: move there one step at a
    /// time, so crossing chapters works exactly as it does from the keyboard.
    pub fn drop_tree_drag(&mut self) {
        let Some((from, to)) = self.tree_drag.take() else {
            return;
        };
        if from == to || from >= self.visible.len() {
            return;
        }
        let idx = self.visible[from];
        let up = to < from;
        let mut path = self.project.nodes[idx].path.clone();
        // However many single moves a drag takes, it's one thing to undo.
        let before = self.tree_undo.len();
        self.drag_moves(up, to, &mut path);
        if self.tree_undo.len() > before + 1 {
            let steps: Vec<TreeStep> = self.tree_undo.drain(before..).collect();
            let merged = match (steps.first(), steps.last()) {
                (
                    Some(TreeStep::Sections { what, before, .. }),
                    Some(TreeStep::Sections { after, .. }),
                ) => TreeStep::Sections {
                    what: what.clone(),
                    before: before.clone(),
                    after: after.clone(),
                },
                _ => {
                    let batches = steps
                        .into_iter()
                        .flat_map(|s| match s {
                            TreeStep::Moved { batches, .. } => batches,
                            _ => Vec::new(),
                        })
                        .collect();
                    let name = self
                        .project
                        .nodes
                        .iter()
                        .find(|n| n.path == path)
                        .map(|n| n.title.clone())
                        .unwrap_or_default();
                    TreeStep::Moved {
                        what: format!("move {name}"),
                        batches,
                        links: true,
                    }
                }
            };
            self.tree_undo.push(merged);
        }
    }

    fn drag_moves(&mut self, up: bool, to: usize, path: &mut PathBuf) {
        for _ in 0..40 {
            let Some(i) = self.project.nodes.iter().position(|n| n.path == *path) else {
                return;
            };
            let Some(pos) = self.visible.iter().position(|&v| v == i) else {
                return;
            };
            if (up && pos <= to) || (!up && pos >= to) {
                return;
            }
            self.sel = pos;
            self.focus = Focus::Tree;
            self.move_selected(up);
            // Follow the dragged thing under its new name. A section keeps its
            // name, so for one it's enough that its row moved.
            match self
                .visible
                .get(self.sel)
                .map(|&v| self.project.nodes[v].path.clone())
            {
                Some(p) if p != *path => *path = p,
                Some(_) if self.sel != pos => {}
                _ => return,
            }
        }
    }

    // ---- key handling -----------------------------------------------------

    pub fn on_tree_key(&mut self, key: Key) {
        match key {
            Key::Char('t') => self.open_theme_picker(),
            Key::Char('n') => self.start_create(New::Scene),
            Key::Char('c') => self.start_create(New::Chapter),
            Key::Char('p') => self.start_create(New::Part),
            Key::Char('N') => self.start_create(New::Folder),
            Key::Char('r') => self.start_rename(),
            Key::Char('d') => self.start_delete(),
            Key::Char('H') => self.open_history(),
            Key::Char('b') => self.open_cork(),
            Key::Char('K') => self.move_selected(true),
            Key::Char('J') => self.move_selected(false),
            Key::Char('/') => self.open_find_book(String::new()),
            Key::Char('v') => self.open_beside(),
            Key::Up => self.sel = self.sel.saturating_sub(1),
            Key::Down => {
                if self.sel + 1 < self.visible.len() {
                    self.sel += 1;
                }
            }
            Key::Left => {
                let idx = self.visible[self.sel];
                let n = &self.project.nodes[idx];
                if n.kind != Kind::Scene && n.expanded {
                    self.project.nodes[idx].expanded = false;
                    self.refresh_visible();
                } else if let Some(p) = self.parents[idx]
                    && let Some(pos) = self.visible.iter().position(|&v| v == p)
                {
                    self.sel = pos;
                }
            }
            // Space is a second Enter here — folding is the most repeated
            // action in the tree and the thumb is already there.
            Key::Enter | Key::Char(' ') => {
                let idx = self.visible[self.sel];
                match self.project.nodes[idx].kind {
                    Kind::Container | Kind::Category => {
                        let open = self.project.nodes[idx].expanded;
                        self.project.nodes[idx].expanded = !open;
                        self.refresh_visible();
                    }
                    Kind::Scene => self.open_scene(idx),
                }
            }
            Key::Right => {
                let idx = self.visible[self.sel];
                match self.project.nodes[idx].kind {
                    Kind::Container | Kind::Category => {
                        self.project.nodes[idx].expanded = true;
                        self.refresh_visible();
                    }
                    Kind::Scene => self.open_scene(idx),
                }
            }
            _ => {}
        }
    }

    pub fn on_editor_key(&mut self, key: Key) {
        // With undo, a selection behaves the way it does everywhere else:
        // typing replaces it and Backspace or Delete removes it. Anything else
        // just lets go of it.
        let replacing = matches!(
            key,
            Key::Char(_) | Key::Enter | Key::Backspace | Key::Delete
        ) && self.editor.delete_selection();
        if !replacing {
            self.editor.clear_selection();
        }
        let rows = self.editor.layout(self.edit_width);
        match key {
            Key::Char(c) => self.editor.insert(c),
            Key::Enter => self.editor.newline(),
            Key::Backspace | Key::Delete if replacing => {}
            Key::Backspace => self.editor.backspace(),
            Key::Delete => self.editor.delete(),
            Key::Left => self.editor.left(),
            Key::Right => self.editor.right(),
            Key::Up => self.editor.up(&rows),
            Key::Down => self.editor.down(&rows),
            Key::Home => self.editor.home(&rows),
            Key::End => self.editor.end(&rows),
            Key::PageUp => {
                for _ in 0..self.edit_height.max(1) {
                    self.editor.up(&rows);
                }
            }
            Key::PageDown => {
                for _ in 0..self.edit_height.max(1) {
                    self.editor.down(&rows);
                }
            }
            _ => {}
        }
        if matches!(
            key,
            Key::Char(_) | Key::Enter | Key::Backspace | Key::Delete
        ) {
            self.flush();
        }
    }

    fn focusable(&self, f: Focus) -> bool {
        match f {
            // Focus mode hides the tree; Tab mustn't land somewhere unseen.
            Focus::Tree => !self.focus_mode,
            Focus::Editor => self.open.is_some(),
            Focus::Codex => self.codex.is_some(),
            Focus::Beside => self.beside.is_some() && self.side_room,
            Focus::Clearing => self.scene_visible,
            Focus::Music => self.music_visible,
        }
    }

    /// Tab through every pane that is actually on screen.
    pub fn cycle_focus(&mut self, forward: bool) {
        const ORDER: [Focus; 6] = [
            Focus::Tree,
            Focus::Editor,
            Focus::Codex,
            Focus::Beside,
            Focus::Clearing,
            Focus::Music,
        ];
        let cur = ORDER.iter().position(|&f| f == self.focus).unwrap_or(0);
        for step in 1..=ORDER.len() {
            let i = if forward {
                (cur + step) % ORDER.len()
            } else {
                (cur + ORDER.len() - step) % ORDER.len()
            };
            if self.focusable(ORDER[i]) {
                if self.focus == Focus::Editor {
                    self.flush();
                }
                self.focus = ORDER[i];
                return;
            }
        }
    }

    pub fn on_clearing_key(&mut self, key: Key) {
        match key {
            Key::Right | Key::Char('l') => self.pane_mode = self.pane_mode.next(),
            Key::Left | Key::Char('h') => self.pane_mode = self.pane_mode.prev(),
            Key::Enter | Key::Char(' ') => self.pomo.toggle(),
            Key::Char('r') => {
                self.pomo.reset();
                self.msg = "timer reset".into();
            }
            _ => {}
        }
    }

    /// The music pane answers every one of the player's (F7) keys.
    pub fn on_music_key(&mut self, key: Key) {
        if key == Key::Enter {
            self.open_player();
        } else {
            self.transport_key(key);
        }
    }

    /// The player's own keys, the same on the music pane and in the F7
    /// player: `[` `]` track, `←` `→` seek, space, `s` shuffle, `r` repeat,
    /// `+` `-` volume, `l` like. True if `key` was one of them. What each did
    /// comes back from the player as a note ("repeat: one").
    pub fn transport_key(&mut self, key: Key) -> bool {
        use music::Cmd;
        let cmd = match key {
            Key::Char(' ') => Cmd::PlayPause,
            Key::Right => Cmd::Seek(10),
            Key::Left => Cmd::Seek(-10),
            Key::Char(']') | Key::Char('n') => Cmd::Next,
            Key::Char('[') | Key::Char('p') => Cmd::Prev,
            Key::Char('s') => Cmd::Shuffle,
            Key::Char('r') => Cmd::Repeat,
            Key::Char('+') | Key::Char('=') => Cmd::Volume(10),
            Key::Char('-') => Cmd::Volume(-10),
            Key::Char('l') => Cmd::Like,
            _ => return false,
        };
        self.music.send(cmd);
        true
    }

    /// A left click focuses the pane under the pointer and acts on it.
    pub fn on_click(&mut self, x: u16, y: u16) {
        // The spelling popup takes clicks on itself; any other click closes
        // it and goes on as usual.
        if matches!(self.overlay, Overlay::Spelling { .. }) && self.click_spelling(x, y) {
            return;
        }
        let clicked = self
            .create_hits
            .iter()
            .find(|(r, _)| hit(*r, x, y))
            .map(|&(_, w)| w);
        let view = self
            .view_hits
            .iter()
            .find(|(r, _)| hit(*r, x, y))
            .map(|&(_, m)| m);
        if let Some(mode) = view {
            if self.focus == Focus::Editor {
                self.flush();
            }
            self.focus = Focus::Clearing;
            self.pane_mode = mode;
        } else if let Some(want) = clicked {
            if self.focus == Focus::Editor {
                self.flush();
            }
            self.focus = Focus::Tree;
            self.start_create(want);
        } else if hit(self.rect_tree, x, y) {
            if self.focus == Focus::Editor {
                self.flush();
            }
            self.focus = Focus::Tree;
            let row = self.tree_scroll + (y - self.rect_tree.y) as usize;
            if row < self.visible.len() {
                self.tree_drag = Some((row, row));
                self.sel = row;
                let idx = self.visible[row];
                match self.project.nodes[idx].kind {
                    Kind::Container | Kind::Category => {
                        let open = self.project.nodes[idx].expanded;
                        self.project.nodes[idx].expanded = !open;
                        self.refresh_visible();
                    }
                    Kind::Scene => self.open_scene(idx),
                }
            }
        } else if hit(self.rect_codex, x, y) && self.codex.is_some() {
            self.flush();
            self.focus = Focus::Codex;
            // A click on an "appears in" row opens that scene.
            let list_top = self.rect_codex.y
                + self
                    .rect_codex
                    .height
                    .saturating_sub(self.codex.as_ref().map_or(0, |p| p.appears.len() as u16));
            if y >= list_top
                && let Some(pane) = &mut self.codex
            {
                pane.sel = (y - list_top) as usize;
                self.on_codex_key(Key::Enter);
            }
        } else if hit(self.rect_beside, x, y) && self.beside.is_some() {
            self.flush();
            self.focus = Focus::Beside;
        } else if hit(self.rect_prose, x, y) && self.open.is_some() {
            // Anywhere in the pane counts; the margins either side of the
            // column land at the nearest end of the line.
            self.focus = Focus::Editor;
            let r = self.rect_editor;
            let cx = x.clamp(r.x, r.x + r.width.saturating_sub(1));
            let cy = y.clamp(r.y, r.y + r.height.saturating_sub(1));
            let rows = self.editor.layout(self.edit_width);
            let vis = self.editor.scroll + (cy - r.y) as usize;
            self.editor.click(&rows, vis, (cx - r.x) as usize);
            // A misspelt word under the click offers its fixes right there.
            self.offer_spelling_at_caret();
        } else if hit(self.rect_scene, x, y) {
            if self.focus == Focus::Editor {
                self.flush();
            }
            self.focus = Focus::Clearing;
            self.pomo.toggle();
        } else if hit(self.rect_music, x, y) {
            if self.focus == Focus::Editor {
                self.flush();
            }
            self.focus = Focus::Music;
            self.music.send(music::Cmd::PlayPause);
        }
    }

    /// A right click in the prose: the same as a left click there, which
    /// offers spelling fixes when it lands on a misspelt word.
    pub fn on_right_click(&mut self, x: u16, y: u16) {
        if matches!(self.overlay, Overlay::Spelling { .. }) {
            self.overlay = Overlay::None;
        }
        if hit(self.rect_prose, x, y) && self.open.is_some() {
            self.on_click(x, y);
        }
    }

    /// Extend the editor selection while the left button is held — or, in the
    /// tree, track where a dragged row would land.
    pub fn on_drag(&mut self, x: u16, y: u16) {
        if let Some((from, _)) = self.tree_drag {
            let r = self.rect_tree;
            let cy = y.clamp(r.y, r.y + r.height.saturating_sub(1));
            let row =
                (self.tree_scroll + (cy - r.y) as usize).min(self.visible.len().saturating_sub(1));
            self.tree_drag = Some((from, row));
            return;
        }
        if self.focus != Focus::Editor || self.open.is_none() {
            return;
        }
        let r = self.rect_editor;
        if r.width == 0 || r.height == 0 {
            return;
        }
        // Clamp to the pane so dragging past an edge keeps extending.
        let cx = x.clamp(r.x, r.x + r.width.saturating_sub(1));
        let cy = y.clamp(r.y, r.y + r.height.saturating_sub(1));
        let rows = self.editor.layout(self.edit_width);
        let vis = self.editor.scroll + (cy - r.y) as usize;
        self.editor.drag(&rows, vis, (cx - r.x) as usize);
        // Dragging from a misspelt word is selecting, not asking for fixes.
        if self.editor.has_selection()
            && matches!(self.overlay, Overlay::Spelling { inline: true, .. })
        {
            self.overlay = Overlay::None;
        }
    }

    /// Copy the editor selection to the system clipboard.
    pub fn copy_selection(&mut self) {
        match self.editor.selected_text() {
            Some(text) if !text.is_empty() => {
                let n = text.chars().count();
                if copy_to_clipboard(&text) {
                    self.msg = format!("copied {n} char{}", if n == 1 { "" } else { "s" });
                } else {
                    self.msg = "no clipboard tool found".into();
                }
            }
            _ => self.msg = "nothing selected".into(),
        }
    }

    /// Wheel scrolls whichever pane is under the pointer, without stealing focus.
    pub fn on_scroll(&mut self, x: u16, y: u16, down: bool) {
        const STEP: usize = 3;
        if hit(self.rect_tree, x, y) {
            if down {
                self.tree_scroll =
                    (self.tree_scroll + STEP).min(self.visible.len().saturating_sub(1));
            } else {
                self.tree_scroll = self.tree_scroll.saturating_sub(STEP);
            }
        } else if hit(self.rect_prose, x, y) {
            let rows = self.editor.layout(self.edit_width);
            if down {
                self.editor.scroll = (self.editor.scroll + STEP).min(rows.len().saturating_sub(1));
            } else {
                self.editor.scroll = self.editor.scroll.saturating_sub(STEP);
            }
        } else if hit(self.rect_beside, x, y)
            && let Some(b) = &mut self.beside
        {
            b.scroll = if down {
                b.scroll + STEP
            } else {
                b.scroll.saturating_sub(STEP)
            };
        }
    }

    // ---- themes ------------------------------------------------------

    /// Preset names plus the custom slot, in picker order.
    pub fn theme_names(&self) -> Vec<String> {
        let mut v: Vec<String> = theme::presets().into_iter().map(|t| t.name).collect();
        v.push("Custom…".into());
        v
    }

    /// The menu, in the book's own words — it offers a new act if that is what
    /// this book calls its parts. Preferences live one level down, in Settings.
    pub fn menu(&self) -> Vec<(String, Action)> {
        let row = |label: &str, key: &str| format!("{label:<26}{key}");
        let m = self.mod_label();
        let on_off =
            |on: bool, what: &str| format!("Turn {what} {}", if on { "off" } else { "on" });
        let writing = self.open.is_some();
        let mut items = vec![
            (
                row("Find anything…", &format!("({m}K)")),
                Action::FindAnything,
            ),
            // Making and unmaking.
            (row("New scene…", "(n)"), Action::NewScene),
            (row("New chapter…", "(c)"), Action::NewChapter),
            (
                row(&format!("New {}…", self.project.meta.part_noun()), "(p)"),
                Action::NewPart,
            ),
            (row("New folder…", "(N)"), Action::NewFolder),
            (row("Rename…", "(r)"), Action::Rename),
            (row("Delete…", "(d)"), Action::Delete),
            // Writing and revising.
            (
                row(
                    if self.focus_mode {
                        "Leave focus mode"
                    } else {
                        "Focus mode"
                    },
                    &format!("({m}D)"),
                ),
                Action::FocusMode,
            ),
            (row("Open a scene beside…", "(v)"), Action::BesidePicker),
            (row("Notes & TKs…", &format!("({m}T)")), Action::NotesList),
            (row("Next scene still in draft", ""), Action::NextDraft),
            (on_off(self.echo_on, "echo words"), Action::EchoWords),
            if self.sprint.is_some() {
                ("Stop the sprint".into(), Action::EndSprint)
            } else {
                ("Start a sprint…".into(), Action::StartSprint)
            },
            (row("Music player…", "(F7)"), Action::MusicPlayer),
            // Word, EPUB and Markdown. The project map and a bare Markdown
            // compile are still in the palette.
            ("Export…".into(), Action::Export),
            ("Settings…".into(), Action::Settings),
            (row("Close", "(Esc)"), Action::CloseMenu),
            (row("Quit Grimoire", &format!("({m}Q)")), Action::Quit),
        ];
        // Rows that couldn't do anything right now aren't offered: nothing to
        // play with music off (Settings is where it comes back on), and the
        // writing tools want a scene open.
        items.retain(|(_, a)| match a {
            Action::MusicPlayer => self.music.enabled,
            Action::FocusMode => writing || self.focus_mode,
            Action::BesidePicker => writing,
            Action::EchoWords => writing || self.echo_on,
            _ => true,
        });
        items
    }

    /// How Grimoire looks and sounds: the menu's Settings, nested.
    pub fn settings_menu(&self) -> Vec<(String, Action)> {
        let on_off =
            |on: bool, what: &str| format!("Turn {what} {}", if on { "off" } else { "on" });
        vec![
            ("Themes…".into(), Action::Themes),
            ("Music source…".into(), Action::MusicSource),
            (on_off(self.music.enabled, "music"), Action::MusicToggle),
            (on_off(self.spell_on, "spellcheck"), Action::Spellcheck),
            (on_off(self.icons_on, "tree icons"), Action::Icons),
            (
                match self.line_width {
                    0 => "Line width: the whole pane".to_string(),
                    w => format!("Line width: {w} columns"),
                },
                Action::LineWidth,
            ),
            (
                on_off(self.typewriter, "typewriter scrolling"),
                Action::Typewriter,
            ),
            ("Back".into(), Action::MenuBack),
        ]
    }

    pub fn open_menu(&mut self) {
        self.overlay = Overlay::Menu { sel: 0 };
    }

    /// Esc with nothing on top: the menu, from any pane. What's in the way
    /// goes first — a selection in the editor, or an open note, whichever pane
    /// has focus — so a second Esc opens it. Closing the menu leaves you in
    /// the pane you were in.
    pub fn escape(&mut self) {
        if self.focus == Focus::Editor && self.editor.has_selection() {
            self.editor.clear_selection();
        } else if self.codex.is_some() {
            self.close_codex();
        } else if self.beside.is_some() {
            self.close_beside();
        } else {
            self.open_menu();
        }
    }

    // ---- focus mode, and a scene beside --------------------------------

    /// Ctrl-D: just the prose, centred, and a quiet status line — or back.
    pub fn toggle_focus_mode(&mut self) {
        if self.focus_mode {
            self.focus_mode = false;
            self.msg = "focus mode off".into();
            return;
        }
        if self.open.is_none() {
            self.msg = "open a scene first — focus mode is for writing one".into();
            return;
        }
        self.focus_mode = true;
        if !matches!(self.focus, Focus::Editor | Focus::Codex | Focus::Beside) {
            self.focus = Focus::Editor;
        }
        // Short, so the hint beside it (which names the way out) fits.
        self.msg = "focus mode".into();
    }

    /// Nothing left to write in (the scene was closed or deleted): focus
    /// mode steps aside rather than leave a blank screen with no tree.
    pub fn check_focus_mode(&mut self) {
        if self.focus_mode && self.open.is_none() {
            self.focus_mode = false;
            self.focus = Focus::Tree;
        }
    }

    /// Show the scene selected in the tree beside the one being written,
    /// read-only. `v` again on the same scene puts it away.
    pub fn open_beside(&mut self) {
        if let Some(&i) = self.visible.get(self.sel) {
            self.show_beside(i);
        }
    }

    /// The palette, narrowed to scenes: the chosen one opens beside.
    pub fn open_beside_picker(&mut self) {
        self.flush();
        let here = self.open.map(|i| self.project.nodes[i].path.clone());
        let entries: Vec<palette::Entry> = palette::entries(self)
            .into_iter()
            .filter_map(|mut e| match e.action {
                Action::Open(p) if Some(&p) != here.as_ref() => {
                    e.action = Action::Beside(p);
                    Some(e)
                }
                _ => None,
            })
            .collect();
        self.overlay = Overlay::Palette {
            query: String::new(),
            sel: 0,
            entries,
        };
    }

    fn show_beside(&mut self, i: usize) {
        let node = &self.project.nodes[i];
        if node.kind != Kind::Scene {
            self.msg = "pick a scene to show beside this one".into();
            return;
        }
        if self.open == Some(i) {
            self.msg = "that's the scene you're writing — pick another to show beside it".into();
            return;
        }
        if self.beside.as_ref().is_some_and(|b| b.path == node.path) {
            self.close_beside();
            return;
        }
        if !self.side_room {
            self.msg = "widen the window to show a scene beside this one".into();
            return;
        }
        let path = node.path.clone();
        self.msg = format!("{} beside", node.title);
        self.close_codex();
        self.beside = Some(BesidePane { path, scroll: 0 });
    }

    pub fn close_beside(&mut self) {
        self.beside = None;
        if self.focus == Focus::Beside {
            self.focus = if self.open.is_some() {
                Focus::Editor
            } else {
                Focus::Tree
            };
        }
    }

    /// The scene shown beside, if it's still in the book.
    pub fn beside_node(&self) -> Option<usize> {
        let b = self.beside.as_ref()?;
        self.project.nodes.iter().position(|n| n.path == b.path)
    }

    pub fn on_beside_key(&mut self, key: Key) {
        let Some(pane) = &mut self.beside else {
            self.focus = Focus::Editor;
            return;
        };
        let page = (self.rect_beside.height as usize).saturating_sub(2).max(1);
        match key {
            Key::Down | Key::Char('j') => pane.scroll += 1,
            Key::Up | Key::Char('k') => pane.scroll = pane.scroll.saturating_sub(1),
            Key::PageDown | Key::Char(' ') => pane.scroll += page,
            Key::PageUp => pane.scroll = pane.scroll.saturating_sub(page),
            Key::Home => pane.scroll = 0,
            Key::End => pane.scroll = usize::MAX / 2,
            // Swap: write in the one beside.
            Key::Enter | Key::Char('o') => {
                if let Some(i) = self.beside_node() {
                    self.close_beside();
                    self.reveal(i);
                    self.open_scene(i);
                    self.focus = Focus::Editor;
                }
            }
            Key::Char('q') => self.close_beside(),
            _ => {}
        }
    }

    /// Cycle the prose measure through a few common widths, and remember it.
    pub fn cycle_line_width(&mut self) {
        const WIDTHS: [usize; 5] = [60, 72, 80, 100, 0];
        let next = WIDTHS
            .iter()
            .position(|&w| w == self.line_width)
            .map_or(grimoire_core::settings::LINE_WIDTH, |i| {
                WIDTHS[(i + 1) % WIDTHS.len()]
            });
        self.line_width = next;
        Self::save_setting(|s| s.line_width = next);
        self.msg = if next == 0 {
            "lines fill the pane".into()
        } else {
            format!("lines up to {next} columns")
        };
    }

    pub fn toggle_typewriter(&mut self) {
        self.typewriter = !self.typewriter;
        let on = self.typewriter;
        Self::save_setting(|s| s.typewriter = on);
        self.msg = if on {
            "typewriter scrolling on (in focus mode)".into()
        } else {
            "typewriter scrolling off".into()
        };
    }

    /// Scroll the editor to follow the cursor — but only when it has moved,
    /// so the wheel can still look elsewhere. Focus mode with typewriter
    /// scrolling keeps the line mid-screen; otherwise a few rows stay free
    /// below it.
    pub fn follow_caret(&mut self, rows: &[grimoire_core::editor::VisRow]) {
        let h = self.edit_height;
        let key = (
            self.editor.cy,
            self.editor.cx,
            self.editor.lines.len(),
            self.edit_width,
            h,
            self.focus_mode,
        );
        let moved = self.last_caret != Some(key);
        self.last_caret = Some(key);
        if !moved {
            self.editor.clamp_scroll(rows, h);
        } else if self.focus_mode && self.typewriter {
            self.editor.centre_on_cursor(rows, h);
        } else {
            self.editor.keep_room_below(rows, h, 3.min(h / 4));
        }
    }

    fn close_codex(&mut self) {
        self.codex = None;
        if self.focus == Focus::Codex {
            self.focus = if self.open.is_some() {
                Focus::Editor
            } else {
                Focus::Tree
            };
        }
    }

    pub fn open_player(&mut self) {
        self.overlay = Overlay::Player {
            tab: Tab::Queue,
            sel: 0,
            follow: true,
            query: String::new(),
            find: String::new(),
            typing: false,
        };
        self.music.note = None;
        self.music.send(music::Cmd::FetchQueue);
        self.player_fetched = Some(std::time::Instant::now());
    }

    /// While the player is open, keep the queue fresh and the selection on
    /// the playing track (until you move it yourself).
    pub fn tick_player(&mut self) {
        let Overlay::Player {
            tab, sel, follow, ..
        } = &mut self.overlay
        else {
            return;
        };
        if *tab == Tab::Queue
            && *follow
            && let Some(i) = self.music.queue.iter().position(|it| it.current)
        {
            *sel = i;
        }
        if self
            .player_fetched
            .is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(4))
        {
            self.music.send(music::Cmd::FetchQueue);
            self.player_fetched = Some(std::time::Instant::now());
        }
    }

    /// project.md: the map of the book, regenerated.
    fn write_project_map(&mut self) {
        self.flush();
        match manuscript::write_project_file(&self.project) {
            Ok(p) => {
                self.msg = format!(
                    "project map written to {}",
                    p.file_name().unwrap_or_default().to_string_lossy()
                )
            }
            Err(e) => self.msg = format!("could not write project.md: {e}"),
        }
    }

    /// The whole manuscript as one Markdown file, in manuscript format.
    fn compile_markdown(&mut self) {
        self.flush();
        match manuscript::compile(&self.project) {
            Ok(c) => {
                let skipped = if c.skipped > 0 {
                    format!(", {} skipped", c.skipped)
                } else {
                    String::new()
                };
                self.msg = format!(
                    "compiled {} ch / {} scenes / {} words{skipped} -> {}",
                    c.chapters,
                    c.scenes,
                    c.words,
                    c.path.file_name().unwrap_or_default().to_string_lossy()
                );
            }
            Err(e) => self.msg = format!("compile failed: {e}"),
        }
    }

    /// Switch music on or off, remember it, and start or stop the poller.
    pub fn set_music(&mut self, on: bool) {
        let mut cfg = music::Config::load();
        cfg.enabled = on;
        if let Err(e) = cfg.save() {
            self.msg = format!("couldn't save the music setting: {e}");
            return;
        }
        self.music = Music::spawn(cfg);
        if !on && self.focus == Focus::Music {
            self.focus = Focus::Tree;
        }
        self.msg = if on {
            match self.music.state {
                music::State::NoToken => {
                    "music on — run grimoire music-setup to connect a player".into()
                }
                _ => format!("music on · {}", self.music.source.label()),
            }
        } else {
            "music off".into()
        };
    }

    pub fn open_theme_picker(&mut self) {
        let names = self.theme_names();
        let sel = names
            .iter()
            .position(|n| *n == self.theme.name)
            .unwrap_or(names.len() - 1);
        self.overlay = Overlay::Themes {
            sel,
            restore: self.theme.clone(),
        };
    }

    /// Show the theme at `i` immediately, so moving the cursor previews it.
    fn preview(&mut self, i: usize) {
        let presets = theme::presets();
        if i < presets.len() {
            self.theme = presets[i].clone();
        } else if self.theme.name != "Custom" {
            // Entering the custom slot starts from whatever you were just on:
            // its colours, and its Pomodoro world and Visualizer to go on with.
            let from = std::mem::replace(&mut self.theme.name, "Custom".into());
            self.theme.pick_world(from.clone());
            self.theme.pick_look(from);
        }
    }

    /// Status-bar hints for whichever pane has focus.
    pub fn hints(&self) -> String {
        let m = self.mod_label();
        // What Esc does comes first, so a narrow status bar never cuts it.
        let esc = if self.codex.is_some() {
            "Esc close note"
        } else if self.beside.is_some() {
            "Esc close beside"
        } else {
            "Esc menu"
        };
        // Focus mode needs a scene to show; where there is one, say how.
        let focus = if self.open.is_some() {
            format!("  {m}D focus mode")
        } else {
            String::new()
        };
        match self.focus {
            Focus::Tree => {
                let sel = self.visible.get(self.sel).copied();
                let keys: Vec<String> = create::offers(&self.project, sel)
                    .iter()
                    .map(|(k, w)| format!("{k} {w}"))
                    .collect();
                format!(
                    "{esc}{focus}  Tab pane  ↵ fold  {}  r rename  d delete  {m}Z undo  H history  {m}Q quit ",
                    keys.join("  ")
                )
            }
            Focus::Editor if self.focus_mode => {
                format!("{esc}  {m}D leave focus  {m}K find anything  {m}Z undo  {m}Q quit ")
            }
            Focus::Editor => {
                format!(
                    "{esc}{focus}  Tab pane  {m}K find anything  {m}Z undo  F8 spelling  {m}Q quit "
                )
            }
            Focus::Beside => "Esc close  Tab pane  ↑↓ PgDn scroll  ↵ write in this one ".into(),
            Focus::Codex => {
                "Esc close  Tab pane  ↑↓ scenes  ↵ go there  o open the note  PgDn scroll ".into()
            }
            Focus::Clearing => {
                format!("{esc}{focus}  Tab pane  ←→ view  ↵ start/pause  r reset  {m}Q quit ")
            }
            Focus::Music => {
                format!(
                    "{esc}  [ ] track  space pause  ←→ seek  r repeat  s shuffle  +/- volume  l like  ↵ player  {m}Q quit "
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Esc,
    Tab,
    BackTab,
    F(u8),
    Other,
}

/// "84 KB", "1.2 MB".
fn human_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1_048_576 => format!("{:.1} MB", b as f64 / 1_048_576.0),
        b if b >= 1024 => format!("{} KB", b / 1024),
        b => format!("{b} bytes"),
    }
}

/// Put the likeliest fix first: fewest edits (a swap counts as one), then the
/// same letters rearranged, then the same length, then the same first letter.
/// "Teh" offers "The" before "Ted" or "Eh".
/// How far a word may be accepted from here on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ignore {
    Now,
    Book,
    Everywhere,
}

/// The best few suggestions for a misspelt word, each in the word's own
/// capitalisation.
fn suggestions_for(speller: &spell::Speller, word: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in rank_suggestions(word, speller.suggest(word, 8), 8) {
        let s = spell::match_case(word, &s);
        if s != word && !out.contains(&s) {
            out.push(s);
        }
    }
    out.truncate(5);
    out
}

fn rank_suggestions(word: &str, mut found: Vec<String>, max: usize) -> Vec<String> {
    let w = word.to_lowercase();
    let len = w.chars().count() as isize;
    let first = w.chars().next();
    let letters = |s: &str| {
        let mut v: Vec<char> = s.chars().collect();
        v.sort_unstable();
        v
    };
    let mine = letters(&w);
    found.sort_by_key(|s| {
        let l = s.to_lowercase();
        (
            search::distance(&w, &l),
            // Same letters in a different order is the classic typo.
            letters(&l) != mine,
            (l.chars().count() as isize - len).unsigned_abs(),
            l.chars().next() != first,
        )
    });
    found.truncate(max);
    found
}

/// Today's starting word count, so the status line can show a session delta.
/// Stored in `.grimoire/progress.toml`, which belongs in .gitignore.
fn today_string() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// How words are counted, recorded beside the baseline. 2: `%% notes %%` and
/// TKs aren't words. A baseline written under an older rule is moved by what
/// the new rule takes out of the book as it stands, so upgrading mid-day
/// doesn't make "today" jump.
const COUNT_RULE: u32 = 2;

fn load_baseline(project: &Project) -> Result<usize> {
    let today = today_string();
    let dir = project.root.join(".grimoire");
    let path = dir.join("progress.toml");
    let total = project.total_words();

    if let Ok(s) = fs::read_to_string(&path) {
        let mut date = String::new();
        let mut baseline = total;
        let mut rule = 1;
        for line in s.lines() {
            if let Some(v) = line.strip_prefix("date = ") {
                date = v.trim().trim_matches('"').to_string();
            } else if let Some(v) = line.strip_prefix("baseline = ") {
                baseline = v.trim().parse().unwrap_or(total);
            } else if let Some(v) = line.strip_prefix("rule = ") {
                rule = v.trim().parse().unwrap_or(1);
            }
        }
        if date == today {
            if rule < COUNT_RULE {
                // Rule 1 counted every whitespace-separated word.
                let before: usize = project
                    .nodes
                    .iter()
                    .filter(|n| n.kind == Kind::Scene && n.in_manuscript)
                    .map(|n| n.body.split_whitespace().count())
                    .sum();
                baseline = (baseline + total).saturating_sub(before);
                write_baseline(&dir, &path, &today, baseline);
            }
            return Ok(baseline);
        }
    }

    write_baseline(&dir, &path, &today, total);
    Ok(total)
}

fn write_baseline(dir: &Path, path: &Path, today: &str, total: usize) {
    let _ = fs::create_dir_all(dir);
    let _ = grimoire_core::atomic::write_text(
        path,
        &format!("date = \"{today}\"\nbaseline = {total}\nrule = {COUNT_RULE}\n"),
    );
}

/// Hand text to whatever clipboard tool this machine has.
fn copy_to_clipboard(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    const TOOLS: &[(&str, &[&str])] = &[
        ("pbcopy", &[]),                         // macOS
        ("wl-copy", &[]),                        // Wayland
        ("xclip", &["-selection", "clipboard"]), // X11
        ("xsel", &["--clipboard", "--input"]),   // X11 alternative
        // Windows. clip.exe mangles anything outside the console code page —
        // every em dash and curly quote in a manuscript — so PowerShell reads
        // stdin as UTF-8 and sets the clipboard itself.
        (
            "powershell",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::InputEncoding = [Text.Encoding]::UTF8; Set-Clipboard -Value ([Console]::In.ReadToEnd())",
            ],
        ),
    ];

    for (cmd, args) in TOOLS {
        let Ok(mut child) = Command::new(cmd)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        return child.wait().map(|s| s.success()).unwrap_or(false);
    }
    false
}

#[cfg(all(test, windows))]
mod clipboard_tests {
    /// Uses the real clipboard, so it only runs when asked for:
    /// `cargo test -- --ignored clipboard`. The Windows CI job asks.
    #[test]
    #[ignore]
    fn clipboard_keeps_a_manuscript_line_intact() {
        let line = "\u{201c}Not yet,\u{201d} she said \u{2014} and meant it. Caf\u{e9}.";
        assert!(super::copy_to_clipboard(line));
        let out = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::OutputEncoding = [Text.Encoding]::UTF8; Get-Clipboard -Raw",
            ])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), line);
    }
}

#[cfg(test)]
mod today_tests {
    use super::*;

    /// A book whose one scene holds 11 words of prose and a 2-word note:
    /// 15 words the old way, counting the note and its fences.
    fn book(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grimoire-today-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let scene = d.join("manuscript/01-Chapter-One/01-Scene.md");
        fs::create_dir_all(scene.parent().unwrap()).unwrap();
        fs::write(&scene, "a b c d e f g h i j %% x y %% k\n").unwrap();
        d
    }

    #[test]
    fn a_new_counting_rule_mid_day_leaves_today_where_it_was() {
        let d = book("rule");
        let p = Project::load(&d).unwrap();
        assert_eq!(p.total_words(), 11);
        // This morning, counted the old way, the book stood at 12: today +3.
        fs::create_dir_all(d.join(".grimoire")).unwrap();
        fs::write(
            d.join(".grimoire/progress.toml"),
            format!("date = \"{}\"\nbaseline = 12\n", today_string()),
        )
        .unwrap();
        let baseline = load_baseline(&p).unwrap();
        assert_eq!(p.total_words() as i64 - baseline as i64, 3);
        // Written back under the new rule, so it isn't moved twice.
        assert_eq!(load_baseline(&p).unwrap(), baseline);
        let _ = fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod spelling_tests {
    #[test]
    fn a_swapped_letter_beats_a_dropped_one() {
        let ranked = super::rank_suggestions(
            "Teh",
            vec!["Tet".into(), "Ted".into(), "Eh".into(), "The".into()],
            3,
        );
        assert_eq!(ranked[0], "The");
        assert_eq!(ranked.len(), 3);
    }
}
