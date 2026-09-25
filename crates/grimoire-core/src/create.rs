//! What `n`, `c`, `p` and `N` make, and where it goes.
//!
//! The tree speaks the book's language: a scene goes in a chapter, a chapter
//! in a part, a part at the end of the manuscript. New items always land at
//! the end of their folder, so nothing existing is renamed or renumbered.

use std::path::{Path, PathBuf};

use crate::manuscript::{self, Section};
use crate::project::{self, Area, Kind, Project};

/// Which key was pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum New {
    Scene,
    Chapter,
    Part,
    /// `N`: a folder beside the selected one. In the manuscript that's a
    /// chapter or a part, and it's treated as one.
    Folder,
}

impl New {
    pub fn from_key(c: char) -> Option<New> {
        match c {
            'n' => Some(New::Scene),
            'c' => Some(New::Chapter),
            'p' => Some(New::Part),
            'N' => Some(New::Folder),
            _ => None,
        }
    }
}

/// Everything the naming prompt needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// "scene", "chapter", "part", "act", "note" or "folder" — the book's own
    /// word, so a project that calls its parts Acts is told "new act".
    pub noun: String,
    /// What it turned out to be, whichever key was pressed.
    pub made: New,
    pub folder: bool,
    /// Where it will be made.
    pub dir: PathBuf,
    /// Says so: "in Part One, after Chapter One".
    pub place: String,
    /// What the name starts as: "Chapter Two", or empty for a scene.
    pub name: String,
}

/// Which section the selection sits in. Nothing is made in the trash or in
/// Novel Format, so those make things in the manuscript.
fn area(p: &Project, sel: Option<usize>) -> Area {
    match sel.map(|i| p.nodes[i].area) {
        Some(Area::Trash | Area::Format) | None => Area::Manuscript,
        Some(a) => a,
    }
}

fn area_dir(p: &Project, a: Area) -> PathBuf {
    a.path(&p.root)
}

/// What `n` makes in a section.
fn item_noun(a: Area) -> &'static str {
    match a {
        Area::Manuscript => "scene",
        Area::FrontMatter => "document",
        Area::Templates => "sheet",
        _ => "note",
    }
}

/// The create keys worth showing for whatever is selected.
pub fn offers(p: &Project, sel: Option<usize>) -> Vec<(char, String)> {
    let own = |v: &[(char, &str)]| -> Vec<(char, String)> {
        v.iter().map(|&(k, w)| (k, w.to_string())).collect()
    };
    match area(p, sel) {
        Area::Manuscript => vec![
            ('n', "scene".into()),
            ('c', "chapter".into()),
            ('p', p.meta.part_noun()),
        ],
        a => own(&[('n', item_noun(a)), ('N', "folder")]),
    }
}

/// Work out what pressing `want` makes, judged from the selection. An error
/// is a sentence for the status line.
pub fn plan(
    p: &Project,
    parents: &[Option<usize>],
    sel: Option<usize>,
    want: New,
) -> Result<Plan, String> {
    let a = area(p, sel);
    let ms = area_dir(p, Area::Manuscript);
    let here = sel.and_then(|i| folder_of(p, parents, i));
    let is_part = |f: usize| manuscript::section_of(p, f) == Section::Part;

    // `N` in the manuscript makes whatever sits beside the selection.
    let want = match want {
        New::Folder if a == Area::Manuscript => match here {
            Some(f) if is_part(f) => New::Part,
            _ => New::Chapter,
        },
        w => w,
    };

    let (noun, folder, dir, name) = match want {
        New::Part => (
            p.meta.part_noun(),
            true,
            ms,
            numbered(p.meta.part_word(), count(p, Section::Part) + 1),
        ),
        New::Chapter => {
            // Beside the selected chapter or inside the selected part. From
            // the notes, into the last part: `c` always means the manuscript.
            let dir = match here.filter(|&f| p.nodes[f].in_manuscript) {
                Some(f) if is_part(f) => p.nodes[f].path.clone(),
                Some(f) => parent_dir(&p.nodes[f].path, &ms),
                None => last_in(p, &ms, Section::Part).map_or(ms, |i| p.nodes[i].path.clone()),
            };
            (
                "chapter".to_string(),
                true,
                dir,
                numbered("Chapter", count(p, Section::Chapter) + 1),
            )
        }
        New::Scene => {
            let noun = item_noun(a);
            let dir = match here {
                // A scene dropped into a part would turn it into a chapter,
                // so it goes in the part's last chapter instead.
                Some(f) if p.nodes[f].in_manuscript && is_part(f) => {
                    match last_in(p, &p.nodes[f].path, Section::Chapter) {
                        Some(c) => p.nodes[c].path.clone(),
                        None => {
                            return Err(format!(
                                "{} has no chapters yet — press c to make one",
                                p.nodes[f].title
                            ));
                        }
                    }
                }
                Some(f) => p.nodes[f].path.clone(),
                None => area_dir(p, a),
            };
            (noun.to_string(), false, dir, String::new())
        }
        New::Folder => {
            let top = area_dir(p, a);
            let dir = here.map_or(top.clone(), |f| parent_dir(&p.nodes[f].path, &top));
            ("folder".to_string(), true, dir, String::new())
        }
    };
    Ok(Plan {
        noun,
        made: want,
        folder,
        place: place(p, &dir),
        dir,
        name,
    })
}

/// The folder the selection stands for: itself, or a scene's folder.
fn folder_of(p: &Project, parents: &[Option<usize>], i: usize) -> Option<usize> {
    match p.nodes[i].kind {
        Kind::Container => Some(i),
        Kind::Scene => parents[i],
        Kind::Category => None,
    }
}

fn parent_dir(path: &Path, or: &Path) -> PathBuf {
    path.parent().unwrap_or(or).to_path_buf()
}

/// The last folder directly inside `dir` that is a `level`.
fn last_in(p: &Project, dir: &Path, level: Section) -> Option<usize> {
    (0..p.nodes.len()).rev().find(|&i| {
        let n = &p.nodes[i];
        n.kind == Kind::Container
            && n.path.parent() == Some(dir)
            && manuscript::section_of(p, i) == level
    })
}

/// How many chapters (or parts) the book already has.
fn count(p: &Project, level: Section) -> usize {
    (0..p.nodes.len())
        .filter(|&i| {
            let n = &p.nodes[i];
            n.kind == Kind::Container && n.in_manuscript && manuscript::section_of(p, i) == level
        })
        .count()
}

/// "in Part One, after Chapter One". New items are numbered after everything
/// already there, so they follow the highest-numbered entry.
fn place(p: &Project, dir: &Path) -> String {
    let within = match p.nodes.iter().find(|n| n.path == dir) {
        Some(n) if n.kind == Kind::Category => match n.area {
            Area::Manuscript => "the manuscript".into(),
            Area::FrontMatter => "the front matter".into(),
            a => a.title().into(),
        },
        Some(n) => n.title.clone(),
        None => "the manuscript".into(),
    };
    let after = p
        .nodes
        .iter()
        .filter(|n| n.kind != Kind::Category && n.path.parent() == Some(dir))
        .filter_map(|n| project::leading_number(&n.path).map(|k| (k, n)))
        .max_by_key(|&(k, _)| k);
    match after {
        Some((_, n)) => format!("in {within}, after {}", n.title),
        None => format!("in {within}"),
    }
}

use crate::manuscript::numbered;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project;
    use std::fs;

    /// A small book built by hand: Part One / Chapter One / Opening, plus one
    /// note. Deliberately *not* the template — these tests are about where a
    /// new thing lands, and they shouldn't move every time the template does.
    fn book(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grimoire-new-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let ch = d.join("manuscript/01-part-one/01-chapter-one");
        fs::create_dir_all(&ch).unwrap();
        fs::write(
            ch.join("01-opening.md"),
            "---\ntitle: Opening\n---\n\nWords.\n",
        )
        .unwrap();
        let notes = d.join("notes/01-characters");
        fs::create_dir_all(&notes).unwrap();
        fs::write(
            notes.join("01-example.md"),
            "---\ntitle: Example\n---\n\nA note.\n",
        )
        .unwrap();
        fs::write(d.join("novel.toml"), "title = \"Test\"\n").unwrap();
        d
    }

    /// Press `want` with the row titled `sel` selected.
    fn press(d: &Path, sel: &str, want: New) -> Result<Plan, String> {
        let p = Project::load(d).unwrap();
        let i = p.nodes.iter().position(|n| n.title == sel);
        let i = i.unwrap_or_else(|| panic!("no {sel} in the tree"));
        plan(&p, &p.parents(), Some(i), want)
    }

    #[test]
    fn c_on_a_scene_adds_the_next_chapter_to_its_part() {
        let d = book("c-scene");
        let plan = press(&d, "Opening", New::Chapter).unwrap();
        assert_eq!(plan.noun, "chapter");
        assert!(plan.folder);
        assert_eq!(plan.dir, d.join("manuscript/01-part-one"));
        assert_eq!(plan.name, "Chapter Two");
        assert_eq!(plan.place, "in Part One, after Chapter One");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn c_on_a_new_empty_chapter_goes_beside_it() {
        let d = book("c-empty");
        project::create(&d.join("manuscript/01-part-one"), "Chapter Two", true).unwrap();
        let plan = press(&d, "Chapter Two", New::Chapter).unwrap();
        assert_eq!(plan.dir, d.join("manuscript/01-part-one"));
        assert_eq!(plan.name, "Chapter Three");
        assert_eq!(plan.place, "in Part One, after Chapter Two");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn c_on_a_part_goes_inside_it() {
        let d = book("c-part");
        project::create(&d.join("manuscript"), "Part Two", true).unwrap();
        let plan = press(&d, "Part Two", New::Chapter).unwrap();
        assert_eq!(plan.dir, d.join("manuscript/02-Part-Two"));
        assert_eq!(plan.name, "Chapter Two");
        assert_eq!(plan.place, "in Part Two");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn c_from_the_notes_goes_to_the_last_part() {
        let d = book("c-notes");
        project::create(&d.join("manuscript"), "Part Two", true).unwrap();
        let plan = press(&d, "Example", New::Chapter).unwrap();
        assert_eq!(plan.dir, d.join("manuscript/02-Part-Two"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn c_in_a_book_without_parts_goes_at_the_top() {
        let d = std::env::temp_dir().join(format!("grimoire-new-flat-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let ch = d.join("manuscript/01-chapter-one");
        fs::create_dir_all(&ch).unwrap();
        fs::write(ch.join("01-opening.md"), "Words.\n").unwrap();
        let plan = press(&d, "Opening", New::Chapter).unwrap();
        assert_eq!(plan.dir, d.join("manuscript"));
        assert_eq!(plan.place, "in the manuscript, after Chapter One");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn p_adds_the_next_part_at_the_end_of_the_manuscript() {
        let d = book("p");
        let plan = press(&d, "Opening", New::Part).unwrap();
        assert_eq!(plan.noun, "part");
        assert!(plan.folder);
        assert_eq!(plan.dir, d.join("manuscript"));
        assert_eq!(plan.name, "Part Two");
        assert_eq!(plan.place, "in the manuscript, after Part One");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn n_on_a_part_goes_into_its_last_chapter() {
        let d = book("n-part");
        project::create(&d.join("manuscript/01-part-one"), "Chapter Two", true).unwrap();
        let plan = press(&d, "Part One", New::Scene).unwrap();
        assert_eq!(plan.noun, "scene");
        assert!(!plan.folder);
        assert_eq!(plan.dir, d.join("manuscript/01-part-one/02-Chapter-Two"));
        assert_eq!(plan.name, "");
        assert_eq!(plan.place, "in Chapter Two");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn n_on_an_empty_part_asks_for_a_chapter_first() {
        let d = book("n-empty-part");
        project::create(&d.join("manuscript"), "Part Two", true).unwrap();
        let err = press(&d, "Part Two", New::Scene).unwrap_err();
        assert!(err.contains("Part Two") && err.contains(" c "), "{err}");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn n_in_the_notes_makes_a_note_in_that_folder() {
        let d = book("n-notes");
        let plan = press(&d, "Example", New::Scene).unwrap();
        assert_eq!(plan.noun, "note");
        assert_eq!(plan.dir, d.join("notes/01-characters"));
        assert_eq!(plan.place, "in Characters, after Example");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn shift_n_in_the_manuscript_is_a_chapter_or_a_part() {
        let d = book("folder-manuscript");
        let beside_chapter = press(&d, "Chapter One", New::Folder).unwrap();
        assert_eq!(beside_chapter.noun, "chapter");
        assert_eq!(beside_chapter.dir, d.join("manuscript/01-part-one"));
        let beside_part = press(&d, "Part One", New::Folder).unwrap();
        assert_eq!(beside_part.noun, "part");
        assert_eq!(beside_part.dir, d.join("manuscript"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn shift_n_in_the_notes_is_a_folder_beside_the_selected_one() {
        let d = book("folder-notes");
        let plan = press(&d, "Characters", New::Folder).unwrap();
        assert_eq!(plan.noun, "folder");
        assert_eq!(plan.dir, d.join("notes"));
        assert_eq!(plan.name, "");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_tree_offers_book_words_in_the_manuscript_and_folders_in_notes() {
        let d = book("offers");
        let p = Project::load(&d).unwrap();
        let at = |t: &str| p.nodes.iter().position(|n| n.title == t);
        let words = |v: Vec<(char, String)>| -> Vec<(char, String)> { v };
        assert_eq!(
            words(offers(&p, at("Opening"))),
            vec![
                ('n', "scene".to_string()),
                ('c', "chapter".to_string()),
                ('p', "part".to_string())
            ]
        );
        assert_eq!(
            words(offers(&p, at("Example"))),
            vec![('n', "note".to_string()), ('N', "folder".to_string())]
        );
        fs::remove_dir_all(&d).unwrap();
    }

    /// A book that calls its parts Acts is offered an act, and naming follows.
    #[test]
    fn the_part_label_is_the_books_own_word() {
        let d = book("acts");
        fs::write(
            d.join("novel.toml"),
            "title = \"Acts\"\ntarget_words = 80000\ndaily_target = 1000\npart_label = \"Act\"\n",
        )
        .unwrap();
        let p = Project::load(&d).unwrap();
        let at = |t: &str| p.nodes.iter().position(|n| n.title == t);
        assert_eq!(offers(&p, at("Opening"))[2], ('p', "act".to_string()));
        let plan = plan(&p, &p.parents(), at("Opening"), New::Part).unwrap();
        assert_eq!(plan.noun, "act");
        assert_eq!(plan.name, "Act Two");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn names_are_spelled_out() {
        assert_eq!(numbered("Part", 3), "Part Three");
        assert_eq!(numbered("Chapter", 21), "Chapter Twenty-One");
        assert_eq!(numbered("Chapter", 120), "Chapter 120");
    }
}
