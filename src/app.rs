//! Application state and key handling.

use anyhow::Result;
use ratatui::layout::Rect;
use std::fs;
use std::path::{Path, PathBuf};

use crate::editor::Editor;
use crate::manuscript;
use crate::music::{self, Music};
use crate::project::{self, Kind, Project};
use crate::scene::{Mode, Pomodoro};
use crate::theme::{self, Theme};
use crate::visualizer::Visualizer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Editor,
    Clearing,
    Music,
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
    pub baseline: usize,
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
    pub theme: Theme,
    pub overlay: Overlay,
}

/// Modal state. Only one can be up at a time.
#[derive(Debug, Clone, PartialEq)]
pub enum Overlay {
    None,
    /// The main menu — the discoverable way to reach everything.
    Menu { sel: usize },
    /// Browsing presets. `restore` is put back if you press Esc.
    Themes { sel: usize, restore: Theme },
    /// Choosing where music comes from.
    Sources { sel: usize },
    /// Editing the custom theme swatch by swatch.
    Custom { field: usize, buf: String },
    /// Naming a new scene or folder. `dir` is where it will go; `label` says
    /// so in the prompt.
    Create {
        folder: bool,
        dir: PathBuf,
        label: String,
        buf: String,
    },
    /// The music player: what's playing, the queue, and search.
    Player {
        /// Which tab: the queue, or search results.
        search: bool,
        sel: usize,
        /// Keep the selection on the playing track until you move it yourself.
        follow: bool,
        query: String,
        typing: bool,
    },
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
        let visible = project.visible();
        let mut parents = vec![None; project.nodes.len()];
        for (i, n) in project.nodes.iter().enumerate() {
            for &c in &n.children {
                parents[c] = Some(i);
            }
        }
        let baseline = load_baseline(&project)?;
        // Open the first scene straight away so launching lands you on prose
        // rather than an empty pane. Focus stays on the tree, so a stray
        // keystroke can't wander into the manuscript.
        let first_scene = project
            .visible()
            .into_iter()
            .find(|&i| project.nodes[i].kind == Kind::Scene && project.nodes[i].in_manuscript);

        Ok(Self {
            project,
            visible,
            parents,
            sel: 0,
            tree_scroll: 0,
            open: None,
            editor: Editor::from_str(""),
            focus: Focus::Tree,
            baseline,
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
            theme: theme::load(),
            overlay: Overlay::None,
        })
        .map(|mut app: App| {
            if let Some(i) = first_scene {
                app.editor = Editor::from_str(&app.project.nodes[i].body);
                app.open = Some(i);
                if let Some(pos) = app.visible.iter().position(|&v| v == i) {
                    app.sel = pos;
                }
            }
            app
        })
    }

    /// Function keys drive the timer and the music, from either pane, so they
    /// never collide with typing.
    pub fn on_function_key(&mut self, n: u8) {
        match n {
            2 => self.pomo.toggle(),
            3 => {
                self.pomo.reset();
                self.msg = "timer reset".into();
            }
            4 => self.music.send(music::Cmd::Prev),
            5 => self.music.send(music::Cmd::PlayPause),
            6 => self.music.send(music::Cmd::Next),
            7 => self.open_player(),
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
    fn flush(&mut self) {
        if let Some(i) = self.open {
            let text = self.editor.text();
            if text != self.project.nodes[i].body {
                self.project.nodes[i].body = text;
                self.project.nodes[i].dirty = true;
            }
        }
    }

    fn open_scene(&mut self, idx: usize) {
        self.flush();
        self.editor = Editor::from_str(&self.project.nodes[idx].body);
        self.open = Some(idx);
        self.focus = Focus::Editor;
        self.msg.clear();
    }

    pub fn save(&mut self) {
        self.flush();
        match self.project.save_all() {
            Ok(0) => self.msg = "nothing to save".into(),
            Ok(n) => self.msg = format!("saved {n} scene{}", if n == 1 { "" } else { "s" }),
            Err(e) => self.msg = format!("save failed: {e}"),
        }
    }

    // ---- creating scenes and folders --------------------------------------

    /// Where `n` (scene) or `N` (folder) puts the new item, judged from the
    /// selection, plus a phrase for the prompt. Always the end of a folder:
    /// nothing existing is ever renamed or renumbered.
    fn creation_target(&self, folder: bool) -> (PathBuf, String) {
        let root = &self.project.root;
        let section = |path: &Path| -> (PathBuf, String) {
            let top = path
                .strip_prefix(root)
                .ok()
                .and_then(|p| p.components().next())
                .map(|c| root.join(c))
                .unwrap_or_else(|| root.join("manuscript"));
            let label = match top.file_name().and_then(|s| s.to_str()) {
                Some("notes") => "notes",
                Some("front-matter") => "the front matter",
                _ => "the manuscript",
            };
            (top, label.to_string())
        };
        let inside = |i: usize| {
            let n = &self.project.nodes[i];
            (n.path.clone(), n.title.clone())
        };
        let Some(&idx) = self.visible.get(self.sel) else {
            return section(&root.join("manuscript"));
        };
        let node = &self.project.nodes[idx];
        let up = |i: usize| self.parents[i];
        match (node.kind, folder) {
            (Kind::Divider, _) => section(&node.path),
            // A scene goes into the selected chapter, or the selected scene's.
            (Kind::Container, false) => inside(idx),
            (Kind::Scene, false) => up(idx).map_or_else(|| section(&node.path), inside),
            // A folder goes beside the selected one: Chapter Two after Chapter One.
            (Kind::Container, true) => up(idx).map_or_else(|| section(&node.path), inside),
            (Kind::Scene, true) => up(idx)
                .and_then(up)
                .map_or_else(|| section(&node.path), inside),
        }
    }

    pub fn start_create(&mut self, folder: bool) {
        let (dir, label) = self.creation_target(folder);
        self.overlay = Overlay::Create {
            folder,
            dir,
            label,
            buf: String::new(),
        };
    }

    fn finish_create(&mut self, folder: bool, dir: PathBuf, name: String) {
        // Save first, so re-reading the tree can't lose an unsaved sentence.
        self.flush();
        let saved = match self.project.save_all() {
            Ok(n) => n,
            Err(e) => {
                self.msg = format!("couldn't save before creating: {e}");
                return;
            }
        };
        let path = match project::create(&dir, &name, folder) {
            Ok(p) => p,
            Err(e) => {
                self.msg = format!("couldn't create it: {e}");
                return;
            }
        };
        if let Err(e) = self.reload_tree() {
            self.msg = format!("created, but couldn't re-read the tree: {e}");
            return;
        }
        if let Some(i) = self.project.nodes.iter().position(|n| n.path == path) {
            let mut p = self.parents[i];
            while let Some(pi) = p {
                self.project.nodes[pi].expanded = true;
                p = self.parents[pi];
            }
            self.refresh_visible();
            if let Some(pos) = self.visible.iter().position(|&v| v == i) {
                self.sel = pos;
            }
            if !folder {
                self.open_scene(i);
            }
        }
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        let also = if saved > 0 { format!(" · saved {saved} first") } else { String::new() };
        self.msg = format!("created {file}{also}");
    }

    /// Re-read the tree from disk, keeping what's folded, which scene is open,
    /// and the editor exactly as it is.
    fn reload_tree(&mut self) -> Result<()> {
        let collapsed: Vec<PathBuf> = self
            .project
            .nodes
            .iter()
            .filter(|n| n.kind == Kind::Container && !n.expanded)
            .map(|n| n.path.clone())
            .collect();
        let open_path = self.open.map(|i| self.project.nodes[i].path.clone());
        let root = self.project.root.clone();
        self.project = Project::load(&root)?;
        for n in &mut self.project.nodes {
            if collapsed.contains(&n.path) {
                n.expanded = false;
            }
        }
        self.parents = vec![None; self.project.nodes.len()];
        for (i, n) in self.project.nodes.iter().enumerate() {
            for &c in &n.children {
                self.parents[c] = Some(i);
            }
        }
        self.open = open_path.and_then(|p| self.project.nodes.iter().position(|n| n.path == p));
        self.refresh_visible();
        Ok(())
    }

    // ---- key handling -----------------------------------------------------

    pub fn on_tree_key(&mut self, key: Key) {
        match key {
            Key::Char('t') => self.open_theme_picker(),
            Key::Char('n') => self.start_create(false),
            Key::Char('N') => self.start_create(true),
            Key::Up => self.sel = self.sel.saturating_sub(1),
            Key::Down => {
                if self.sel + 1 < self.visible.len() {
                    self.sel += 1;
                }
            }
            Key::Left => {
                let idx = self.visible[self.sel];
                let n = &self.project.nodes[idx];
                if n.kind == Kind::Container && n.expanded {
                    self.project.nodes[idx].expanded = false;
                    self.refresh_visible();
                } else if let Some(p) = self.parents[idx] {
                    if let Some(pos) = self.visible.iter().position(|&v| v == p) {
                        self.sel = pos;
                    }
                }
            }
            // Space is a second Enter here — folding is the most repeated
            // action in the tree and the thumb is already there.
            Key::Enter | Key::Char(' ') => {
                let idx = self.visible[self.sel];
                match self.project.nodes[idx].kind {
                    Kind::Container => {
                        let open = self.project.nodes[idx].expanded;
                        self.project.nodes[idx].expanded = !open;
                        self.refresh_visible();
                    }
                    Kind::Scene => self.open_scene(idx),
                    Kind::Divider => {}
                }
            }
            Key::Right => {
                let idx = self.visible[self.sel];
                match self.project.nodes[idx].kind {
                    Kind::Container => {
                        self.project.nodes[idx].expanded = true;
                        self.refresh_visible();
                    }
                    Kind::Scene => self.open_scene(idx),
                    Kind::Divider => {}
                }
            }
            _ => {}
        }
    }

    pub fn on_editor_key(&mut self, key: Key) {
        // Collapse the selection on any keystroke. Deliberately does NOT
        // delete it — there is no undo yet, so a stray key must not eat text.
        self.editor.clear_selection();
        let rows = self.editor.layout(self.edit_width);
        match key {
            Key::Char(c) => self.editor.insert(c),
            Key::Enter => self.editor.newline(),
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
            Key::Esc => {
                self.flush();
                self.focus = Focus::Tree;
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
            Focus::Clearing => self.scene_visible,
            Focus::Music => self.music_visible,
        }
    }

    /// Tab through every pane that is actually on screen.
    pub fn cycle_focus(&mut self, forward: bool) {
        const ORDER: [Focus; 4] = [Focus::Tree, Focus::Editor, Focus::Clearing, Focus::Music];
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
            Key::Esc => self.focus = Focus::Tree,
            _ => {}
        }
    }

    pub fn on_music_key(&mut self, key: Key) {
        match key {
            Key::Enter => self.open_player(),
            Key::Char(' ') => self.music.send(music::Cmd::PlayPause),
            Key::Right | Key::Char('l') | Key::Char('n') => self.music.send(music::Cmd::Next),
            Key::Left | Key::Char('h') | Key::Char('p') => self.music.send(music::Cmd::Prev),
            Key::Esc => self.focus = Focus::Tree,
            _ => {}
        }
    }

    /// A left click focuses the pane under the pointer and acts on it.
    pub fn on_click(&mut self, x: u16, y: u16) {
        if hit(self.rect_tree, x, y) {
            if self.focus == Focus::Editor {
                self.flush();
            }
            self.focus = Focus::Tree;
            let row = self.tree_scroll + (y - self.rect_tree.y) as usize;
            if row < self.visible.len() {
                self.sel = row;
                let idx = self.visible[row];
                match self.project.nodes[idx].kind {
                    Kind::Container => {
                        let open = self.project.nodes[idx].expanded;
                        self.project.nodes[idx].expanded = !open;
                        self.refresh_visible();
                    }
                    Kind::Scene => self.open_scene(idx),
                    Kind::Divider => {}
                }
            }
        } else if hit(self.rect_editor, x, y) && self.open.is_some() {
            self.focus = Focus::Editor;
            let rows = self.editor.layout(self.edit_width);
            let vis = self.editor.scroll + (y - self.rect_editor.y) as usize;
            self.editor.click(&rows, vis, (x - self.rect_editor.x) as usize);
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

    /// Extend the editor selection while the left button is held.
    pub fn on_drag(&mut self, x: u16, y: u16) {
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
                self.tree_scroll = (self.tree_scroll + STEP)
                    .min(self.visible.len().saturating_sub(1));
            } else {
                self.tree_scroll = self.tree_scroll.saturating_sub(STEP);
            }
        } else if hit(self.rect_editor, x, y) {
            let rows = self.editor.layout(self.edit_width);
            if down {
                self.editor.scroll = (self.editor.scroll + STEP)
                    .min(rows.len().saturating_sub(1));
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

    pub const MENU: [&'static str; 8] = [
        "New scene…          (n)",
        "New folder…         (N)",
        "Update project map  (project.md)",
        "Compile manuscript",
        "Music player…       (F7)",
        "Music source…",
        "Themes…",
        "Close",
    ];

    pub fn open_menu(&mut self) {
        self.overlay = Overlay::Menu { sel: 0 };
    }

    pub fn open_player(&mut self) {
        self.overlay = Overlay::Player {
            search: false,
            sel: 0,
            follow: true,
            query: String::new(),
            typing: false,
        };
        self.music.note = None;
        self.music.send(music::Cmd::FetchQueue);
        self.player_fetched = Some(std::time::Instant::now());
    }

    /// While the player is open, keep the queue fresh and the selection on
    /// the playing track (until you move it yourself).
    pub fn tick_player(&mut self) {
        let Overlay::Player { search, sel, follow, .. } = &mut self.overlay else {
            return;
        };
        if !*search && *follow {
            if let Some(i) = self.music.queue.iter().position(|it| it.current) {
                *sel = i;
            }
        }
        if self
            .player_fetched
            .is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(4))
        {
            self.music.send(music::Cmd::FetchQueue);
            self.player_fetched = Some(std::time::Instant::now());
        }
    }

    fn run_menu(&mut self, i: usize) {
        match i {
            0 => self.start_create(false),
            1 => self.start_create(true),
            2 => {
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
            3 => {
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
            4 => self.open_player(),
            5 => {
                let cur = self.music.source;
                let sel = music::Source::ALL.iter().position(|s| *s == cur).unwrap_or(0);
                self.overlay = Overlay::Sources { sel };
            }
            6 => self.open_theme_picker(),
            _ => self.overlay = Overlay::None,
        }
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
        match &mut self.overlay {
            Overlay::None => {}

            Overlay::Menu { sel } => {
                let n = App::MENU.len();
                match key {
                    Key::Down | Key::Char('j') => *sel = (*sel + 1) % n,
                    Key::Up | Key::Char('k') => *sel = (*sel + n - 1) % n,
                    Key::Enter | Key::Char(' ') => {
                        let i = *sel;
                        self.run_menu(i);
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
                _ => {},
            },

            Overlay::Player { search, sel, follow, query, typing } => {
                use music::Cmd;
                let len = if *search { self.music.results.len() } else { self.music.queue.len() };
                if *typing {
                    match key {
                        Key::Char(c) if !c.is_control() && query.chars().count() < 80 => query.push(c),
                        Key::Backspace => {
                            query.pop();
                        }
                        Key::Enter => {
                            *typing = false;
                            *sel = 0;
                            let q = query.trim().to_string();
                            if !q.is_empty() {
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
                    Key::Esc | Key::F(7) => self.overlay = Overlay::None,
                    Key::Tab | Key::BackTab => {
                        *search = !*search;
                        *sel = 0;
                        *follow = !*search;
                    }
                    Key::Char('/') => {
                        *search = true;
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
                    Key::Enter => {
                        if *search {
                            if let Some(it) = self.music.results.get(*sel) {
                                self.music.note = Some(format!("playing {}", it.title));
                                self.music.send(Cmd::Enqueue { id: it.id.clone(), now: true });
                            }
                        } else if let Some(it) = self.music.queue.get(*sel) {
                            *follow = true;
                            self.music.send(Cmd::JumpTo(it.pos));
                        }
                    }
                    Key::Char('a') if *search => {
                        if let Some(it) = self.music.results.get(*sel) {
                            self.music.note = Some(format!("added {} to the end of the queue", it.title));
                            self.music.send(Cmd::Enqueue { id: it.id.clone(), now: false });
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

            Overlay::Create { folder, dir, buf, .. } => match key {
                Key::Char(c) if !c.is_control() && buf.chars().count() < 60 => buf.push(c),
                Key::Backspace => {
                    buf.pop();
                }
                Key::Enter => {
                    let (folder, dir, name) = (*folder, dir.clone(), buf.clone());
                    self.overlay = Overlay::None;
                    self.finish_create(folder, dir, name);
                }
                Key::Esc => self.overlay = Overlay::None,
                _ => {}
            },
        }
    }

    /// Status-bar hints for whichever pane has focus.
    pub fn hints(&self) -> String {
        let m = self.mod_label();
        match self.focus {
            Focus::Tree => format!("Tab pane  ↵ fold  n scene  N folder  F1 menu  {m}S save  {m}Q quit "),
            Focus::Editor => format!("Tab pane  Esc tree  F1 menu  {m}S save  {m}Q quit "),
            Focus::Clearing => format!("Tab pane  ←→ view  ↵ start/pause  r reset  {m}Q quit "),
            Focus::Music => format!("Tab pane  ↵ open player  space pause  ←→ track  {m}Q quit "),
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

/// Today's starting word count, so the status line can show a session delta.
/// Stored in `.grimoire/progress.toml`, which belongs in .gitignore.
fn load_baseline(project: &Project) -> Result<usize> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let dir = project.root.join(".grimoire");
    let path = dir.join("progress.toml");
    let total = project.total_words();

    if let Ok(s) = fs::read_to_string(&path) {
        let mut date = String::new();
        let mut baseline = total;
        for line in s.lines() {
            if let Some(v) = line.strip_prefix("date = ") {
                date = v.trim().trim_matches('"').to_string();
            } else if let Some(v) = line.strip_prefix("baseline = ") {
                baseline = v.trim().parse().unwrap_or(total);
            }
        }
        if date == today {
            return Ok(baseline);
        }
    }

    write_baseline(&dir, &path, &today, total);
    Ok(total)
}

fn write_baseline(dir: &Path, path: &Path, today: &str, total: usize) {
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(path, format!("date = \"{today}\"\nbaseline = {total}\n"));
}

/// Hand text to whatever clipboard tool this machine has.
fn copy_to_clipboard(text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    const TOOLS: &[(&str, &[&str])] = &[
        ("pbcopy", &[]),                              // macOS
        ("wl-copy", &[]),                             // Wayland
        ("xclip", &["-selection", "clipboard"]),      // X11
        ("xsel", &["--clipboard", "--input"]),        // X11 alternative
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
