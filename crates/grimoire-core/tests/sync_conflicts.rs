//! Sync apps' conflict copies, dropped beside a scene the way each app does
//! it, and what the book makes of them: shown, never counted or exported,
//! carried along by moves, and settled without losing either version.

use grimoire_core::export::{ExportOptions, export};
use grimoire_core::project::{self, Project, Settle};
use grimoire_core::sync::{self, Source};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

fn book(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("grimoire-syncconf-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(d.join("manuscript/01-Act-One/01-Chapter-One")).unwrap();
    fs::write(d.join("novel.toml"), "title = \"The Crossing\"\n").unwrap();
    put(
        &d,
        "01-Scene-One.md",
        "Scene One",
        "The ferry left at dawn.",
    );
    put(&d, "02-Scene-Two.md", "Scene Two", "Gulls over the wake.");
    d
}

fn chapter(d: &Path) -> PathBuf {
    d.join("manuscript/01-Act-One/01-Chapter-One")
}

fn put(d: &Path, name: &str, title: &str, body: &str) -> PathBuf {
    let p = chapter(d).join(name);
    fs::write(&p, format!("---\ntitle: \"{title}\"\n---\n\n{body}\n")).unwrap();
    p
}

fn node<'a>(p: &'a Project, name: &str) -> &'a project::Node {
    p.nodes
        .iter()
        .find(|n| n.path.file_name().is_some_and(|f| f == name))
        .unwrap_or_else(|| panic!("{name} not in the tree"))
}

fn names(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(chapter(d))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.starts_with('.'))
        .collect();
    v.sort();
    v
}

/// Every app's way of naming the version that lost, with who it says it is.
const COPIES: &[(&str, Source)] = &[
    (
        "01-Scene-One (Josh's conflicted copy 2026-09-25).md",
        Source::Dropbox,
    ),
    ("01-Scene-One (conflicted copy).md", Source::Dropbox),
    ("01-Scene-One (Case Conflict).md", Source::Dropbox),
    (
        "01-Scene-One (Unicode Encoding Conflict).md",
        Source::Dropbox,
    ),
    (
        "01-Scene-One (copie en conflit de Josh 2026-09-25).md",
        Source::Dropbox,
    ),
    ("01-Scene-One (conflicted).md", Source::PCloud),
    ("01-Scene-One (conflicted 2).md", Source::PCloud),
    ("01-Scene-One [conflicted].md", Source::PCloud),
    ("01-Scene-One.conflicted.md", Source::PCloud),
    ("01-Scene-One (josh@example.com).md", Source::Box),
    ("01-Scene-One (1).md", Source::SyncApp),
    ("01-Scene-One-DESKTOP-4F2K9QX.md", Source::OneDrive),
    ("01-Scene-One 2.md", Source::ICloud),
    (
        "01-Scene-One.sync-conflict-20260925-140233-ABCDEFG.md",
        Source::Syncthing,
    ),
    ("01-Scene-One [Conflict].md", Source::GoogleDrive),
    ("01-Scene-One_conf(1).md", Source::GoogleDrive),
    (
        "01-Scene-One (from phone, 2026-09-25 14-02).md",
        Source::Grimoire,
    ),
];

#[test]
fn every_apps_copy_is_shown_named_and_never_counted() {
    for (name, source) in COPIES {
        let d = book(&format!("each-{}", source.name().replace(' ', "")));
        put(&d, name, "Scene One", "The ferry left at noon, late, loud.");
        let p = Project::load(&d).unwrap();
        let copy = node(&p, name);
        assert!(copy.parked, "{name} is a conflict copy");
        let of = copy.copy_of.as_ref().unwrap();
        assert_eq!(of.source, *source, "{name}");
        assert_eq!(of.original, "01-Scene-One", "{name}");
        assert_eq!(
            p.total_words(),
            5 + 4,
            "{name}: the copy adds nothing to the book's count"
        );
        assert!(!copy.compile, "{name} is never compiled");
        let _ = fs::remove_dir_all(&d);
    }
}

#[test]
fn a_copy_sits_right_after_its_original_in_the_tree() {
    let d = book("order");
    // "(" sorts before ".", so on disk the copy comes first.
    put(&d, "01-Scene-One (1).md", "Scene One", "Other words.");
    let p = Project::load(&d).unwrap();
    let ch = p
        .nodes
        .iter()
        .find(|n| n.path == chapter(&d))
        .unwrap()
        .children
        .clone();
    let order: Vec<String> = ch
        .iter()
        .map(|&i| {
            p.nodes[i]
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    assert_eq!(
        order,
        ["01-Scene-One.md", "01-Scene-One (1).md", "02-Scene-Two.md"]
    );
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn names_that_could_be_titles_count_only_beside_their_original() {
    let d = book("ambiguous");
    // No "03-Chapter" or "04-Draft" or "05-Scene" beside these: they're scenes.
    put(&d, "03-Chapter 2.md", "Chapter 2", "Real words here.");
    put(&d, "04-Draft (1).md", "Draft (1)", "Real words here.");
    put(&d, "05-Scene-NYC.md", "Scene NYC", "Real words here.");
    put(&d, "06-The-Gate.md", "The Gate", "Real words here.");
    // An ordinary title that happens to extend another one.
    put(
        &d,
        "06-The-Gate-House.md",
        "The Gate House",
        "Real words here.",
    );
    let p = Project::load(&d).unwrap();
    for name in [
        "03-Chapter 2.md",
        "04-Draft (1).md",
        "05-Scene-NYC.md",
        "06-The-Gate-House.md",
    ] {
        assert!(!node(&p, name).parked, "{name} is an ordinary scene");
    }
    let _ = fs::remove_dir_all(&d);
}

fn unzip(path: &Path) -> Vec<(String, String)> {
    let bytes = fs::read(path).unwrap();
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut text = String::new();
            let _ = f.read_to_string(&mut text);
            (f.name().to_string(), text)
        })
        .collect()
}

#[test]
fn a_copy_never_reaches_an_export() {
    let d = book("export");
    put(
        &d,
        "01-Scene-One (Josh's conflicted copy 2026-09-25).md",
        "Scene One",
        "PARALLEL VERSION from the laptop.",
    );
    let p = Project::load(&d).unwrap();
    let out = export(&p, &ExportOptions::default()).unwrap();
    assert_eq!(out.words, 9, "only the book's own words");
    for file in &out.files {
        let entries = unzip(file);
        let mut seen_scene = false;
        for (name, text) in &entries {
            if name.ends_with(".xml") || name.ends_with(".xhtml") {
                // Well-formed, as the exporter promises.
                let opts = roxmltree::ParsingOptions {
                    allow_dtd: true,
                    ..Default::default()
                };
                roxmltree::Document::parse_with_options(text, opts)
                    .unwrap_or_else(|e| panic!("{name} in {}: {e}", file.display()));
                assert!(
                    !text.contains("PARALLEL VERSION"),
                    "{name}: the copy is out"
                );
                seen_scene |= text.contains("The ferry left at dawn.");
            }
        }
        assert!(seen_scene, "{}: the scene itself is in", file.display());
    }
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn a_moved_scene_takes_its_copies_with_it_and_brings_them_back() {
    let d = book("move");
    let copy_name = "01-Scene-One (Josh's conflicted copy 2026-09-25).md";
    put(&d, copy_name, "Scene One", "The laptop's version.");
    let one = chapter(&d).join("01-Scene-One.md");
    let moved = project::move_item(&d, &one, false).unwrap();
    assert_eq!(
        names(&d),
        [
            "01-Scene-Two.md",
            "02-Scene-One (Josh's conflicted copy 2026-09-25).md",
            "02-Scene-One.md",
        ],
        "the copy follows its scene, not the number"
    );
    // and back again, the way undo does it
    let back: Vec<(PathBuf, PathBuf)> = moved
        .renames
        .iter()
        .rev()
        .map(|(f, t)| (t.clone(), f.clone()))
        .collect();
    project::apply_moves(&d, &back, true).unwrap();
    assert_eq!(
        names(&d),
        [
            "01-Scene-One (Josh's conflicted copy 2026-09-25).md",
            "01-Scene-One.md",
            "02-Scene-Two.md",
        ]
    );
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn a_renamed_scene_takes_its_copies_with_it() {
    let d = book("rename");
    put(&d, "01-Scene-One (conflicted).md", "Scene One", "Other.");
    project::rename(&chapter(&d).join("01-Scene-One.md"), "The Ferry").unwrap();
    assert_eq!(
        names(&d),
        [
            "01-The-Ferry (conflicted).md",
            "01-The-Ferry.md",
            "02-Scene-Two.md"
        ]
    );
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn a_copy_is_never_moved_onto_a_file_already_there() {
    let d = book("taken");
    put(&d, "01-Scene-One (1).md", "Scene One", "A copy.");
    // Something unrelated already sits where the copy would follow to.
    let there = put(
        &d,
        "01-The-Ferry (1).md",
        "Ferry (1)",
        "Not a copy of anything.",
    );
    project::rename(&chapter(&d).join("01-Scene-One.md"), "The Ferry").unwrap();
    assert_eq!(
        text(&there),
        "---\ntitle: \"Ferry (1)\"\n---\n\nNot a copy of anything.\n",
        "the file that was there is untouched"
    );
    assert_eq!(
        text(&chapter(&d).join("01-Scene-One (1).md")),
        "---\ntitle: \"Scene One\"\n---\n\nA copy.\n",
        "and the copy stays where it was rather than land on it"
    );
    let _ = fs::remove_dir_all(&d);
}

fn text(p: &Path) -> String {
    fs::read_to_string(p).unwrap()
}

/// Undo a settle the way the desk does: texts back, then the moves reversed.
fn undo(d: &Path, s: &project::Settled) {
    for (path, before, _) in &s.texts {
        project::write_atomic(path, before).unwrap();
    }
    let back: Vec<(PathBuf, PathBuf)> = s
        .moves
        .iter()
        .rev()
        .map(|(f, t)| (t.clone(), f.clone()))
        .collect();
    project::apply_moves(d, &back, s.links).unwrap();
}

#[test]
fn taking_the_copy_keeps_the_scenes_words_in_history_and_the_copy_in_the_trash() {
    let d = book("take");
    let copy = put(
        &d,
        "01-Scene-One (Josh's conflicted copy 2026-09-25).md",
        "Scene One",
        "The laptop's longer version of the crossing.",
    );
    let one = chapter(&d).join("01-Scene-One.md");
    let before = text(&one);
    let s = project::settle(&d, &copy, Settle::TakeCopy, None).unwrap();
    assert!(text(&one).contains("laptop's longer version"));
    assert!(!copy.exists(), "the copy is out of the book");
    assert!(
        s.moves[0].1.starts_with(d.join(".grimoire/trash")),
        "to the trash"
    );
    let kept = grimoire_core::history::versions(&d, &one);
    assert!(
        kept.iter().any(|v| v.text == before),
        "the scene's own words are in its history"
    );
    undo(&d, &s);
    assert_eq!(text(&one), before);
    assert!(copy.exists(), "and the copy is back");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn taking_the_copy_refuses_when_the_scene_changed_meanwhile() {
    let d = book("take-guard");
    let copy = put(&d, "01-Scene-One (conflicted).md", "Scene One", "Copy.");
    let one = chapter(&d).join("01-Scene-One.md");
    let seen = sync::Fingerprint::read(&one);
    fs::write(&one, "a third version, just arrived\n").unwrap();
    assert!(project::settle(&d, &copy, Settle::TakeCopy, seen).is_err());
    assert_eq!(text(&one), "a third version, just arrived\n", "untouched");
    assert!(copy.exists(), "and so is the copy");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn keeping_the_original_sends_only_the_copy_to_the_trash() {
    let d = book("keep");
    let copy = put(&d, "01-Scene-One [conflicted].md", "Scene One", "Copy.");
    let one = chapter(&d).join("01-Scene-One.md");
    let before = text(&one);
    let s = project::settle(&d, &copy, Settle::KeepOriginal, None).unwrap();
    assert_eq!(text(&one), before);
    assert!(!copy.exists());
    assert_eq!(
        text(&s.moves[0].1),
        text(&s.moves[0].1),
        "it's in the trash"
    );
    undo(&d, &s);
    assert!(copy.exists());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn keeping_both_makes_the_copy_a_scene_right_after_the_original() {
    let d = book("both");
    let copy = put(
        &d,
        "01-Scene-One (Josh's conflicted copy 2026-09-25).md",
        "Scene One",
        "The laptop's version.",
    );
    let s = project::settle(&d, &copy, Settle::KeepBoth, None).unwrap();
    assert_eq!(
        names(&d),
        [
            "01-Scene-One.md",
            "02-Scene-One-Dropbox-copy.md",
            "03-Scene-Two.md"
        ]
    );
    let p = Project::load(&d).unwrap();
    let kept = node(&p, "02-Scene-One-Dropbox-copy.md");
    assert!(!kept.parked, "an ordinary scene now");
    assert_eq!(kept.title, "Scene One — Dropbox copy");
    assert_eq!(p.total_words(), 5 + 3 + 4, "and it counts");
    undo(&d, &s);
    assert_eq!(
        names(&d),
        [
            "01-Scene-One (Josh's conflicted copy 2026-09-25).md",
            "01-Scene-One.md",
            "02-Scene-Two.md"
        ]
    );
    assert!(
        text(&copy).contains("title: \"Scene One\""),
        "its title back too"
    );
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn a_dropped_copy_goes_to_the_trash_and_a_scene_cannot_be_dropped() {
    let d = book("drop");
    let copy = put(&d, "01-Scene-One (1).md", "Scene One", "Copy.");
    let went = sync::drop_conflict_copy(&copy).unwrap().unwrap();
    assert!(went.starts_with(d.join(".grimoire/trash")));
    assert!(text(&went).contains("Copy."));
    assert!(sync::drop_conflict_copy(&chapter(&d).join("01-Scene-One.md")).is_err());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn another_machines_half_finished_move_is_left_for_that_machine() {
    let d = book("foreign-move");
    // Synced from a laptop mid-move: its name says whose it is.
    let foreign = chapter(&d).join(".grimoire-moving-0-12345~LAPTOP-02-Scene-Two.md");
    fs::write(&foreign, "the laptop's move\n").unwrap();
    let restored = project::restore_stranded(&d);
    assert!(restored.is_empty(), "nothing restored here: {restored:?}");
    assert!(foreign.exists(), "left where the laptop will look for it");
    assert!(
        !chapter(&d).join("02-Scene-Two-recovered.md").exists(),
        "and no duplicate scene appears"
    );
    let _ = fs::remove_dir_all(&d);
}
