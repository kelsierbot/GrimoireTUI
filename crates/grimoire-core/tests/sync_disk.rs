//! What sync clients do to a book on disk, and what Grimoire must do back.
//!
//! Each test stages one real behaviour of pCloud, Dropbox, Google Drive, Box,
//! OneDrive or iCloud on a temporary book and checks that nobody's words are
//! lost, overwritten or taken for something they're not. An online-only file
//! that isn't downloaded is staged as one that can't be read (`chmod 000`):
//! the same error an app gets from a dataless placeholder with no connection.

#![cfg(unix)]

use grimoire_core::project::{self, DiskChange, Project, trash_dir};
use grimoire_core::{atomic, manuscript, sync};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A small book: one chapter with two scenes. Returns (root, scene one, scene two).
fn book(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("grimoire-syncdisk-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let ch = root.join("manuscript/01-Part-One/01-Chapter-One");
    fs::create_dir_all(&ch).unwrap();
    fs::write(
        root.join("novel.toml"),
        "title = \"Salt\"\nauthor = \"A. Writer\"\n",
    )
    .unwrap();
    let one = ch.join("01-Scene-One.md");
    let two = ch.join("02-Scene-Two.md");
    fs::write(
        &one,
        "---\ntitle: \"Scene One\"\n---\n\nThe rain had stopped.\n",
    )
    .unwrap();
    fs::write(
        &two,
        "---\ntitle: \"Scene Two\"\n---\n\nThe third floor smelled of the sea.\n",
    )
    .unwrap();
    // Old enough that its size and time are trusted, as a file synced an
    // hour ago would be.
    age(&one);
    age(&two);
    (root, one, two)
}

fn age(path: &Path) {
    let an_hour_ago = SystemTime::now() - Duration::from_secs(3600);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(an_hour_ago)
        .unwrap();
}

fn lock(path: &Path) {
    fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
}

fn unlock(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn idx(p: &Project, path: &Path) -> usize {
    p.nodes
        .iter()
        .position(|n| n.path == path)
        .expect("in the tree")
}

/// Every conflict copy Grimoire has written anywhere in the book.
fn copies(root: &Path) -> Vec<PathBuf> {
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        for e in fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if sync::is_conflict_copy(&p) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

fn done(root: &Path) {
    // Put permissions back so the temp book can be removed.
    fn open_up(d: &Path) {
        let _ = fs::set_permissions(d, fs::Permissions::from_mode(0o755));
        for e in fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                open_up(&p);
            } else {
                let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o644));
            }
        }
    }
    open_up(root);
    let _ = fs::remove_dir_all(root);
}

// ---- files that can't be read -------------------------------------------

/// An online-only scene with no connection: the book still opens, the scene
/// is a row that says so, it is never written, and it's read properly once
/// it downloads.
#[test]
fn an_unreadable_scene_never_stops_the_book_opening_and_is_never_written() {
    let (root, one, two) = book("unreadable");
    lock(&two);
    let mut p = Project::load(&root).expect("the book opens");
    let i = idx(&p, &two);
    let n = &p.nodes[i];
    assert!(n.never_read() && n.read_only);
    assert_eq!(n.disk.unavailable.as_ref().unwrap().tag, "can't read");
    assert_eq!(n.title, "Scene Two", "titled from its filename");
    assert_eq!(p.total_words(), 4, "counted as nothing, not as empty words");

    // Nothing reaches it: not a save, not a compile with a hole in it.
    p.nodes[i].dirty = true;
    let _ = p.save_dirty();
    assert!(
        manuscript::compile(&p).is_err(),
        "no book with a hole in it"
    );
    unlock(&two, 0o644);
    assert!(
        fs::read_to_string(&two)
            .unwrap()
            .contains("smelled of the sea"),
        "written over"
    );

    // Downloaded: a look to see it's settled, then it's the scene.
    assert_eq!(p.check_disk(i, false).unwrap(), DiskChange::Same);
    assert_eq!(p.check_disk(i, false).unwrap(), DiskChange::Available);
    assert!(p.nodes[i].body.contains("smelled of the sea"));
    assert!(p.nodes[i].disk.unavailable.is_none() && !p.nodes[i].read_only);
    let _ = one;
    done(&root);
}

#[test]
fn an_unreadable_folder_is_a_row_not_a_failure() {
    let (root, one, _) = book("unreadable-dir");
    let ch = one.parent().unwrap().to_path_buf();
    lock(&ch);
    let p = Project::load(&root).expect("the book opens");
    let c = idx(&p, &ch);
    assert!(p.nodes[c].disk.unavailable.is_some());
    assert!(p.nodes[c].children.is_empty());
    assert!(!p.tree_is_stale(), "no endless re-reading");
    done(&root);
}

/// Dropbox and Google Drive show an online-only file as empty. Empty where
/// there were words is never taken in, and never written over.
#[test]
fn an_empty_placeholder_is_not_taken_for_a_scene_cut_to_nothing() {
    let (root, one, _) = book("empty");
    let mut p = Project::load(&root).unwrap();
    let i = idx(&p, &one);
    fs::write(&one, "").unwrap();
    let DiskChange::Unavailable(why) = p.check_disk(i, false).unwrap() else {
        panic!("an empty file where there were words should wait");
    };
    assert_eq!(why.tag, "downloading");
    assert!(
        p.nodes[i].body.contains("The rain had stopped"),
        "words kept"
    );

    // Typed into meanwhile: kept, not written over the placeholder.
    p.nodes[i].body.push_str("More.\n");
    p.nodes[i].dirty = true;
    let report = p.save_dirty();
    assert_eq!(report.waiting, vec![i]);
    assert_eq!(fs::read_to_string(&one).unwrap(), "", "never written over");
    assert!(copies(&root).is_empty());

    // It downloads — with an edit made elsewhere. Both versions survive.
    fs::write(
        &one,
        "---\ntitle: \"Scene One\"\n---\n\nThe rain had stopped, then started.\n",
    )
    .unwrap();
    assert_eq!(
        p.check_disk(i, false).unwrap(),
        DiskChange::Same,
        "settling"
    );
    let DiskChange::Parked(copy) = p.check_disk(i, false).unwrap() else {
        panic!("the download and the unsaved words should both be kept");
    };
    assert!(p.nodes[i].body.contains("then started"));
    assert!(fs::read_to_string(copy).unwrap().contains("More."));
    done(&root);
}

/// A scene whose file turns unreadable while it has unsaved words keeps them
/// — once. The old code parked a new copy on every look.
#[test]
fn an_unreadable_changed_scene_does_not_spawn_a_copy_every_look() {
    let (root, one, _) = book("spam");
    let mut p = Project::load(&root).unwrap();
    let i = idx(&p, &one);
    p.nodes[i].body = "Mine, unsaved.\n".into();
    p.nodes[i].dirty = true;
    fs::write(&one, "---\ntitle: \"Scene One\"\n---\n\nTheirs.\n").unwrap();
    lock(&one);
    for _ in 0..3 {
        let _ = p.check_disk(i, false);
        let _ = p.save_dirty();
    }
    assert!(copies(&root).is_empty(), "{:?}", copies(&root));
    assert!(p.nodes[i].dirty, "the words are still here, unsaved");
    unlock(&one, 0o644);
    let _ = p.check_disk(i, false);
    let _ = p.check_disk(i, false);
    assert_eq!(copies(&root).len(), 1, "one copy, once it could be read");
    done(&root);
}

/// A guarded save won't write a file it can't read to check.
#[test]
fn a_guarded_save_refuses_a_file_it_cannot_check() {
    let (root, one, _) = book("guard");
    let seen = sync::Fingerprint::read(&one);
    lock(&one);
    assert!(sync::save_guarded(&one, "over the top\n", seen).is_err());
    unlock(&one, 0o644);
    assert!(
        fs::read_to_string(&one)
            .unwrap()
            .contains("The rain had stopped")
    );
    done(&root);
}

/// An unreadable novel.toml isn't an empty one: the upgrade that writes
/// `part_label` into it must not replace the title and targets.
#[test]
fn an_unreadable_novel_toml_is_never_rewritten() {
    let (root, _, _) = book("toml");
    // A book in pages with no label is one the upgrade writes into.
    fs::rename(
        root.join("manuscript/01-Part-One"),
        root.join("manuscript/01-Page-One"),
    )
    .unwrap();
    let toml = root.join("novel.toml");
    let before = fs::read_to_string(&toml).unwrap();
    lock(&toml);
    let _ = project::upgrade(&root);
    let p = Project::load(&root).expect("the book opens on the defaults");
    assert!(p.meta_unreadable.is_some());
    unlock(&toml, 0o644);
    assert_eq!(fs::read_to_string(&toml).unwrap(), before);
    done(&root);
}

// ---- files that blink, arrive in pieces, or move under you --------------

/// A client that replaces a file by deleting and recreating it (or a folder
/// renamed away and back) isn't deleting the scene.
#[test]
fn a_file_deleted_and_recreated_between_looks_is_not_sent_to_the_trash() {
    let (root, one, _) = book("blink");
    let mut p = Project::load(&root).unwrap();
    let i = idx(&p, &one);
    p.nodes[i].body = "Being typed.\n".into();
    p.nodes[i].dirty = true;
    let back = fs::read(&one).unwrap();
    fs::remove_file(&one).unwrap();
    assert_eq!(
        p.check_disk(i, false).unwrap(),
        DiskChange::Same,
        "one miss isn't gone"
    );
    let report = p.save_dirty();
    assert!(report.gone.is_empty() && report.waiting == vec![i]);
    fs::write(&one, back).unwrap();
    age(&one);
    assert_eq!(p.check_disk(i, false).unwrap(), DiskChange::Same);
    let trash = trash_dir(&root);
    let trashed = fs::read_dir(&trash).map(|r| r.count()).unwrap_or(0);
    assert_eq!(trashed, 0, "nothing went to the trash");
    assert!(
        p.nodes[i].dirty,
        "the typing is still unsaved here, not moved away"
    );
    assert!(p.save_dirty().saved.contains(&i));
    assert!(fs::read_to_string(&one).unwrap().contains("Being typed."));
    done(&root);
}

/// Deleted for real: missing on two looks, and the unsaved words are kept in
/// the trash rather than written back to a file someone removed.
#[test]
fn a_file_missing_on_two_looks_is_gone_and_its_words_go_to_the_trash() {
    let (root, one, _) = book("gone");
    let mut p = Project::load(&root).unwrap();
    let i = idx(&p, &one);
    p.nodes[i].body = "Typed after it went.\n".into();
    p.nodes[i].dirty = true;
    fs::remove_file(&one).unwrap();
    assert_eq!(p.check_disk(i, false).unwrap(), DiskChange::Same);
    let DiskChange::GoneParked(copy) = p.check_disk(i, false).unwrap() else {
        panic!("a second miss is gone");
    };
    assert!(copy.starts_with(trash_dir(&root)));
    assert!(
        fs::read_to_string(copy)
            .unwrap()
            .contains("Typed after it went.")
    );
    assert!(!one.exists(), "not brought back");
    done(&root);
}

/// iCloud (before macOS 14) replaces a file it hasn't downloaded with a
/// hidden `.Name.md.icloud`. That's a scene waiting, not a scene deleted.
#[test]
fn an_icloud_stub_is_a_scene_not_downloaded_not_a_deletion() {
    let (root, one, _) = book("icloud");
    let mut p = Project::load(&root).unwrap();
    let i = idx(&p, &one);
    let stub = project::icloud_stub(&one);
    fs::rename(&one, &stub).unwrap();
    for _ in 0..3 {
        let change = p.check_disk(i, false).unwrap();
        assert!(
            !matches!(change, DiskChange::Gone | DiskChange::GoneParked(_)),
            "{change:?}"
        );
    }
    assert_eq!(
        p.nodes[i].disk.unavailable.as_ref().unwrap().tag,
        "in iCloud"
    );
    assert!(
        p.nodes[i].body.contains("The rain had stopped"),
        "words kept"
    );

    // Opened fresh, the stub is still the scene's row, and its number is taken.
    let p = Project::load(&root).unwrap();
    let j = idx(&p, &one);
    assert_eq!(
        p.nodes[j].disk.unavailable.as_ref().unwrap().tag,
        "in iCloud"
    );
    assert!(!p.tree_is_stale());
    let made = project::create(one.parent().unwrap(), "Scene Three", false).unwrap();
    assert!(
        made.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("03-"),
        "{}",
        made.display()
    );
    done(&root);
}

/// A client writing a file in place is caught halfway: the half isn't taken
/// in, the whole is, once it stops changing.
#[test]
fn a_half_written_file_is_taken_in_only_once_it_settles() {
    let (root, one, _) = book("half");
    let mut p = Project::load(&root).unwrap();
    let i = idx(&p, &one);
    let whole = "---\ntitle: \"Scene One\"\n---\n\nThe rain had stopped an hour before Wren reached the archive.\n";
    fs::write(&one, &whole[..30]).unwrap();
    assert_eq!(
        p.check_disk(i, false).unwrap(),
        DiskChange::Same,
        "half a file isn't taken in"
    );
    assert!(
        p.nodes[i].body.contains("The rain had stopped."),
        "the old words stay"
    );
    fs::write(&one, whole).unwrap();
    assert_eq!(
        p.check_disk(i, false).unwrap(),
        DiskChange::Same,
        "still arriving"
    );
    assert_eq!(p.check_disk(i, false).unwrap(), DiskChange::Adopted);
    assert!(p.nodes[i].body.contains("before Wren reached"));
    done(&root);
}

/// pCloud keeps whole-second times. An edit the same size as the file, in
/// the same second it was last looked at, leaves size and time as they were
/// — and must still be noticed.
#[test]
fn a_same_size_edit_in_the_same_second_is_still_noticed() {
    let (root, one, _) = book("racy");
    let now = SystemTime::now();
    let set = |t: SystemTime| {
        fs::File::options()
            .write(true)
            .open(&one)
            .unwrap()
            .set_modified(t)
            .unwrap()
    };
    set(now);
    let mut p = Project::load(&root).unwrap();
    let i = idx(&p, &one);
    // Same length, one word different, and the same modification time.
    let text = fs::read_to_string(&one)
        .unwrap()
        .replace("stopped", "started");
    fs::write(&one, text).unwrap();
    set(now);
    let _ = p.check_disk(i, false);
    let _ = p.check_disk(i, false);
    assert!(
        p.nodes[i].body.contains("started"),
        "the edit went unnoticed"
    );
    done(&root);
}

// ---- writing ------------------------------------------------------------

/// Saves go through a hidden `.~lock.<name>.<pid>.tmp` sibling that every
/// sync client leaves alone, renamed over the scene — and nothing is left
/// behind.
#[test]
fn a_save_leaves_no_temporary_files_and_uses_a_name_clients_ignore() {
    let (root, one, _) = book("temp");
    let mut p = Project::load(&root).unwrap();
    let i = idx(&p, &one);
    p.nodes[i].body = "Rewritten.\n".into();
    p.nodes[i].dirty = true;
    assert!(p.save_dirty().saved.contains(&i));
    let dir = one.parent().unwrap();
    let names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(names.iter().all(|n| !n.starts_with('.')), "{names:?}");
    let tmp = atomic::temp_path(&one);
    let t = tmp.file_name().unwrap().to_string_lossy();
    assert!(t.starts_with(".~lock.") && t.ends_with(".tmp"), "{t}");
    done(&root);
}
