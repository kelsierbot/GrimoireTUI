//! Words that couldn't be saved the normal way.
//!
//! Autosave writes a scene a couple of seconds after you stop typing, so most
//! of the time there is nothing here. This is for the rest: a save that failed
//! (a full disk, a file another program has locked), a crash, a terminal closed
//! while a save was refused. The scene's full text goes to
//! `.grimoire/recovery/<same path as the scene>`, and the next launch offers it
//! back beside what's on disk.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::project::{self, Kind, Project};

pub fn dir(root: &Path) -> PathBuf {
    root.join(".grimoire").join("recovery")
}

/// Where a scene's copy is kept. `None` for a scene outside the book, which
/// has no place in the book's recovery folder.
fn file_for(root: &Path, scene: &Path) -> Option<PathBuf> {
    let rel = scene.strip_prefix(root).ok()?;
    Some(dir(root).join(rel))
}

/// Keep this scene's text safe until it can be saved for real.
pub fn keep(root: &Path, scene: &Path, text: &str) -> Result<()> {
    let file = file_for(root, scene)
        .with_context(|| format!("{} isn't inside this book", scene.display()))?;
    if let Some(d) = file.parent() {
        fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    project::write_atomic(&file, text)
}

/// The scene saved properly, so any copy kept for it is out of date.
pub fn clear(root: &Path, scene: &Path) {
    let Some(file) = file_for(root, scene) else {
        return;
    };
    if file.exists() {
        let _ = fs::remove_file(&file);
    }
}

/// A recovered copy that differs from what's saved.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub scene: PathBuf,
    pub title: String,
    pub file: PathBuf,
    pub text: String,
    pub saved_words: usize,
    pub recovered_words: usize,
    pub when: Option<SystemTime>,
}

/// Recovered copies worth offering. A copy identical to the saved scene is
/// just deleted. One whose scene no longer exists (moved or deleted since) is
/// offered too, with nothing saved to set beside it: [`keep_orphan`] puts it
/// somewhere safe.
pub fn pending(p: &Project) -> Vec<Pending> {
    let base = dir(&p.root);
    let mut files = Vec::new();
    collect(&base, &mut files);
    files.sort();
    let mut out = Vec::new();
    for file in files {
        let Ok(rel) = file.strip_prefix(&base) else {
            continue;
        };
        let scene = p.root.join(rel);
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let Some(node) = p
            .nodes
            .iter()
            .find(|n| n.kind == Kind::Scene && n.path == scene)
        else {
            if scene.symlink_metadata().is_err() {
                out.push(orphan(scene, file, text));
            }
            continue;
        };
        if text == node.file_text() {
            let _ = fs::remove_file(&file);
            continue;
        }
        let (_, body) = project::split_frontmatter(&text);
        out.push(Pending {
            scene,
            title: node.title.clone(),
            when: fs::metadata(&file).and_then(|m| m.modified()).ok(),
            file,
            saved_words: node.words(),
            recovered_words: crate::notes::count_words(&body),
            text,
        });
    }
    out
}

/// A copy whose scene is gone: titled from its own front matter, or its name.
fn orphan(scene: PathBuf, file: PathBuf, text: String) -> Pending {
    let (front, body) = project::split_frontmatter(&text);
    let title = front
        .as_deref()
        .and_then(|f| project::front_scalar(f, "title"))
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| {
            let stem = scene
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            stem.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-')
                .replace('-', " ")
        });
    Pending {
        scene,
        title,
        when: fs::metadata(&file).and_then(|m| m.modified()).ok(),
        file,
        saved_words: 0,
        recovered_words: crate::notes::count_words(&body),
        text,
    }
}

/// Keep the words of a copy whose scene is gone: a new file in the Trash,
/// named for the scene with "recovered" on it, never over anything already
/// there. Restoring it from the Trash puts it where the scene was. The
/// recovery copy goes once the words are safely written. Returns the new file.
pub fn keep_orphan(root: &Path, item: &Pending) -> Result<PathBuf> {
    use std::io::Write;
    let trash = project::trash_dir(root);
    fs::create_dir_all(&trash).with_context(|| format!("creating {}", trash.display()))?;
    let stem = item
        .scene
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "scene".into());
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut n = 1;
    let (target, mut f) = loop {
        let count = if n == 1 {
            String::new()
        } else {
            format!("{n}-")
        };
        let target = trash.join(format!("{stamp}-{count}{stem}-recovered.md"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(f) => break (target, f),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e).with_context(|| format!("writing {}", target.display())),
        }
    };
    if let Err(e) = f.write_all(item.text.as_bytes()).and_then(|_| f.sync_all()) {
        drop(f);
        let _ = fs::remove_file(&target);
        return Err(e).with_context(|| format!("writing {}", target.display()));
    }
    project::note_trash_origin(root, &target, &item.scene);
    let _ = fs::remove_file(&item.file);
    Ok(target)
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("grimoire-recovery-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("manuscript/01-Act-One")).unwrap();
        fs::write(
            d.join("manuscript/01-Act-One/01-Gravel.md"),
            "---\ntitle: Gravel\n---\n\nThe lot was empty.\n",
        )
        .unwrap();
        d
    }

    #[test]
    fn a_kept_copy_that_differs_is_offered_and_an_identical_one_is_dropped() {
        let d = book("offer");
        let p = Project::load(&d).unwrap();
        let scene = d.join("manuscript/01-Act-One/01-Gravel.md");

        keep(
            &d,
            &scene,
            "---\ntitle: Gravel\n---\n\nThe lot was empty except for the truck.\n",
        )
        .unwrap();
        let found = pending(&p);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Gravel");
        assert_eq!((found[0].saved_words, found[0].recovered_words), (4, 8));

        // Saved for real: the kept copy now matches and quietly goes away.
        keep(
            &d,
            &scene,
            &p.nodes
                .iter()
                .find(|n| n.path == scene)
                .unwrap()
                .file_text(),
        )
        .unwrap();
        assert!(pending(&p).is_empty());
        assert!(!file_for(&d, &scene).unwrap().exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn words_whose_scene_is_gone_are_offered_and_kept_in_the_trash() {
        let d = book("rv-orphan");
        let gone = d.join("manuscript/01-Act-One/02-Rain.md");
        let text = "---\ntitle: \"Rain, at Last\"\n---\n\nAda Quill ran for the bus.\n";
        keep(&d, &gone, text).unwrap();
        let p = Project::load(&d).unwrap();
        let found = pending(&p);
        assert_eq!(found.len(), 1, "an orphan is still offered");
        let o = &found[0];
        assert_eq!(o.scene, gone);
        assert_eq!(o.title, "Rain, at Last");
        assert_eq!((o.saved_words, o.recovered_words), (0, 6));

        // Something already wears the name it would take: never written over.
        let trash = project::trash_dir(&d);
        let first = keep_orphan(&d, o).unwrap();
        assert!(first.starts_with(&trash), "{}", first.display());
        let name = first.file_name().unwrap().to_string_lossy().to_string();
        assert!(
            name.contains("02-Rain") && name.contains("recovered"),
            "{name}"
        );
        assert_eq!(fs::read_to_string(&first).unwrap(), text);
        assert!(!o.file.exists(), "the recovery copy is done with");
        assert!(pending(&p).is_empty());

        keep(&d, &gone, "---\ntitle: Rain\n---\n\nA second try.\n").unwrap();
        let again = pending(&p);
        let second = keep_orphan(&d, &again[0]).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read_to_string(&first).unwrap(), text, "untouched");
        assert!(
            fs::read_to_string(&second)
                .unwrap()
                .contains("A second try.")
        );
        // And restoring it from the Trash puts it where the scene was.
        let back = project::restore(&d, &first).unwrap();
        assert_eq!(back, gone);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_scene_outside_the_book_never_touches_anything() {
        let d = book("rv-outside");
        let elsewhere = std::env::temp_dir().join(format!(
            "grimoire-recovery-rv-elsewhere-{}.md",
            std::process::id()
        ));
        fs::write(&elsewhere, "Ada's own file.").unwrap();
        assert!(keep(&d, &elsewhere, "other words").is_err());
        clear(&d, &elsewhere);
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "Ada's own file.");
        fs::remove_file(&elsewhere).unwrap();
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn clearing_removes_the_copy() {
        let d = book("clear");
        let scene = d.join("manuscript/01-Act-One/01-Gravel.md");
        keep(&d, &scene, "anything").unwrap();
        assert!(file_for(&d, &scene).unwrap().exists());
        clear(&d, &scene);
        assert!(!file_for(&d, &scene).unwrap().exists());
        fs::remove_dir_all(&d).unwrap();
    }
}
