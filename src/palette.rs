//! The command palette: one box that finds any action, scene, note or theme
//! by typing a few letters of its name, and shows the key that does the same
//! thing — so using it teaches the shortcuts.

use crate::project::Kind;

/// Everything the palette can do. Scenes and notes are opened by path.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    NewScene,
    NewChapter,
    NewPart,
    NewFolder,
    Rename,
    Delete,
    MoveUp,
    MoveDown,
    History,
    Save,
    Undo,
    Redo,
    FindInScene,
    FindInBook,
    CheckNames,
    Corkboard,
    OpenCodex,
    Spellcheck,
    Icons,
    SpellingSuggestions,
    Export,
    Sessions,
    SaveSession,
    ProjectMap,
    Compile,
    Themes,
    Theme(String),
    MusicToggle,
    MusicPlayer,
    MusicSource,
    PlayPause,
    NextTrack,
    PrevTrack,
    Timer,
    TimerReset,
    Menu,
    Quit,
    Open(std::path::PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// What it's called: "New chapter", "Gravel".
    pub label: String,
    /// Where or what it is, shown dim: "Act One › Chapter Two", "scene".
    pub detail: String,
    /// The shortcut that does the same thing, if there is one.
    pub key: String,
    pub action: Action,
}

impl Entry {
    fn new(label: impl Into<String>, key: &str, action: Action) -> Entry {
        Entry { label: label.into(), detail: String::new(), key: key.into(), action }
    }
}

/// Everything available right now, actions first in the order they're most
/// often wanted, then scenes and notes in book order, then themes.
pub fn entries(app: &crate::app::App) -> Vec<Entry> {
    let m = app.mod_label();
    let part = app.project.meta.part_noun();
    let mut v = vec![
        Entry::new("Find in this scene", &format!("{m}F"), Action::FindInScene),
        Entry::new("Find in the whole book", "/", Action::FindInBook),
        Entry::new("New scene", "n", Action::NewScene),
        Entry::new("New chapter", "c", Action::NewChapter),
        Entry::new(format!("New {part}"), "p", Action::NewPart),
        Entry::new("New folder", "N", Action::NewFolder),
        Entry::new("Rename", "r", Action::Rename),
        Entry::new("Delete (to the trash)", "d", Action::Delete),
        Entry::new("Move up", "Alt ↑", Action::MoveUp),
        Entry::new("Move down", "Alt ↓", Action::MoveDown),
        Entry::new("Scene history", "H", Action::History),
        Entry::new("Undo", &format!("{m}Z"), Action::Undo),
        Entry::new("Redo", &format!("{m}Y"), Action::Redo),
        Entry::new("Save now", &format!("{m}S"), Action::Save),
        Entry::new("Corkboard", "b", Action::Corkboard),
        Entry::new("Open the note for the name under the cursor", &format!("{m}O"), Action::OpenCodex),
        Entry::new("Check names for near-miss spellings", "", Action::CheckNames),
        Entry::new(
            if app.spell_on { "Turn spellcheck off" } else { "Turn spellcheck on" },
            "",
            Action::Spellcheck,
        ),
        Entry::new(
            if app.icons_on { "Turn tree icons off" } else { "Turn tree icons on" },
            "",
            Action::Icons,
        ),
        Entry::new("Spelling suggestions for this word", "F8", Action::SpellingSuggestions),
        Entry::new("Export for readers (Word, EPUB)", "", Action::Export),
        Entry::new("Writing sessions", "", Action::Sessions),
        Entry::new("Save this session now", "", Action::SaveSession),
        Entry::new("Update project map", "", Action::ProjectMap),
        Entry::new("Compile manuscript (Markdown)", "", Action::Compile),
        Entry::new("Themes", "F9", Action::Themes),
        Entry::new(
            if app.music.enabled { "Turn music off" } else { "Turn music on" },
            "",
            Action::MusicToggle,
        ),
    ];
    if app.music.enabled {
        v.extend([
            Entry::new("Music player", "F7", Action::MusicPlayer),
            Entry::new("Music source", "", Action::MusicSource),
            Entry::new("Play / pause", "F5", Action::PlayPause),
            Entry::new("Next track", "F6", Action::NextTrack),
            Entry::new("Previous track", "F4", Action::PrevTrack),
        ]);
    }
    v.extend([
        Entry::new("Start or pause the timer", "F2", Action::Timer),
        Entry::new("Reset the timer", "F3", Action::TimerReset),
        Entry::new("Menu", "F1", Action::Menu),
        Entry::new("Quit", &format!("{m}Q"), Action::Quit),
    ]);

    // Scenes and notes, by name, with where they live.
    let p = &app.project;
    for (i, n) in p.nodes.iter().enumerate() {
        if n.kind != Kind::Scene || p.in_trash(i) {
            continue;
        }
        let mut trail = Vec::new();
        let mut up = app.parents[i];
        while let Some(pi) = up {
            if p.nodes[pi].kind == Kind::Container {
                trail.push(p.nodes[pi].title.clone());
            }
            up = app.parents[pi];
        }
        trail.reverse();
        let what = match n.area {
            crate::project::Area::Manuscript => "scene",
            crate::project::Area::FrontMatter | crate::project::Area::Format => "document",
            crate::project::Area::Templates => "sheet",
            _ => "note",
        };
        let detail = if trail.is_empty() { what.to_string() } else { format!("{what} · {}", trail.join(" › ")) };
        v.push(Entry { label: n.title.clone(), detail, key: String::new(), action: Action::Open(n.path.clone()) });
    }

    for t in crate::theme::presets() {
        v.push(Entry {
            label: t.name.clone(),
            detail: "theme".into(),
            key: String::new(),
            action: Action::Theme(t.name),
        });
    }
    v
}

/// How well `query` matches `text`, or None if it doesn't. Each word of the
/// query must appear in order as a run of letters; matching at the start of a
/// word, matching consecutive letters and matching early all score higher.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut pos = 0usize;
    let mut total = 0i32;
    for word in query.to_lowercase().split_whitespace() {
        let word: Vec<char> = word.chars().collect();
        let mut best: Option<(i32, usize)> = None;
        // Try every place this word could start, keep the best.
        for start in pos..text.len() {
            if text[start] != word[0] {
                continue;
            }
            let mut ti = start;
            let mut s = 0i32;
            let mut last = None;
            let mut ok = true;
            for &wc in &word {
                while ti < text.len() && text[ti] != wc {
                    ti += 1;
                }
                if ti >= text.len() {
                    ok = false;
                    break;
                }
                let at_word_start = ti == 0 || !text[ti - 1].is_alphanumeric();
                s += if at_word_start { 12 } else { 1 };
                if last.is_some_and(|l: usize| l + 1 == ti) {
                    s += 8;
                }
                last = Some(ti);
                ti += 1;
            }
            if !ok {
                break;
            }
            s -= (start as i32) / 4;
            if best.is_none_or(|(b, _)| s > b) {
                best = Some((s, ti));
            }
        }
        let (s, end) = best?;
        total += s;
        pos = end;
    }
    Some(total)
}

/// The entries matching `query`, best first. An empty query keeps the
/// natural order.
pub fn filter(all: &[Entry], query: &str) -> Vec<Entry> {
    if query.trim().is_empty() {
        return all.iter().filter(|e| !matches!(e.action, Action::Open(_) | Action::Theme(_))).cloned().collect();
    }
    let mut scored: Vec<(i32, usize, &Entry)> = all
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            // The name matters most; where it lives can still find it, read
            // either way round ("gravel chapter two", "chapter two gravel").
            let name = score(query, &e.label);
            let after = score(query, &format!("{} {}", e.label, e.detail)).map(|s| s - 20);
            let before = score(query, &format!("{} {}", e.detail, e.label)).map(|s| s - 20);
            name.max(after).max(before).map(|s| (s, i, e))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, e)| e.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(label: &str, action: Action) -> Entry {
        Entry::new(label, "", action)
    }

    #[test]
    fn a_few_letters_of_each_word_find_it() {
        assert!(score("new ch", "New chapter").is_some());
        assert!(score("nch", "New chapter").is_some());
        assert!(score("chapter new", "New chapter").is_none(), "words keep their order");
        assert!(score("xyz", "New chapter").is_none());
    }

    #[test]
    fn word_starts_beat_letters_buried_in_the_middle() {
        let all = vec![
            e("Turn music on", Action::MusicToggle),
            e("New chapter", Action::NewChapter),
            e("Scene history", Action::History),
        ];
        let got = filter(&all, "new ch");
        assert_eq!(got[0].action, Action::NewChapter);
        let got = filter(&all, "hist");
        assert_eq!(got[0].action, Action::History);
    }

    #[test]
    fn a_scene_is_found_by_where_it_lives_too() {
        let mut gravel = e("Gravel", Action::Open("a.md".into()));
        gravel.detail = "scene · Act One › Chapter Two".into();
        let all = vec![e("Move up", Action::MoveUp), gravel];
        let got = filter(&all, "chapter two gravel");
        assert!(matches!(got[0].action, Action::Open(_)));
        assert_eq!(filter(&all, "gravel").len(), 1);
    }

    #[test]
    fn an_empty_query_lists_actions_not_every_scene() {
        let all = vec![e("Save now", Action::Save), e("Gravel", Action::Open("a.md".into())), e("Nord", Action::Theme("Nord".into()))];
        let got = filter(&all, "  ");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].action, Action::Save);
    }
}
