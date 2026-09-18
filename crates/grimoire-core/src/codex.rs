//! The living codex: the notebook and the manuscript knowing each other.
//!
//! Every note in the notebook is a name the prose can mention — its title, any
//! `aliases:` in its frontmatter (the way Obsidian spells them), and the
//! distinctive words of a longer title ("Kaelen" from "Kaelen Voss", but not
//! "Ashen" from "Ashen Reach", which is an ordinary word). Nothing is stored:
//! the index is rebuilt from the files whenever the tree is read.

use std::path::PathBuf;

use crate::project::{Kind, Project};
use crate::search;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub note: PathBuf,
    pub title: String,
    /// The notebook section it lives in: "Characters", "Regions".
    pub section: String,
    /// Every spelling that means this note, longest first.
    pub names: Vec<String>,
}

/// `aliases: [Kae, the Ferryman]` or a YAML list under `aliases:`.
pub fn aliases(front: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_list = false;
    for line in front.lines() {
        if in_list {
            if let Some(item) = line.trim_start().strip_prefix("- ") {
                let v = item.trim().trim_matches(['"', '\'']);
                if !v.is_empty() {
                    out.push(v.to_string());
                }
                continue;
            }
            in_list = false;
        }
        let Some((k, v)) = line.split_once(':') else { continue };
        if k.trim() != "aliases" && k.trim() != "alias" {
            continue;
        }
        let v = v.trim();
        if v.is_empty() {
            in_list = true;
        } else {
            let inner = v.trim_start_matches('[').trim_end_matches(']');
            out.extend(inner.split(',').map(|s| s.trim().trim_matches(['"', '\'']).to_string()).filter(|s| !s.is_empty()));
        }
    }
    out
}

/// Every note in the notebook, with the names that mean it. `ordinary` says
/// whether a word is everyday English, so title words like "Ashen" or "Reach"
/// aren't treated as names on their own.
pub fn index(p: &Project, parents: &[Option<usize>], ordinary: &dyn Fn(&str) -> bool) -> Vec<Entry> {
    let mut out = Vec::new();
    for (i, n) in p.nodes.iter().enumerate() {
        if n.kind != Kind::Scene || !n.area.is_notebook() || p.in_trash(i) {
            continue;
        }
        let mut section = String::new();
        let mut up = parents[i];
        while let Some(pi) = up {
            if p.nodes[pi].kind == Kind::Container {
                section = p.nodes[pi].title.clone();
            }
            up = parents[pi];
        }
        let mut names: Vec<String> = vec![n.title.trim().to_string()];
        if let Some(f) = &n.front {
            names.extend(aliases(f));
        }
        let words: Vec<&str> = n.title.split_whitespace().collect();
        if words.len() > 1 {
            for w in words {
                let w = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
                if w.chars().count() >= 4 && w.chars().next().is_some_and(char::is_uppercase) && !ordinary(&w.to_lowercase()) {
                    names.push(w.to_string());
                }
            }
        }
        names.retain(|s| s.chars().count() >= 2);
        names.sort_by_key(|s| std::cmp::Reverse(s.chars().count()));
        names.dedup();
        out.push(Entry { note: n.path.clone(), title: n.title.clone(), section, names });
    }
    out
}

/// Where names from the index occur in one paragraph: (start, end, entry).
/// Whole words only, exact case, longest name first so "Kaelen Voss" wins
/// over "Kaelen"; a trailing possessive isn't part of the name.
pub fn spans(line: &str, entries: &[Entry]) -> Vec<(usize, usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let mut all: Vec<(usize, &str)> = entries
        .iter()
        .enumerate()
        .flat_map(|(i, e)| e.names.iter().map(move |n| (i, n.as_str())))
        .collect();
    all.sort_by_key(|(_, n)| std::cmp::Reverse(n.chars().count()));
    let mut taken = vec![false; chars.len()];
    let mut out = Vec::new();
    for (entry, name) in all {
        let needle: Vec<char> = name.chars().collect();
        if needle.is_empty() || needle.len() > chars.len() {
            continue;
        }
        let mut i = 0;
        while i + needle.len() <= chars.len() {
            let end = i + needle.len();
            let boundary_before = i == 0 || !chars[i - 1].is_alphanumeric();
            let boundary_after = end == chars.len() || !chars[end].is_alphanumeric() || is_possessive(&chars, end);
            if chars[i..end] == needle[..] && boundary_before && boundary_after && !taken[i..end].iter().any(|&t| t) {
                taken[i..end].iter_mut().for_each(|t| *t = true);
                out.push((i, end, entry));
                i = end;
            } else {
                i += 1;
            }
        }
    }
    out.sort();
    out
}

fn is_possessive(chars: &[char], at: usize) -> bool {
    matches!(chars.get(at), Some('\'' | '’')) && chars.get(at + 1) == Some(&'s') && chars.get(at + 2).is_none_or(|c| !c.is_alphanumeric())
}

/// The note a `[[link]]` under the cursor points at, by file name or title.
pub fn link_at(line: &str, cursor: usize, entries: &[Entry]) -> Option<usize> {
    let chars: Vec<char> = line.chars().collect();
    let text: String = chars.iter().collect();
    let before: String = chars[..cursor.min(chars.len())].iter().collect();
    let open = before.rfind("[[")?;
    let rest = &text[open + 2..];
    let close = rest.find("]]")?;
    let target = rest[..close].split(['|', '#']).next()?.trim();
    let target_stem = target.rsplit('/').next().unwrap_or(target);
    entries.iter().position(|e| {
        let stem = e.note.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        stem == target_stem || e.title == target_stem || e.names.iter().any(|n| n == target_stem)
    })
}

/// One scene that mentions a note.
#[derive(Debug, Clone, PartialEq)]
pub struct Appearance {
    pub scene: PathBuf,
    pub place: String,
    pub count: usize,
}

/// Every manuscript scene that mentions this note by any of its names, in
/// book order.
pub fn appearances(p: &Project, parents: &[Option<usize>], entry: &Entry) -> Vec<Appearance> {
    let one = std::slice::from_ref(entry);
    let mut out = Vec::new();
    for &r in &p.roots {
        walk(p, parents, r, one, &mut out);
    }
    out
}

fn walk(p: &Project, parents: &[Option<usize>], idx: usize, one: &[Entry], out: &mut Vec<Appearance>) {
    let n = &p.nodes[idx];
    if p.in_trash(idx) || !n.in_manuscript {
        return;
    }
    if n.kind == Kind::Scene {
        let count: usize = n.body.split('\n').map(|l| spans(l, one).len()).sum();
        if count > 0 {
            out.push(Appearance { scene: n.path.clone(), place: search::place_of(p, parents, idx), count });
        }
    }
    for &c in &n.children {
        walk(p, parents, c, one, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn aliases_read_both_ways_obsidian_writes_them() {
        assert_eq!(aliases("title: Kaelen\naliases: [Kae, \"the Ferryman\"]\n"), ["Kae", "the Ferryman"]);
        assert_eq!(aliases("aliases:\n  - Kae\n  - the Ferryman\nstatus: x\n"), ["Kae", "the Ferryman"]);
        assert!(aliases("title: Kaelen\n").is_empty());
    }

    fn entry(title: &str, names: &[&str]) -> Entry {
        Entry { note: PathBuf::from(format!("{title}.md")), title: title.into(), section: "Characters".into(), names: names.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn names_match_whole_words_longest_first_and_keep_their_possessive_out() {
        let e = vec![entry("Kaelen Voss", &["Kaelen Voss", "Kaelen"]), entry("Wren", &["Wren"])];
        let line = "Kaelen Voss nodded. Kaelen's hand, then Wren. Wrenna laughed.";
        let found = spans(line, &e);
        let text = |s: usize, e: usize| line.chars().skip(s).take(e - s).collect::<String>();
        assert_eq!(found.iter().map(|&(s, e, _)| text(s, e)).collect::<Vec<_>>(), ["Kaelen Voss", "Kaelen", "Wren"]);
        assert_eq!(found[1].2, 0);
        assert_eq!(found[2].2, 1);
    }

    #[test]
    fn a_link_under_the_cursor_finds_its_note() {
        let e = vec![entry("Kaelen", &["Kaelen"])];
        let line = "See [[01-Kaelen|him]] now";
        let mut e2 = e.clone();
        e2[0].note = PathBuf::from("notes/01-Characters/01-Kaelen.md");
        assert_eq!(link_at(line, 8, &e2), Some(0));
        assert_eq!(link_at(line, 1, &e2), None);
    }

    #[test]
    fn the_index_and_appearances_come_from_the_files() {
        let d = std::env::temp_dir().join(format!("grimoire-codex-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("manuscript/01-Act-One/01-Chapter-One")).unwrap();
        fs::create_dir_all(d.join("notes/01-Characters")).unwrap();
        fs::create_dir_all(d.join("notes/03-Regions")).unwrap();
        fs::write(d.join("manuscript/01-Act-One/01-Chapter-One/01-Gravel.md"), "Kae waited. Kaelen Voss came.\nKaelen left.\n").unwrap();
        fs::write(d.join("manuscript/01-Act-One/01-Chapter-One/02-Tide.md"), "Nobody here.\n").unwrap();
        fs::write(d.join("notes/01-Characters/01-Kaelen-Voss.md"), "---\ntitle: Kaelen Voss\naliases: [Kae]\n---\n\nGrey eyes.\n").unwrap();
        fs::write(d.join("notes/03-Regions/01-Ashen-Reach.md"), "---\ntitle: Ashen Reach\n---\n\nAsh.\n").unwrap();
        let p = Project::load(&d).unwrap();
        let parents = p.parents();
        let ordinary = |w: &str| matches!(w, "ashen" | "reach");
        let idx = index(&p, &parents, &ordinary);
        let kaelen = idx.iter().find(|e| e.title == "Kaelen Voss").unwrap();
        assert_eq!(kaelen.names, ["Kaelen Voss", "Kaelen", "Voss", "Kae"], "a surname on its own means them too");
        assert_eq!(kaelen.section, "Characters");
        let reach = idx.iter().find(|e| e.title == "Ashen Reach").unwrap();
        assert_eq!(reach.names, ["Ashen Reach"], "ordinary words aren't names on their own");
        let seen = appearances(&p, &parents, kaelen);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].count, 3);
        assert_eq!(seen[0].place, "Act One › Chapter One › Gravel");
        fs::remove_dir_all(&d).unwrap();
    }
}
