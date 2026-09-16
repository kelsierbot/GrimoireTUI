//! The corkboard: a scene per index card, laid out chapter by chapter.
//!
//! Everything on a card comes from the scene's own frontmatter — `pov`,
//! `status`, `synopsis`, `target` — plus its word count, so the board is a
//! view of the files rather than a second copy of anything.

use crate::manuscript::{self, Section};
use crate::project::{Kind, Project};

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub idx: usize,
    pub title: String,
    pub pov: Option<String>,
    pub status: Option<String>,
    pub synopsis: Option<String>,
    pub words: usize,
    pub target: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// The chapter's name, or "" for scenes that sit outside one.
    pub title: String,
    pub cards: Vec<Card>,
}

/// The status a card cycles through with `s`, in the order drafts move.
pub const STATUSES: [&str; 5] = ["idea", "outline", "draft", "revised", "done"];

pub fn next_status(current: Option<&str>) -> &'static str {
    match current.and_then(|c| STATUSES.iter().position(|s| s.eq_ignore_ascii_case(c.trim()))) {
        Some(i) => STATUSES[(i + 1) % STATUSES.len()],
        None => STATUSES[0],
    }
}

/// The top-level parts of the manuscript (acts), in order. Empty when the
/// book has no part level.
pub fn parts(p: &Project) -> Vec<usize> {
    p.roots
        .iter()
        .copied()
        .filter(|&r| p.nodes[r].kind == Kind::Container && p.nodes[r].in_manuscript && manuscript::section_of(p, r) == Section::Part)
        .collect()
}

/// The part (act) a node belongs to, if the book has parts.
pub fn part_of(p: &Project, parents: &[Option<usize>], idx: usize) -> Option<usize> {
    let all = parts(p);
    let mut cur = Some(idx);
    while let Some(i) = cur {
        if all.contains(&i) {
            return Some(i);
        }
        cur = parents[i];
    }
    None
}

/// Cards for one part (or the whole manuscript when `scope` is None), grouped
/// by chapter, in book order.
pub fn board(p: &Project, scope: Option<usize>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let starts: Vec<usize> = match scope {
        Some(s) => vec![s],
        None => p.roots.iter().copied().filter(|&r| p.nodes[r].in_manuscript && p.nodes[r].kind != Kind::Divider).collect(),
    };
    for s in starts {
        walk(p, s, "", &mut groups);
    }
    groups.retain(|g| !g.cards.is_empty());
    groups
}

fn walk(p: &Project, idx: usize, chapter: &str, groups: &mut Vec<Group>) {
    let n = &p.nodes[idx];
    match n.kind {
        Kind::Scene => {
            if groups.last().is_none_or(|g| g.title != chapter) {
                groups.push(Group { title: chapter.to_string(), cards: Vec::new() });
            }
            groups.last_mut().unwrap().cards.push(Card {
                idx,
                title: n.title.clone(),
                pov: n.pov.clone(),
                status: n.status.clone(),
                synopsis: n.meta("synopsis"),
                words: n.words(),
                target: n.meta("target").and_then(|t| t.parse().ok()).filter(|&t: &usize| t > 0),
            });
        }
        Kind::Container => {
            let title = if manuscript::section_of(p, idx) == Section::Chapter { n.title.as_str() } else { chapter };
            for &c in &n.children {
                walk(p, c, title, groups);
            }
        }
        Kind::Divider => {}
    }
}

/// Where each card sits when every chapter's cards are laid in rows of
/// `cols`: (card index in reading order, visual row, column).
pub fn layout(groups: &[Group], cols: usize) -> Vec<(usize, usize, usize)> {
    let cols = cols.max(1);
    let mut out = Vec::new();
    let mut row = 0;
    let mut n = 0;
    for g in groups {
        for (i, _) in g.cards.iter().enumerate() {
            out.push((n, row + i / cols, i % cols));
            n += 1;
        }
        row += g.cards.len().div_ceil(cols);
    }
    out
}

/// Move the selection one card left/right (reading order) or one row up/down
/// (nearest column), staying put at the edges.
pub fn step(groups: &[Group], cols: usize, sel: usize, dx: isize, dy: isize) -> usize {
    let pos = layout(groups, cols);
    if pos.is_empty() {
        return 0;
    }
    let sel = sel.min(pos.len() - 1);
    if dx != 0 {
        return (sel as isize + dx).clamp(0, pos.len() as isize - 1) as usize;
    }
    let (_, row, col) = pos[sel];
    let want = row as isize + dy;
    if want < 0 {
        return sel;
    }
    let in_row: Vec<&(usize, usize, usize)> = pos.iter().filter(|(_, r, _)| *r as isize == want).collect();
    match in_row.iter().rev().find(|(_, _, c)| *c <= col).or(in_row.first()) {
        Some((i, _, _)) => *i,
        None => sel,
    }
}

/// The POV characters in the order they first appear, so each keeps one
/// colour for the whole board.
pub fn povs(groups: &[Group]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in groups.iter().flat_map(|g| &g.cards) {
        if let Some(p) = &c.pov
            && !out.iter().any(|x| x.eq_ignore_ascii_case(p))
        {
            out.push(p.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn book() -> (PathBuf, Project) {
        let d = std::env::temp_dir().join(format!("grimoire-cork-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        for (ch, scenes) in [
            ("manuscript/01-Act-One/01-Chapter-One", vec![("01-Gravel", "pov: Wren\nstatus: draft\nsynopsis: Confronts the caretaker.\ntarget: 1200"), ("02-Lantern", "pov: Kaelen\nstatus: revised")]),
            ("manuscript/01-Act-One/02-Chapter-Two", vec![("01-Ashfall", "pov: Wren\nstatus: idea")]),
            ("manuscript/02-Act-Two/03-Chapter-Three", vec![("01-Tide", "pov: Oren")]),
        ] {
            fs::create_dir_all(d.join(ch)).unwrap();
            for (name, front) in scenes {
                fs::write(d.join(ch).join(format!("{name}.md")), format!("---\n{front}\n---\n\none two three\n")).unwrap();
            }
        }
        let p = Project::load(&d).unwrap();
        (d, p)
    }

    #[test]
    fn a_part_becomes_cards_grouped_by_chapter_with_their_frontmatter() {
        let (d, p) = book();
        let acts = parts(&p);
        assert_eq!(acts.len(), 2);
        let g = board(&p, Some(acts[0]));
        assert_eq!(g.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), ["Chapter One", "Chapter Two"]);
        let gravel = &g[0].cards[0];
        assert_eq!(gravel.pov.as_deref(), Some("Wren"));
        assert_eq!(gravel.synopsis.as_deref(), Some("Confronts the caretaker."));
        assert_eq!((gravel.words, gravel.target), (3, Some(1200)));
        assert_eq!(povs(&g), ["Wren", "Kaelen"]);
        assert_eq!(board(&p, None).iter().map(|g| g.cards.len()).sum::<usize>(), 4);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn arrows_move_along_rows_and_between_chapters_by_column() {
        let g = vec![
            Group { title: "A".into(), cards: (0..5).map(|i| card(i)).collect() },
            Group { title: "B".into(), cards: (5..7).map(|i| card(i)).collect() },
        ];
        // cols 3: A = rows 0 [0,1,2], 1 [3,4]; B = row 2 [5,6]
        assert_eq!(step(&g, 3, 1, 0, 1), 4, "down from column 1 lands in column 1");
        assert_eq!(step(&g, 3, 2, 0, 1), 4, "column 2 has nothing below; nearest to the left");
        assert_eq!(step(&g, 3, 4, 0, 1), 6);
        assert_eq!(step(&g, 3, 5, 0, -1), 3);
        assert_eq!(step(&g, 3, 0, 0, -1), 0, "stays at the top");
        assert_eq!(step(&g, 3, 6, 1, 0), 6, "stays at the end");
    }

    fn card(i: usize) -> Card {
        Card { idx: i, title: format!("{i}"), pov: None, status: None, synopsis: None, words: 0, target: None }
    }

    #[test]
    fn status_cycles_in_drafting_order() {
        assert_eq!(next_status(None), "idea");
        assert_eq!(next_status(Some("draft")), "revised");
        assert_eq!(next_status(Some("Done")), "idea");
        assert_eq!(next_status(Some("something else")), "idea");
    }
}
