//! Names and structure that survive every sync client: Dropbox, Google Drive,
//! pCloud, Box, OneDrive and iCloud, opened on Linux, a Mac and Windows.
//!
//! Each test sets up what a client or another machine would leave on disk and
//! checks Grimoire neither makes a name that breaks elsewhere nor loses track
//! of a scene because of how its name is spelled.

use grimoire_core::project::{self, Project};
use grimoire_core::{history, names};
use std::fs;
use std::path::{Path, PathBuf};

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("grimoire-sync-names-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn book(tag: &str) -> PathBuf {
    let d = temp(tag);
    project::scaffold(&d).unwrap();
    d
}

fn name(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().to_string()
}

const WINDOWS_FORBIDDEN: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

fn safe_everywhere(file: &str) {
    assert!(
        !file
            .chars()
            .any(|c| WINDOWS_FORBIDDEN.contains(&c) || c.is_control()),
        "{file}"
    );
    assert!(
        !file.ends_with('.') && !file.ends_with(' ') && !file.starts_with(' '),
        "{file}"
    );
    assert!(!names::is_reserved(file), "{file}");
    assert!(file.len() <= 255, "{} bytes", file.len());
}

#[test]
fn new_items_are_named_so_every_client_takes_them() {
    let d = temp("create");
    for title in [
        r#"what? a:b* <c> |d| "e". "#,
        "Con",
        "  trailing dots... ",
        "tab\tand\u{7}bell",
        "Rose\u{301} by the sea",
        &"a very long title ".repeat(30),
        "書".repeat(120).as_str(),
    ] {
        let p = project::create(&d, title, false).unwrap();
        safe_everywhere(&name(&p));
        assert!(
            p.to_string_lossy().chars().count() <= names::PATH_MAX,
            "{}",
            p.display()
        );
        let folder = project::create(&d, title, true).unwrap();
        safe_everywhere(&name(&folder));
    }
    // The title is kept as typed in the scene itself; only the name is tidied.
    let p = project::create(&d, "Rose\u{301}", false).unwrap();
    assert!(
        name(&p).ends_with("-Ros\u{e9}.md"),
        "composed: {}",
        name(&p)
    );
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn renaming_a_note_to_a_neighbours_name_in_other_capitals_is_refused() {
    let d = temp("rename-case");
    let notes = d.join("characters");
    fs::create_dir_all(&notes).unwrap();
    fs::write(notes.join("Wren.md"), "---\ntitle: Wren\n---\nA courier.\n").unwrap();
    fs::write(
        notes.join("Oren.md"),
        "---\ntitle: Oren\n---\nThe archivist.\n",
    )
    .unwrap();
    let err = project::rename(&notes.join("Oren.md"), "wren").unwrap_err();
    assert!(err.to_string().contains("clash"), "{err}");
    assert!(notes.join("Oren.md").exists(), "nothing moved");
    assert!(!notes.join("wren.md").exists());
    // Accents too: Box treats Café and Cafe as one file.
    fs::write(notes.join("Café.md"), "x\n").unwrap();
    let err = project::rename(&notes.join("Oren.md"), "Cafe").unwrap_err();
    assert!(err.to_string().contains("clash"), "{err}");
    // A note may change the capitals of its own name.
    let p = project::rename(&notes.join("Wren.md"), "WREN").unwrap();
    assert_eq!(name(&p), "WREN.md");
    assert!(fs::read_to_string(&p).unwrap().contains("A courier."));
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn renaming_to_a_reserved_or_unsafe_name_makes_a_safe_one() {
    let d = temp("rename-reserved");
    let notes = d.join("places");
    fs::create_dir_all(&notes).unwrap();
    fs::write(
        notes.join("Harbour.md"),
        "---\ntitle: Harbour\n---\nSalt.\n",
    )
    .unwrap();
    let p = project::rename(&notes.join("Harbour.md"), "Con").unwrap();
    safe_everywhere(&name(&p));
    assert!(
        fs::read_to_string(&p).unwrap().contains("title: \"Con\""),
        "title kept as typed"
    );
    let p = project::rename(&p, "the: harbour?").unwrap();
    safe_everywhere(&name(&p));
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_move_that_would_land_beside_a_case_twin_is_refused() {
    let d = temp("move-case");
    let a = d.join("a");
    let b = d.join("b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("Wren.md"), "mine\n").unwrap();
    fs::write(b.join("wren.md"), "theirs\n").unwrap();
    let err =
        project::apply_moves(&d, &[(a.join("Wren.md"), b.join("Wren.md"))], false).unwrap_err();
    assert!(err.to_string().contains("clash"), "{err}");
    assert_eq!(fs::read_to_string(a.join("Wren.md")).unwrap(), "mine\n");
    assert_eq!(fs::read_to_string(b.join("wren.md")).unwrap(), "theirs\n");
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn twins_already_on_disk_both_load_and_are_flagged() {
    let d = book("twins");
    let ch = d.join("manuscript/01-Part-One/01-Chapter-One");
    fs::write(
        ch.join("01-scene-one.md"),
        "---\ntitle: \"Scene One\"\n---\n\nfrom a Linux box\n",
    )
    .unwrap();
    fs::write(
        ch.join("01-Scene-Two.md"),
        "---\ntitle: \"Scene Two\"\n---\n\nplain\n",
    )
    .unwrap();
    let p = Project::load(&d).unwrap();
    let flagged: Vec<String> = p
        .nodes
        .iter()
        .filter(|n| n.clash)
        .map(|n| name(&n.path))
        .collect();
    assert_eq!(flagged.len(), 2, "{flagged:?}");
    assert!(flagged.contains(&"01-Scene-One.md".to_string()));
    assert!(flagged.contains(&"01-scene-one.md".to_string()));
    // Nothing was renamed behind the writer's back.
    assert!(ch.join("01-scene-one.md").exists() && ch.join("01-Scene-One.md").exists());
    fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_scene_a_mac_spelled_in_decomposed_form_keeps_its_history() {
    let d = temp("nfd");
    let composed = d.join("01-Ros\u{e9}.md");
    let decomposed = d.join("01-Rose\u{301}.md");
    // History written on Linux under the composed spelling…
    history::snapshot(&d, &composed, "the first draft\n", None).unwrap();
    // …and the file as a Mac handed it over, decomposed.
    fs::write(&decomposed, "the first draft\nand more\n").unwrap();
    let v = history::versions(&d, &decomposed);
    assert_eq!(
        v.len(),
        1,
        "the scene's history is found whichever way it's spelled"
    );
    assert!(names::same_path(&composed, &decomposed));
    assert_eq!(
        names::resolve(&composed).as_deref(),
        Some(decomposed.as_path())
    );
    fs::remove_dir_all(&d).unwrap();
}
