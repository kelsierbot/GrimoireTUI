//! Writing aids: the notes & TK list, revision passes, echoes, and sprints.
//!
//! A child of `app`, so it works on the App's own state; kept apart so the
//! features read as one piece.

use std::path::PathBuf;
use std::time::Duration;

use super::{App, Focus, Overlay};
use crate::scene::Phase;
use grimoire_core::notes::{self, MarkKind};
use grimoire_core::project::Kind;
use grimoire_core::revision;

/// One row of the Ctrl-T list.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkRow {
    pub scene: PathBuf,
    /// Where it is, in the tree's words: "Chapter Two › Gravel".
    pub place: String,
    pub kind: MarkKind,
    pub line: usize,
    pub col: usize,
    pub end: usize,
    pub snippet: String,
}

/// A writing sprint: so many words while the timer's focus runs.
#[derive(Debug, Clone, PartialEq)]
pub struct Sprint {
    pub goal: usize,
    /// The book's word count when it started (moved, like "today", when a
    /// scene is deleted or brought back — that isn't writing).
    pub start: usize,
    pub reached: bool,
    /// The timer's usual focus length, put back when the sprint ends.
    pub usual: Duration,
}

impl Sprint {
    pub fn written(&self, total: usize) -> i64 {
        total as i64 - self.start as i64
    }
}

/// The rows whose scene, place or words contain every word typed.
pub fn filter_marks<'a>(rows: &'a [MarkRow], query: &str) -> Vec<&'a MarkRow> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    rows.iter()
        .filter(|r| {
            let hay = format!(
                "{} {} {}",
                r.place.to_lowercase(),
                r.snippet.to_lowercase(),
                if r.kind == MarkKind::Tk { "tk" } else { "note" }
            );
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

impl App {
    /// Every `%% note %%` and TK in the book, in book order.
    pub(super) fn mark_rows(&mut self) -> Vec<MarkRow> {
        self.flush();
        let mut out = Vec::new();
        for i in revision::scenes_in_order(&self.project) {
            let n = &self.project.nodes[i];
            for m in notes::marks(&n.body) {
                out.push(MarkRow {
                    scene: n.path.clone(),
                    place: grimoire_core::search::place_of(&self.project, &self.parents, i),
                    kind: m.kind,
                    line: m.line,
                    col: m.col,
                    end: m.end,
                    snippet: m.snippet,
                });
            }
        }
        out
    }

    /// Ctrl-T: the list of notes and TKs.
    pub fn open_marks(&mut self) {
        let rows = self.mark_rows();
        if rows.is_empty() {
            self.msg = "no notes or TKs yet — %% like this %% or TK, in any scene".into();
            return;
        }
        self.overlay = Overlay::Marks {
            query: String::new(),
            sel: 0,
            rows,
        };
    }

    /// The next note or TK after the cursor, across the book, wrapping.
    pub fn next_tk(&mut self) {
        let rows = self.mark_rows();
        if rows.is_empty() {
            self.msg = "no notes or TKs in the book".into();
            return;
        }
        let here = self.open.map(|i| self.project.nodes[i].path.clone());
        let order: Vec<PathBuf> = revision::scenes_in_order(&self.project)
            .into_iter()
            .map(|i| self.project.nodes[i].path.clone())
            .collect();
        let rank = |p: &PathBuf| order.iter().position(|o| o == p).unwrap_or(0);
        let now = (
            here.as_ref().map_or(0, rank),
            self.editor.cy,
            self.editor.cx,
        );
        let next = rows
            .iter()
            .find(|r| (rank(&r.scene), r.line, r.col) > now)
            .unwrap_or(&rows[0])
            .clone();
        self.go_to_mark(&next);
    }

    pub(super) fn go_to_mark(&mut self, r: &MarkRow) {
        self.go_to_hit(&r.scene, r.line, r.col, r.end);
        self.msg = match r.kind {
            MarkKind::Tk => format!("TK · {}", r.place),
            MarkKind::Note => format!("note · {}", r.place),
        };
    }

    // ---- revision ------------------------------------------------------

    /// Open the next scene still waiting for its revision.
    pub fn next_draft(&mut self) {
        self.flush();
        let left = revision::drafts(&self.project).len();
        let Some(i) = revision::next_in_draft(&self.project, self.open) else {
            self.msg = "every written scene is revised or done".into();
            return;
        };
        self.reveal(i);
        if self.open != Some(i) {
            self.open_scene(i);
        }
        self.focus = Focus::Editor;
        let status = self.project.nodes[i]
            .status
            .clone()
            .unwrap_or_else(|| "no status".into());
        self.msg = format!(
            "{} · {status} · {left} scene{} still in draft",
            self.project.nodes[i].title,
            if left == 1 { "" } else { "s" }
        );
    }

    /// The notebook's names, which may repeat as often as the story needs.
    pub fn echo_skip(&self) -> Vec<String> {
        self.codex_index
            .iter()
            .flat_map(|e| e.names.iter().cloned())
            .collect()
    }

    pub fn toggle_echoes(&mut self) {
        self.echo_on = !self.echo_on;
        self.msg = if self.echo_on {
            let n = revision::echo_count(&revision::echoes(&self.editor.lines, &self.echo_skip()));
            format!(
                "echo words on · {n} in this scene, within {} words of each other",
                revision::ECHO_WINDOW
            )
        } else {
            "echo words off".into()
        };
    }

    // ---- sprints -------------------------------------------------------

    pub fn start_sprint_dialog(&mut self) {
        let minutes = self
            .sprint
            .as_ref()
            .map_or(self.pomo.focus_len, |s| s.usual)
            .as_secs()
            / 60;
        self.overlay = Overlay::Sprint {
            words: "500".into(),
            minutes: minutes.max(1).to_string(),
            on_minutes: false,
            fresh: true,
        };
    }

    pub fn start_sprint(&mut self, goal: usize, length: Duration) {
        self.flush();
        let usual = self.sprint.take().map_or(self.pomo.focus_len, |s| s.usual);
        self.pomo.reset();
        self.pomo.focus_len = length;
        self.pomo.toggle();
        self.sprint = Some(Sprint {
            goal,
            start: self.project.total_words(),
            reached: false,
            usual,
        });
        self.msg = format!("sprint: {goal} words in {} minutes", length.as_secs() / 60);
    }

    /// End the sprint, putting the timer's usual length back.
    pub fn end_sprint(&mut self, why: &str) {
        let Some(s) = self.sprint.take() else {
            return;
        };
        let written = s.written(self.project.total_words()).max(0);
        self.pomo.focus_len = s.usual;
        self.msg = format!("{why} · {written} words");
    }

    /// Called every tick: one quiet note at the goal, and the end when the
    /// timer's focus runs out (or is reset).
    pub fn tick_sprint(&mut self) {
        let total = self.project.total_words();
        let Some(s) = &mut self.sprint else {
            return;
        };
        if !s.reached && s.written(total) >= s.goal as i64 {
            s.reached = true;
            self.msg = format!("sprint goal reached · {} words", s.goal);
        }
        if self.pomo.phase != Phase::Focus {
            self.end_sprint("sprint over");
        }
    }

    /// The status bar's `sprint 212/500 · 14:32`, while one runs.
    pub fn sprint_label(&self) -> Option<(String, bool)> {
        let s = self.sprint.as_ref()?;
        let r = self.pomo.remaining().as_secs();
        let pause = if self.pomo.running() { "" } else { " (paused)" };
        Some((
            format!(
                "sprint {}/{} · {:02}:{:02}{pause}",
                s.written(self.project.total_words()),
                s.goal,
                r / 60,
                r % 60
            ),
            s.reached,
        ))
    }

    /// The open scene's progress towards its `target:`, when it has one.
    pub fn scene_target(&self) -> Option<(usize, usize)> {
        let n = &self.project.nodes[self.open?];
        if n.kind != Kind::Scene {
            return None;
        }
        let target = n
            .meta("target")?
            .replace([',', '_'], "")
            .trim()
            .parse()
            .ok()?;
        (target > 0).then(|| (n.words(), target))
    }

    /// Misspellings with anything inside a note or TK left out.
    pub fn misspellings_outside_marks(
        list: Vec<(usize, usize)>,
        marks: &[(usize, usize, MarkKind)],
    ) -> Vec<(usize, usize)> {
        list.into_iter()
            .filter(|&(a, b)| !marks.iter().any(|&(s, e, _)| a < e && b > s))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(place: &str, snippet: &str, kind: MarkKind) -> MarkRow {
        MarkRow {
            scene: PathBuf::from(place),
            place: place.into(),
            kind,
            line: 0,
            col: 0,
            end: 0,
            snippet: snippet.into(),
        }
    }

    #[test]
    fn the_list_filters_on_every_word_typed() {
        let rows = vec![
            row("Chapter One › Gravel", "check the weather", MarkKind::Note),
            row("Chapter Two › Lamp", "She drove to TK.", MarkKind::Tk),
        ];
        assert_eq!(filter_marks(&rows, "").len(), 2);
        assert_eq!(filter_marks(&rows, "weather").len(), 1);
        assert_eq!(filter_marks(&rows, "tk lamp").len(), 1);
        assert_eq!(filter_marks(&rows, "note gravel").len(), 1);
        assert_eq!(filter_marks(&rows, "note lamp").len(), 0);
    }

    #[test]
    fn spelling_leaves_notes_and_tks_alone() {
        let bad = vec![(0, 4), (10, 14), (20, 22)];
        let marks = vec![(8, 16, MarkKind::Note), (20, 22, MarkKind::Tk)];
        assert_eq!(App::misspellings_outside_marks(bad, &marks), vec![(0, 4)]);
    }
}
