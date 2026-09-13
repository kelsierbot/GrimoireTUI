//! Application state and key handling.

use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::editor::Editor;
use crate::project::{Kind, Project};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Editor,
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
}

impl App {
    pub fn new(project: Project) -> Result<Self> {
        let visible = project.visible();
        let mut parents = vec![None; project.nodes.len()];
        for (i, n) in project.nodes.iter().enumerate() {
            for &c in &n.children {
                parents[c] = Some(i);
            }
        }
        let baseline = load_baseline(&project)?;
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
        })
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

    // ---- key handling -----------------------------------------------------

    pub fn on_tree_key(&mut self, key: Key) {
        match key {
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
            Key::Right | Key::Enter => {
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

    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Tree if self.open.is_some() => Focus::Editor,
            Focus::Tree => Focus::Tree,
            Focus::Editor => {
                self.flush();
                Focus::Tree
            }
        };
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
