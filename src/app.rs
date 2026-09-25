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
use grimoire_core::project::{self, Kind, Project};
use grimoire_core::recovery;
use grimoire_core::resume;
use grimoire_core::search;
use grimoire_core::sessions;
use grimoire_core::settings::Settings;
use grimoire_core::spell;

mod aids;
pub use aids::{MarkRow, Sprint, filter_marks as aids_filter};

/// Save a couple of seconds after typing stops…
const AUTOSAVE_IDLE: Duration = Duration::from_secs(2);
/// …and never let unsaved words sit longer than this, however fast you type.
const AUTOSAVE_MAX: Duration = Duration::from_secs(20);
/// After a failed save, try again this often rather than on every tick.
const RETRY: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq)]
pub enum SaveState {
    Clean,
    Saved(Instant),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Editor,
    /// The note open beside the scene.
    Codex,
    Clearing,
    Music,
}

/// What a row of the menu (or of Settings, inside it) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Find,
    NewScene,
    NewChapter,
    NewPart,
    NewFolder,
    Rename,
    Delete,
    ProjectMap,
    Compile,
    Export,
    Player,
    Settings,
    Close,
    Themes,
    MusicSource,
    Music,
    Spellcheck,
    Icons,
    Back,
    Quit,
}

/// Something done to the book's files from the tree, kept so it can be taken
/// back.
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
    /// The garden's last step and when it last grew, so new growth can glint.
    pub growth_step: Option<usize>,
    pub growth_changed: Option<std::time::Instant>,
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
    /// Spellcheck underlines are showing.
    pub spell_on: bool,
    /// The tree shows a symbol beside each row.
    pub icons_on: bool,
    /// The dictionary, once it has loaded in the background.
    pub speller: Option<spell::Speller>,
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodexPane {
    pub entry: codex::Entry,
    pub appears: Vec<codex::Appearance>,
    pub sel: usize,
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

    pub fn new(mut project: Project) -> Result<Self> {
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
            pane_mode: Mode::Clearing,
            music: Music::spawn(music::Config::load()),
            viz: Visualizer::new(),
            growth_step: None,
            growth_changed: None,
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
            theme: theme::load(),
            overlay: Overlay::None,
            last_edit: None,
            unsaved_since: None,
            save_state: SaveState::Clean,
            last_attempt: None,
            pre_snapshotted: HashSet::new(),
            undo_stash: HashMap::new(),
            tree_undo: Vec::new(),
            tree_redo: Vec::new(),
            spell_on: Settings::load().spellcheck,
            icons_on: Settings::load().icons,
            speller: None,
            speller_rx: None,
            tree_drag: None,
            screen: (120, 40),
            codex_index: Vec::new(),
            codex: None,
            rect_codex: Rect::default(),
            last_resume: None,
            sessions_on: false,
            backup_rx: None,
            backup_note: None,
            echo_on: false,
            sprint: None,
        })
        .map(|mut app: App| {
            app.load_speller();
            app.rebuild_codex();
            app.sessions_on = sessions::git_available() && sessions::is_enabled(&app.project.root);
            if app.sessions_on {
                app.back_up();
            }
            let items = recovery::pending(&app.project);
            if !items.is_empty() {
                app.overlay = Overlay::Recover { items };
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
            }
        }
    }

    /// A scene's text changed in memory; autosave will pick it up.
    fn mark_changed(&mut self, i: usize) {
        self.project.nodes[i].dirty = true;
        let now = Instant::now();
        self.last_edit = Some(now);
        self.unsaved_since.get_or_insert(now);
    }

    fn open_scene(&mut self, idx: usize) {
        self.flush();
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

    pub fn autosave_tick(&mut self) {
        if self.project.dirty_count() == 0 {
            self.unsaved_since = None;
            return;
        }
        let idle = self.last_edit.is_none_or(|t| t.elapsed() >= AUTOSAVE_IDLE);
        let overdue = self
            .unsaved_since
            .is_some_and(|t| t.elapsed() >= AUTOSAVE_MAX);
        let may_retry = match self.save_state {
            SaveState::Failed(_) => self.last_attempt.is_none_or(|t| t.elapsed() >= RETRY),
            _ => true,
        };
        if (idle || overdue) && may_retry {
            self.commit_saves();
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
        if self.focus != Focus::Editor || self.open.is_none() {
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
        if self.focus != Focus::Editor || self.open.is_none() {
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
            Action::ProjectMap => self.run_menu(MenuItem::ProjectMap),
            Action::Compile => self.run_menu(MenuItem::Compile),
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
            Action::MusicSource => self.run_menu(MenuItem::MusicSource),
            Action::PlayPause => self.on_function_key(5),
            Action::NextTrack => self.on_function_key(6),
            Action::PrevTrack => self.on_function_key(4),
            Action::Timer => self.on_function_key(2),
            Action::TimerReset => self.on_function_key(3),
            Action::Menu => self.open_menu(),
            Action::Quit => self.quit = true,
            Action::Open(path) => {
                if let Some(i) = self.project.nodes.iter().position(|n| n.path == path) {
                    self.reveal(i);
                    self.open_scene(i);
                }
            }
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
                search::matches(line, query)
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
        let is_match = self
            .editor
            .selected_text()
            .is_some_and(|t| search::matches(&t, query) == vec![(0, t.chars().count())]);
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
                let (text, n) = search::replace_all(&self.editor.text(), &query, &with);
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

    pub fn open_find_book(&mut self, query: String) {
        self.flush();
        let hits = search::book(&self.project, &self.parents, &query);
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
        let root = self.project.root.clone();
        let mut scenes = 0;
        let mut total = 0;
        for i in 0..self.project.nodes.len() {
            if self.project.nodes[i].kind != Kind::Scene || self.project.in_trash(i) {
                continue;
            }
            let (text, n) = search::replace_all(&self.project.nodes[i].body, query, with);
            if n == 0 {
                continue;
            }
            // Every scene's version from before goes into its history.
            let path = self.project.nodes[i].path.clone();
            let _ = history::snapshot(&root, &path, &self.project.nodes[i].file_text(), None);
            if self.open == Some(i) {
                self.editor.set_text(&text);
            }
            self.project.nodes[i].body = text;
            self.mark_changed(i);
            scenes += 1;
            total += n;
        }
        self.commit_saves();
        self.msg = format!(
            "replaced {total} in {scenes} scene{} — each one's previous version is in its history (H)",
            if scenes == 1 { "" } else { "s" }
        );
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
        let mut words = spell::names_from_titles(&self.note_titles());
        words.extend(spell::book_words(&self.project.root));
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut s = spell::Speller::new();
            s.add_words(words);
            let _ = tx.send(s);
        });
        self.speller_rx = Some(rx);
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

    pub fn toggle_icons(&mut self) {
        self.icons_on = !self.icons_on;
        let _ = Settings {
            spellcheck: self.spell_on,
            icons: self.icons_on,
        }
        .save();
        self.msg = if self.icons_on {
            "tree icons on".into()
        } else {
            "tree icons off".into()
        };
    }

    pub fn toggle_spellcheck(&mut self) {
        self.spell_on = !self.spell_on;
        let _ = Settings {
            spellcheck: self.spell_on,
            icons: self.icons_on,
        }
        .save();
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
        let suggestions = rank_suggestions(&word, speller.suggest(&word, 8), 6);
        self.editor.select((line, start), (line, end));
        self.overlay = Overlay::Spelling {
            line,
            start,
            end,
            word,
            suggestions,
            sel: 0,
        };
    }

    fn apply_spelling(&mut self, line: usize, start: usize, end: usize, with: &str) {
        self.editor.select((line, start), (line, end));
        self.editor.delete_selection();
        self.editor.insert_str(with);
        self.flush();
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
            if n.kind != Kind::Scene || !n.in_manuscript || self.project.in_trash(i) {
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
        if open_path.is_some_and(|p| p.starts_with(&path)) {
            self.open = None;
            self.editor = Editor::from_text("");
            self.focus = Focus::Tree;
        }
        let done = if permanent {
            project::destroy(&path).map(|_| String::new())
        } else {
            let m = self.mod_label();
            project::trash(&root, &path).map(|to| {
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
        if let Some(i) = self.open
            && let Some(p) = moved(&self.project.nodes[i].path)
        {
            self.project.nodes[i].path = p;
        }
    }

    /// Re-read the tree from disk, keeping what's folded, which scene is open,
    /// and the editor exactly as it is.
    fn reload_tree(&mut self) -> Result<()> {
        let collapsed: Vec<(PathBuf, bool)> = self
            .project
            .nodes
            .iter()
            .filter(|n| n.kind != Kind::Scene)
            .map(|n| (n.path.clone(), n.expanded))
            .collect();
        let open_path = self.open.map(|i| self.project.nodes[i].path.clone());
        let root = self.project.root.clone();
        self.project = Project::load(&root)?;
        for n in &mut self.project.nodes {
            if let Some(&(_, open)) = collapsed.iter().find(|(p, _)| *p == n.path) {
                n.expanded = open;
            }
        }
        self.parents = self.project.parents();
        self.open = open_path.and_then(|p| self.project.nodes.iter().position(|n| n.path == p));
        self.refresh_visible();
        self.refresh_names();
        Ok(())
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

    fn cork_key(&mut self, key: Key) {
        let cols = self.cork_cols();
        let Overlay::Cork {
            scope,
            sel,
            pov,
            typing,
        } = &mut self.overlay
        else {
            return;
        };
        let scope_idx = scope
            .as_ref()
            .and_then(|p| self.project.nodes.iter().position(|n| &n.path == p));
        let groups = cork::board(&self.project, scope_idx);
        let cards: Vec<cork::Card> = groups.iter().flat_map(|g| g.cards.clone()).collect();
        if cards.is_empty() {
            self.overlay = Overlay::None;
            return;
        }
        *sel = (*sel).min(cards.len() - 1);
        let card = cards[*sel].clone();

        if let Some((field, buf)) = typing {
            match key {
                Key::Char(c) if !c.is_control() && buf.chars().count() < 200 => buf.push(c),
                Key::Backspace => {
                    buf.pop();
                }
                Key::Enter => {
                    let (field, value) = (*field, buf.clone());
                    *typing = None;
                    let key = match field {
                        CardField::Synopsis => "synopsis",
                        CardField::Pov => "pov",
                    };
                    self.project.nodes[card.idx].set_meta(key, &value);
                    self.mark_changed(card.idx);
                    self.msg = format!("{} · {key} saved", card.title);
                }
                Key::Esc => *typing = None,
                _ => {}
            }
            return;
        }

        match key {
            Key::Left | Key::Char('h') => *sel = cork::step(&groups, cols, *sel, -1, 0),
            Key::Right | Key::Char('l') => *sel = cork::step(&groups, cols, *sel, 1, 0),
            Key::Up | Key::Char('k') => *sel = cork::step(&groups, cols, *sel, 0, -1),
            Key::Down | Key::Char('j') => *sel = cork::step(&groups, cols, *sel, 0, 1),
            Key::Char('[') | Key::Char(']') => {
                let parts = cork::parts(&self.project);
                if let Some(at) = scope_idx.and_then(|s| parts.iter().position(|&p| p == s)) {
                    let next = if key == Key::Char('[') {
                        at.checked_sub(1)
                    } else {
                        (at + 1 < parts.len()).then_some(at + 1)
                    };
                    if let Some(n) = next {
                        *scope = Some(self.project.nodes[parts[n]].path.clone());
                        *sel = 0;
                    }
                }
            }
            Key::Char('p') => {
                let all = cork::povs(&groups);
                *pov = match pov
                    .as_ref()
                    .and_then(|cur| all.iter().position(|x| x == cur))
                {
                    None if !all.is_empty() && pov.is_none() => Some(all[0].clone()),
                    Some(i) if i + 1 < all.len() => Some(all[i + 1].clone()),
                    _ => None,
                };
            }
            Key::Char('s') => {
                let next = cork::next_status(card.status.as_deref());
                self.project.nodes[card.idx].set_meta("status", next);
                self.mark_changed(card.idx);
            }
            Key::Char('e') => {
                *typing = Some((
                    CardField::Synopsis,
                    card.synopsis.clone().unwrap_or_default(),
                ))
            }
            Key::Char('v') => {
                *typing = Some((CardField::Pov, card.pov.clone().unwrap_or_default()))
            }
            Key::Enter => {
                self.overlay = Overlay::None;
                self.reveal(card.idx);
                self.open_scene(card.idx);
            }
            Key::Esc | Key::Char('b') => self.overlay = Overlay::None,
            _ => {}
        }
    }

    // ---- undoing what the tree did ---------------------------------------

    fn record(&mut self, step: TreeStep) {
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
                            let t = batch[0].1.clone();
                            (Ok(()), Some(t), None)
                        }
                        Err(e) => (Err(e), trashed, None),
                    }
                } else {
                    match &trashed {
                        Some(t) => {
                            let batch = vec![(t.clone(), path.clone())];
                            (
                                project::apply_moves(&root, &batch, false),
                                None,
                                Some(path.clone()),
                            )
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
        // Rewritten links may have changed the open scene on disk.
        if let Some(i) = self.open
            && self.editor.text() != self.project.nodes[i].body
        {
            let body = self.project.nodes[i].body.clone();
            self.editor.set_text(&body);
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
            Focus::Tree => true,
            Focus::Editor => self.open.is_some(),
            Focus::Codex => self.codex.is_some(),
            Focus::Clearing => self.scene_visible,
            Focus::Music => self.music_visible,
        }
    }

    /// Tab through every pane that is actually on screen.
    pub fn cycle_focus(&mut self, forward: bool) {
        const ORDER: [Focus; 5] = [
            Focus::Tree,
            Focus::Editor,
            Focus::Codex,
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

    pub fn on_music_key(&mut self, key: Key) {
        match key {
            Key::Enter => self.open_player(),
            Key::Char(' ') => self.music.send(music::Cmd::PlayPause),
            Key::Right | Key::Char('l') | Key::Char('n') => self.music.send(music::Cmd::Next),
            Key::Left | Key::Char('h') | Key::Char('p') => self.music.send(music::Cmd::Prev),
            _ => {}
        }
    }

    /// A left click focuses the pane under the pointer and acts on it.
    pub fn on_click(&mut self, x: u16, y: u16) {
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
        } else if hit(self.rect_editor, x, y) && self.open.is_some() {
            self.focus = Focus::Editor;
            let rows = self.editor.layout(self.edit_width);
            let vis = self.editor.scroll + (y - self.rect_editor.y) as usize;
            self.editor
                .click(&rows, vis, (x - self.rect_editor.x) as usize);
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
        } else if hit(self.rect_editor, x, y) {
            let rows = self.editor.layout(self.edit_width);
            if down {
                self.editor.scroll = (self.editor.scroll + STEP).min(rows.len().saturating_sub(1));
            } else {
                self.editor.scroll = self.editor.scroll.saturating_sub(STEP);
            }
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
    pub fn menu(&self) -> Vec<(String, MenuItem)> {
        let row = |label: String, key: &str| format!("{label:<20}{key}");
        let m = self.mod_label();
        let mut items = vec![
            (
                row("Find anything…".into(), &format!("({m}K)")),
                MenuItem::Find,
            ),
            (row("New scene…".into(), "(n)"), MenuItem::NewScene),
            (row("New chapter…".into(), "(c)"), MenuItem::NewChapter),
            (
                row(format!("New {}…", self.project.meta.part_noun()), "(p)"),
                MenuItem::NewPart,
            ),
            (row("New folder…".into(), "(N)"), MenuItem::NewFolder),
            (row("Rename…".into(), "(r)"), MenuItem::Rename),
            (row("Delete…".into(), "(d)"), MenuItem::Delete),
            // Word, EPUB and Markdown. The project map and a bare Markdown
            // compile are still in the palette.
            ("Export…".into(), MenuItem::Export),
            (row("Music player…".into(), "(F7)"), MenuItem::Player),
            ("Settings…".into(), MenuItem::Settings),
            (row("Close".into(), "(Esc)"), MenuItem::Close),
            (
                row("Quit Grimoire".into(), &format!("({m}Q)")),
                MenuItem::Quit,
            ),
        ];
        // Nothing to play with music off; Settings is where it comes back on.
        if !self.music.enabled {
            items.retain(|(_, i)| *i != MenuItem::Player);
        }
        items
    }

    /// How Grimoire looks and sounds: the menu's Settings, nested.
    pub fn settings_menu(&self) -> Vec<(String, MenuItem)> {
        let on_off =
            |on: bool, what: &str| format!("Turn {what} {}", if on { "off" } else { "on" });
        vec![
            ("Themes…".into(), MenuItem::Themes),
            ("Music source…".into(), MenuItem::MusicSource),
            (on_off(self.music.enabled, "music"), MenuItem::Music),
            (on_off(self.spell_on, "spellcheck"), MenuItem::Spellcheck),
            (on_off(self.icons_on, "tree icons"), MenuItem::Icons),
            ("Back".into(), MenuItem::Back),
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
        } else {
            self.open_menu();
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

    fn run_menu(&mut self, item: MenuItem) {
        match item {
            MenuItem::Find => self.open_palette(),
            MenuItem::NewScene => self.start_create(New::Scene),
            MenuItem::NewChapter => self.start_create(New::Chapter),
            MenuItem::NewPart => self.start_create(New::Part),
            MenuItem::NewFolder => self.start_create(New::Folder),
            MenuItem::Rename => self.start_rename(),
            MenuItem::Delete => self.start_delete(),
            MenuItem::ProjectMap => {
                self.flush_public();
                match manuscript::write_project_file(&self.project) {
                    Ok(p) => {
                        self.msg = format!(
                            "project map written to {}",
                            p.file_name().unwrap_or_default().to_string_lossy()
                        )
                    }
                    Err(e) => self.msg = format!("could not write project.md: {e}"),
                }
                self.overlay = Overlay::None;
            }
            MenuItem::Compile => {
                self.flush_public();
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
                self.overlay = Overlay::None;
            }
            MenuItem::Player if !self.music.enabled => {
                self.overlay = Overlay::None;
                self.msg = "music is off — turn it on first".into();
            }
            MenuItem::Player => self.open_player(),
            MenuItem::Settings => self.overlay = Overlay::Settings { sel: 0 },
            MenuItem::MusicSource => {
                let cur = self.music.source;
                let sel = music::Source::ALL
                    .iter()
                    .position(|s| *s == cur)
                    .unwrap_or(0);
                self.overlay = Overlay::Sources { sel };
            }
            // Toggles stay in Settings, so the change shows on its row.
            MenuItem::Music => self.set_music(!self.music.enabled),
            MenuItem::Spellcheck => self.toggle_spellcheck(),
            MenuItem::Icons => self.toggle_icons(),
            MenuItem::Themes => self.open_theme_picker(),
            MenuItem::Back => {
                let sel = self
                    .menu()
                    .iter()
                    .position(|(_, i)| *i == MenuItem::Settings)
                    .unwrap_or(0);
                self.overlay = Overlay::Menu { sel };
            }
            MenuItem::Export => self.open_export(),
            MenuItem::Close => self.overlay = Overlay::None,
            MenuItem::Quit => {
                self.overlay = Overlay::None;
                self.quit = true;
            }
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

    /// Push unsaved editor text into the tree so generated files see it.
    fn flush_public(&mut self) {
        self.flush();
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
            // Entering the custom slot starts from whatever you were just on.
            self.theme.name = "Custom".into();
        }
    }

    pub fn on_overlay_key(&mut self, key: Key) {
        let menu_items: Vec<MenuItem> = self.menu().into_iter().map(|(_, i)| i).collect();
        let settings_items: Vec<MenuItem> =
            self.settings_menu().into_iter().map(|(_, i)| i).collect();
        let nested = matches!(self.overlay, Overlay::Settings { .. });
        match &mut self.overlay {
            Overlay::None => {}

            Overlay::Marks { .. } => self.on_marks_key(key),
            Overlay::Sprint { .. } => self.on_sprint_key(key),

            Overlay::Palette {
                query,
                sel,
                entries,
            } => match key {
                Key::Char(c) if !c.is_control() => {
                    query.push(c);
                    *sel = 0;
                }
                Key::Backspace => {
                    query.pop();
                    *sel = 0;
                }
                Key::Down => *sel += 1,
                Key::Up => *sel = sel.saturating_sub(1),
                Key::PageDown => *sel += 10,
                Key::PageUp => *sel = sel.saturating_sub(10),
                Key::Enter => {
                    let hits = palette::filter(entries, query);
                    if let Some(e) = hits.get((*sel).min(hits.len().saturating_sub(1))) {
                        let action = e.action.clone();
                        self.run_action(action);
                    }
                }
                Key::Esc => self.overlay = Overlay::None,
                _ => {}
            },

            Overlay::Find {
                query,
                with,
                on_with,
                from,
            } => {
                let (q, from_pos) = (query.clone(), *from);
                match key {
                    Key::Char(c) if !c.is_control() => {
                        if *on_with {
                            with.get_or_insert_with(String::new).push(c);
                        } else {
                            query.push(c);
                            let q = query.clone();
                            self.find_step(&q, true, Some(from_pos));
                        }
                    }
                    Key::Backspace => {
                        if *on_with {
                            if let Some(w) = with {
                                w.pop();
                            }
                        } else {
                            query.pop();
                            let q = query.clone();
                            self.find_step(&q, true, Some(from_pos));
                        }
                    }
                    Key::Tab | Key::BackTab => {
                        if with.is_none() {
                            *with = Some(String::new());
                        }
                        *on_with = !*on_with;
                    }
                    Key::Enter if *on_with => {
                        let w = with.clone().unwrap_or_default();
                        self.replace_current(&q, &w);
                    }
                    Key::Enter | Key::Down => self.find_step(&q, true, None),
                    Key::Up => self.find_step(&q, false, None),
                    Key::Esc => {
                        self.overlay = Overlay::None;
                        self.focus = Focus::Editor;
                    }
                    _ => {}
                }
            }

            Overlay::FindBook {
                query,
                with,
                on_with,
                hits,
                sel,
                confirm,
            } => {
                if *confirm {
                    match key {
                        Key::Char('y') | Key::Char('Y') => {
                            let (q, w) = (query.clone(), with.clone().unwrap_or_default());
                            self.overlay = Overlay::None;
                            self.replace_in_book(&q, &w);
                        }
                        _ => *confirm = false,
                    }
                    return;
                }
                let mut changed = false;
                match key {
                    Key::Char(c) if !c.is_control() => {
                        if *on_with {
                            with.get_or_insert_with(String::new).push(c);
                        } else {
                            query.push(c);
                            changed = true;
                        }
                    }
                    Key::Backspace => {
                        if *on_with {
                            if let Some(w) = with {
                                w.pop();
                            }
                        } else {
                            query.pop();
                            changed = true;
                        }
                    }
                    Key::Tab | Key::BackTab => {
                        if with.is_none() {
                            *with = Some(String::new());
                        }
                        *on_with = !*on_with;
                    }
                    Key::Down => *sel = (*sel + 1).min(hits.len().saturating_sub(1)),
                    Key::Up => *sel = sel.saturating_sub(1),
                    Key::PageDown => *sel = (*sel + 10).min(hits.len().saturating_sub(1)),
                    Key::PageUp => *sel = sel.saturating_sub(10),
                    Key::Enter if *on_with => {
                        if !hits.is_empty() && with.is_some() {
                            *confirm = true;
                        }
                    }
                    Key::Enter => {
                        if let Some(h) = hits.get(*sel).cloned() {
                            self.go_to_hit(&h.path, h.line, h.start, h.end);
                        }
                    }
                    Key::Esc => self.overlay = Overlay::None,
                    _ => {}
                }
                if changed
                    && let Overlay::FindBook {
                        query, hits, sel, ..
                    } = &mut self.overlay
                {
                    *hits = search::book(&self.project, &self.parents, query);
                    *sel = 0;
                }
            }

            Overlay::Cork { .. } => self.cork_key(key),

            Overlay::SessionsOff => match key {
                Key::Char('y') | Key::Char('Y') => {
                    self.overlay = Overlay::None;
                    self.commit_saves();
                    self.save_resume(true);
                    match sessions::enable(&self.project.root) {
                        Ok(()) => {
                            self.sessions_on = true;
                            self.msg =
                                "session history is on — each session is kept when you quit".into();
                            self.open_sessions();
                        }
                        Err(e) => self.msg = format!("couldn't turn on session history: {e}"),
                    }
                }
                _ => self.overlay = Overlay::None,
            },

            Overlay::Sessions {
                list, sel, changes, ..
            } => {
                if let Some((which, items, csel)) = changes {
                    match key {
                        Key::Down | Key::Char('j') => {
                            *csel = (*csel + 1).min(items.len().saturating_sub(1))
                        }
                        Key::Up | Key::Char('k') => *csel = csel.saturating_sub(1),
                        Key::Enter => {
                            if let (Some(s), Some(c)) =
                                (list.get(*which).cloned(), items.get(*csel).cloned())
                            {
                                let root = self.project.root.clone();
                                let parent = format!("{}^", s.hash);
                                let old_path = match &c.kind {
                                    sessions::ChangeKind::Renamed { from } => from.clone(),
                                    _ => c.path.clone(),
                                };
                                let before = sessions::file_at(&root, &parent, &old_path)
                                    .ok()
                                    .flatten()
                                    .unwrap_or_default();
                                let after = sessions::file_at(&root, &s.hash, &c.path)
                                    .ok()
                                    .flatten()
                                    .unwrap_or_default();
                                let title = c
                                    .path
                                    .file_stem()
                                    .map(|x| x.to_string_lossy().to_string())
                                    .unwrap_or_default();
                                self.overlay = Overlay::SessionDiff {
                                    title,
                                    label: s.label.clone(),
                                    before: grimoire_core::project::split_frontmatter(&before).1,
                                    after: grimoire_core::project::split_frontmatter(&after).1,
                                    scroll: 0,
                                };
                            }
                        }
                        Key::Esc | Key::Left | Key::Char('h') => *changes = None,
                        _ => {}
                    }
                    return;
                }
                match key {
                    Key::Down | Key::Char('j') => {
                        *sel = (*sel + 1).min(list.len().saturating_sub(1))
                    }
                    Key::Up | Key::Char('k') => *sel = sel.saturating_sub(1),
                    Key::Enter | Key::Right | Key::Char('h') => {
                        if let Some(s) = list.get(*sel) {
                            match sessions::changes(&self.project.root, &s.hash) {
                                Ok(items) => *changes = Some((*sel, items, 0)),
                                Err(e) => self.msg = format!("couldn't read that session: {e}"),
                            }
                        }
                    }
                    Key::Char('s') => {
                        self.overlay = Overlay::None;
                        if let Some(label) = self.save_session() {
                            self.msg = format!("session saved: {label}");
                        }
                        self.open_sessions();
                    }
                    Key::Esc => self.overlay = Overlay::None,
                    _ => {}
                }
            }

            Overlay::SessionDiff { scroll, .. } => match key {
                Key::Down | Key::PageDown | Key::Char(' ') => *scroll += 5,
                Key::Up | Key::PageUp => *scroll = scroll.saturating_sub(5),
                _ => self.open_sessions_keeping_place(),
            },

            Overlay::Export {
                formats,
                parts,
                sel,
                done,
            } => {
                if done.is_some() {
                    self.overlay = Overlay::None;
                    return;
                }
                // Rows: three formats, each act, then the export button.
                let rows = 3 + parts.len() + 1;
                match key {
                    Key::Down | Key::Char('j') => *sel = (*sel + 1).min(rows - 1),
                    Key::Up | Key::Char('k') => *sel = sel.saturating_sub(1),
                    Key::Char(' ') | Key::Enter if *sel < 3 => formats[*sel] = !formats[*sel],
                    Key::Char(' ') | Key::Enter if *sel < 3 + parts.len() => {
                        let p = &mut parts[*sel - 3];
                        p.2 = !p.2;
                    }
                    Key::Enter | Key::Char('x') => {
                        if !formats.iter().any(|&f| f) {
                            self.msg = "choose at least one format".into();
                        } else if !parts.is_empty() && !parts.iter().any(|p| p.2) {
                            self.msg =
                                format!("choose at least one {}", self.project.meta.part_noun());
                        } else {
                            let (f, p) = (*formats, parts.clone());
                            let lines = self.run_export(f, &p);
                            if let Overlay::Export { done, .. } = &mut self.overlay {
                                *done = Some(lines);
                            }
                        }
                    }
                    Key::Esc => self.overlay = Overlay::None,
                    _ => {}
                }
            }

            Overlay::Spelling {
                line,
                start,
                end,
                word,
                suggestions,
                sel,
            } => {
                // Rows: each suggestion, then "add to this book", then "leave it".
                let rows = suggestions.len() + 2;
                match key {
                    Key::Down | Key::Char('j') => *sel = (*sel + 1).min(rows - 1),
                    Key::Up | Key::Char('k') => *sel = sel.saturating_sub(1),
                    Key::Char(c @ '1'..='9') if (c as usize - '1' as usize) < suggestions.len() => {
                        let (l, s, e, with) = (
                            *line,
                            *start,
                            *end,
                            suggestions[c as usize - '1' as usize].clone(),
                        );
                        self.overlay = Overlay::None;
                        self.apply_spelling(l, s, e, &with);
                    }
                    Key::Char('a') => {
                        let w = word.clone();
                        self.overlay = Overlay::None;
                        self.add_to_book(&w);
                    }
                    Key::Enter => {
                        let (l, s, e, i) = (*line, *start, *end, *sel);
                        let (word, pick) = (word.clone(), suggestions.get(i).cloned());
                        let n = suggestions.len();
                        self.overlay = Overlay::None;
                        match pick {
                            Some(with) => self.apply_spelling(l, s, e, &with),
                            None if i == n => self.add_to_book(&word),
                            None => {}
                        }
                    }
                    Key::F(8) => {
                        // Leave this one and go on to the next.
                        let (l, e) = (*line, *end);
                        self.overlay = Overlay::None;
                        self.editor.place(l, e);
                        self.spelling();
                    }
                    Key::Esc => self.overlay = Overlay::None,
                    _ => {}
                }
            }

            Overlay::Names { drifts, sel } => match key {
                Key::Down | Key::Char('j') => *sel = (*sel + 1).min(drifts.len().saturating_sub(1)),
                Key::Up | Key::Char('k') => *sel = sel.saturating_sub(1),
                Key::Enter => {
                    if let Some(h) = drifts.get(*sel).and_then(|d| d.hits.first()).cloned() {
                        self.go_to_hit(&h.path, h.line, h.start, h.end);
                    }
                }
                Key::Char('f') | Key::Char('F') => {
                    if let Some(d) = drifts.get(*sel).cloned() {
                        drifts.remove(*sel);
                        if *sel >= drifts.len() {
                            *sel = drifts.len().saturating_sub(1);
                        }
                        if drifts.is_empty() {
                            self.overlay = Overlay::None;
                        }
                        self.fix_name(&d.variant, &d.name.name);
                    }
                }
                Key::Esc => self.overlay = Overlay::None,
                _ => {}
            },

            Overlay::Recover { items } => match key {
                Key::Char('y') | Key::Char('Y') | Key::Enter => {
                    let items = std::mem::take(items);
                    self.overlay = Overlay::None;
                    let mut restored = 0;
                    for it in &items {
                        let Some(idx) = self.project.nodes.iter().position(|n| n.path == it.scene)
                        else {
                            continue;
                        };
                        let (front, body) = grimoire_core::project::split_frontmatter(&it.text);
                        self.project.nodes[idx].front = front;
                        if self.open == Some(idx) {
                            self.editor.set_text(&body);
                        }
                        self.project.nodes[idx].body = body;
                        self.mark_changed(idx);
                        restored += 1;
                    }
                    if self.commit_saves() {
                        self.msg = format!(
                            "restored {restored} scene{} — the saved version is in its history",
                            if restored == 1 { "" } else { "s" }
                        );
                    }
                }
                Key::Char('n') | Key::Char('N') => {
                    for it in items.iter() {
                        let _ = fs::remove_file(&it.file);
                    }
                    self.overlay = Overlay::None;
                    self.msg = "kept the saved versions".into();
                }
                Key::Esc => {
                    self.overlay = Overlay::None;
                    self.msg = "left for now — offered again next time".into();
                }
                _ => {}
            },

            Overlay::History {
                scene,
                versions,
                sel,
                scroll,
                ..
            } => match key {
                Key::Down | Key::Char('j') => {
                    *sel = (*sel + 1).min(versions.len().saturating_sub(1));
                    *scroll = 0;
                }
                Key::Up | Key::Char('k') => {
                    *sel = sel.saturating_sub(1);
                    *scroll = 0;
                }
                Key::PageDown | Key::Char(' ') => *scroll += 10,
                Key::PageUp => *scroll = scroll.saturating_sub(10),
                Key::Enter => {
                    let (scene, text) = (scene.clone(), versions[*sel].text.clone());
                    self.overlay = Overlay::None;
                    self.restore_version(&scene, &text);
                }
                Key::Esc => self.overlay = Overlay::None,
                _ => {}
            },

            Overlay::Menu { sel } | Overlay::Settings { sel } => {
                let items = if nested { &settings_items } else { &menu_items };
                let n = items.len();
                match key {
                    Key::Down | Key::Char('j') => *sel = (*sel + 1) % n,
                    Key::Up | Key::Char('k') => *sel = (*sel + n - 1) % n,
                    Key::Enter | Key::Char(' ') | Key::Right | Key::Char('l') => {
                        let item = items[*sel];
                        self.run_menu(item);
                    }
                    Key::Esc | Key::Left | Key::Char('h') if nested => {
                        self.run_menu(MenuItem::Back)
                    }
                    Key::Esc => self.overlay = Overlay::None,
                    _ => {}
                }
            }

            Overlay::Sources { sel } => {
                let n = music::Source::ALL.len();
                match key {
                    Key::Down | Key::Char('j') => *sel = (*sel + 1) % n,
                    Key::Up | Key::Char('k') => *sel = (*sel + n - 1) % n,
                    Key::Enter | Key::Char(' ') => {
                        let chosen = music::Source::ALL[*sel];
                        let mut cfg = music::Config::load();
                        cfg.source = chosen;
                        // Choosing a source is asking for music.
                        cfg.enabled = true;
                        let _ = cfg.save();
                        // Restart the poller against the new source.
                        self.music = Music::spawn(cfg);
                        self.msg = format!("music: {}", chosen.label());
                        self.overlay = Overlay::None;
                    }
                    Key::Esc => self.overlay = Overlay::None,
                    _ => {}
                }
            }

            Overlay::Themes { sel, restore } => {
                let n = theme::presets().len() + 1;
                match key {
                    Key::Down | Key::Char('j') => {
                        let i = (*sel + 1) % n;
                        *sel = i;
                        self.preview(i);
                    }
                    Key::Up | Key::Char('k') => {
                        let i = (*sel + n - 1) % n;
                        *sel = i;
                        self.preview(i);
                    }
                    Key::Enter => {
                        let i = *sel;
                        if i == n - 1 {
                            self.theme.name = "Custom".into();
                            self.overlay = Overlay::Custom {
                                field: 0,
                                buf: String::new(),
                            };
                        } else {
                            let _ = theme::save(&self.theme);
                            self.msg = format!("theme: {}", self.theme.name);
                            self.overlay = Overlay::None;
                        }
                    }
                    Key::Esc => {
                        self.theme = restore.clone();
                        self.overlay = Overlay::None;
                    }
                    _ => {}
                }
            }

            Overlay::Custom { field, buf } => match key {
                Key::Down | Key::Enter => {
                    commit(&mut self.theme, *field, buf);
                    *field = (*field + 1) % theme::ROLES.len();
                    buf.clear();
                }
                Key::Up => {
                    commit(&mut self.theme, *field, buf);
                    *field = (*field + theme::ROLES.len() - 1) % theme::ROLES.len();
                    buf.clear();
                }
                Key::Backspace => {
                    buf.pop();
                }
                Key::Char(c) if c.is_ascii_hexdigit() && buf.len() < 6 => {
                    buf.push(c.to_ascii_lowercase());
                    // Six digits is a complete colour — apply it live.
                    if buf.len() == 6 {
                        commit(&mut self.theme, *field, buf);
                    }
                }
                Key::Esc => {
                    commit(&mut self.theme, *field, buf);
                    self.theme.name = "Custom".into();
                    let _ = theme::save(&self.theme);
                    self.msg = "theme: Custom saved".into();
                    self.overlay = Overlay::None;
                }
                _ => {}
            },

            Overlay::Player {
                tab,
                sel,
                follow,
                query,
                find,
                typing,
            } => {
                use music::Cmd;
                let len = match tab {
                    Tab::Queue => self.music.queue.len(),
                    Tab::Playlists => self.music.playlists.len(),
                    Tab::Search => self.music.results.len(),
                };
                if *typing {
                    // The playlist tab types into its finder; the others, into search.
                    let on_playlists = *tab == Tab::Playlists;
                    let buf = if on_playlists { find } else { query };
                    match key {
                        Key::Char(c) if !c.is_control() && buf.chars().count() < 80 => buf.push(c),
                        Key::Backspace => {
                            buf.pop();
                        }
                        Key::Enter => {
                            *typing = false;
                            *sel = 0;
                            let q = buf.trim().to_string();
                            if on_playlists {
                                self.music.playlists.clear();
                                self.music.note = Some(if q.is_empty() {
                                    "fetching your playlists…".into()
                                } else {
                                    format!("searching playlists for “{q}”…")
                                });
                                self.music
                                    .send(Cmd::Playlists((!q.is_empty()).then_some(q)));
                            } else if !q.is_empty() {
                                self.music.results.clear();
                                self.music.note = Some(format!("searching for “{q}”…"));
                                self.music.send(Cmd::Search(q));
                            }
                        }
                        Key::Esc => *typing = false,
                        _ => {}
                    }
                    return;
                }
                match key {
                    // From a playlist search, Esc goes back to your own first.
                    Key::Esc if *tab == Tab::Playlists && !find.is_empty() => {
                        find.clear();
                        *sel = 0;
                        self.music.note = Some("fetching your playlists…".into());
                        self.music.send(Cmd::Playlists(None));
                    }
                    Key::Esc | Key::F(7) => self.overlay = Overlay::None,
                    Key::Tab | Key::BackTab => {
                        *tab = if key == Key::Tab {
                            tab.next()
                        } else {
                            tab.prev()
                        };
                        *sel = 0;
                        *follow = *tab == Tab::Queue;
                        if *tab == Tab::Playlists && self.music.playlists.is_empty() {
                            self.music.note = Some("fetching your playlists…".into());
                            self.music.send(Cmd::Playlists(None));
                        }
                    }
                    Key::Char('/') => {
                        if *tab != Tab::Playlists {
                            *tab = Tab::Search;
                            *sel = 0;
                        }
                        *follow = false;
                        *typing = true;
                    }
                    Key::Down | Key::Char('j') => {
                        *sel = (*sel + 1).min(len.saturating_sub(1));
                        *follow = false;
                    }
                    Key::Up | Key::Char('k') => {
                        *sel = sel.saturating_sub(1);
                        *follow = false;
                    }
                    Key::PageDown => {
                        *sel = (*sel + 10).min(len.saturating_sub(1));
                        *follow = false;
                    }
                    Key::PageUp => {
                        *sel = sel.saturating_sub(10);
                        *follow = false;
                    }
                    Key::Enter => match *tab {
                        Tab::Queue => {
                            if let Some(it) = self.music.queue.get(*sel) {
                                *follow = true;
                                self.music.send(Cmd::JumpTo(it.pos));
                            }
                        }
                        Tab::Search => {
                            if let Some(it) = self.music.results.get(*sel) {
                                self.music.note = Some(format!("playing {}", it.title));
                                self.music.send(Cmd::Enqueue {
                                    id: it.id.clone(),
                                    now: true,
                                });
                            }
                        }
                        Tab::Playlists => {
                            if let Some(it) = self.music.playlists.get(*sel) {
                                self.music.note = Some(format!("loading {}…", it.title));
                                self.music.send(Cmd::Playlist {
                                    id: it.id.clone(),
                                    title: it.title.clone(),
                                    now: true,
                                });
                                // Over to the queue, to watch it fill.
                                *tab = Tab::Queue;
                                *sel = 0;
                                *follow = true;
                            }
                        }
                    },
                    Key::Char('a') if *tab == Tab::Search => {
                        if let Some(it) = self.music.results.get(*sel) {
                            self.music.note = Some(format!("up next: {}", it.title));
                            self.music.send(Cmd::Enqueue {
                                id: it.id.clone(),
                                now: false,
                            });
                        }
                    }
                    Key::Char('a') if *tab == Tab::Playlists => {
                        if let Some(it) = self.music.playlists.get(*sel) {
                            self.music.note =
                                Some(format!("queueing {} after this song…", it.title));
                            self.music.send(Cmd::Playlist {
                                id: it.id.clone(),
                                title: it.title.clone(),
                                now: false,
                            });
                        }
                    }
                    Key::Char(' ') => self.music.send(Cmd::PlayPause),
                    Key::Right => self.music.send(Cmd::Seek(10)),
                    Key::Left => self.music.send(Cmd::Seek(-10)),
                    Key::Char(']') | Key::Char('n') => self.music.send(Cmd::Next),
                    Key::Char('[') | Key::Char('p') => self.music.send(Cmd::Prev),
                    Key::Char('s') => self.music.send(Cmd::Shuffle),
                    Key::Char('r') => self.music.send(Cmd::Repeat),
                    Key::Char('+') | Key::Char('=') => self.music.send(Cmd::Volume(10)),
                    Key::Char('-') => self.music.send(Cmd::Volume(-10)),
                    Key::Char('l') => {
                        self.music.note = Some("liked".into());
                        self.music.send(Cmd::Like);
                    }
                    Key::F(n) => self.on_function_key(n),
                    _ => {}
                }
            }

            Overlay::Create { plan, buf, fresh } => match key {
                // The suggestion is selected: typing replaces it, Backspace
                // clears it, → or End keeps it to add to.
                Key::Char(c) if !c.is_control() => {
                    if std::mem::take(fresh) {
                        buf.clear();
                    }
                    if buf.chars().count() < 60 {
                        buf.push(c);
                    }
                }
                Key::Backspace => {
                    if std::mem::take(fresh) {
                        buf.clear();
                    } else {
                        buf.pop();
                    }
                }
                Key::Right | Key::End => *fresh = false,
                Key::Enter => {
                    let (plan, name) = (plan.clone(), buf.clone());
                    self.overlay = Overlay::None;
                    self.finish_create(plan, name);
                }
                Key::Esc => self.overlay = Overlay::None,
                _ => {}
            },

            Overlay::Rename {
                path, buf, fresh, ..
            } => match key {
                Key::Char(c) if !c.is_control() => {
                    if std::mem::take(fresh) {
                        buf.clear();
                    }
                    if buf.chars().count() < 60 {
                        buf.push(c);
                    }
                }
                Key::Backspace => {
                    if std::mem::take(fresh) {
                        buf.clear();
                    } else {
                        buf.pop();
                    }
                }
                Key::Right | Key::End => *fresh = false,
                Key::Enter => {
                    let (path, name) = (path.clone(), buf.clone());
                    self.overlay = Overlay::None;
                    self.finish_rename(path, name);
                }
                Key::Esc => self.overlay = Overlay::None,
                _ => {}
            },

            // Only `y` deletes. Enter is the fold key two rows up and the
            // fingers know it — it must not be able to destroy a chapter.
            Overlay::Confirm {
                path,
                name,
                permanent,
                ..
            } => match key {
                Key::Char('y') | Key::Char('Y') => {
                    let (path, name, permanent) = (path.clone(), name.clone(), *permanent);
                    self.overlay = Overlay::None;
                    self.finish_delete(path, name, permanent);
                }
                Key::Esc | Key::Enter | Key::Char('n') | Key::Char('N') | Key::Char('q') => {
                    self.overlay = Overlay::None;
                    self.msg = "kept it".into();
                }
                _ => {}
            },
        }
    }

    /// Status-bar hints for whichever pane has focus.
    pub fn hints(&self) -> String {
        let m = self.mod_label();
        // What Esc does comes first, so a narrow status bar never cuts it.
        let esc = if self.codex.is_some() {
            "Esc close note"
        } else {
            "Esc menu"
        };
        match self.focus {
            Focus::Tree => {
                let sel = self.visible.get(self.sel).copied();
                let keys: Vec<String> = create::offers(&self.project, sel)
                    .iter()
                    .map(|(k, w)| format!("{k} {w}"))
                    .collect();
                format!(
                    "{esc}  Tab pane  ↵ fold  {}  r rename  d delete  {m}Z undo  H history  {m}Q quit ",
                    keys.join("  ")
                )
            }
            Focus::Editor => {
                format!("{esc}  Tab pane  {m}K find anything  {m}Z undo  F8 spelling  {m}Q quit ")
            }
            Focus::Codex => {
                "Esc close  Tab pane  ↑↓ scenes  ↵ go there  o open the note  PgDn scroll ".into()
            }
            Focus::Clearing => {
                format!("{esc}  Tab pane  ←→ view  ↵ start/pause  r reset  {m}Q quit ")
            }
            Focus::Music => {
                format!("{esc}  Tab pane  ↵ open player  space pause  ←→ track  {m}Q quit ")
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
    let _ = fs::write(
        path,
        format!("date = \"{today}\"\nbaseline = {total}\nrule = {COUNT_RULE}\n"),
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

/// Apply a typed hex buffer to a role, ignoring anything unparseable.
fn commit(t: &mut Theme, field: usize, buf: &str) {
    if let Some(c) = theme::parse_hex(buf) {
        t.set_role(field, c);
        t.name = "Custom".into();
    }
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
