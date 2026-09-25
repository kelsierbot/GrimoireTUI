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

fn file_for(root: &Path, scene: &Path) -> PathBuf {
    let rel = scene.strip_prefix(root).unwrap_or(scene);
    dir(root).join(rel)
}

/// Keep this scene's text safe until it can be saved for real.
pub fn keep(root: &Path, scene: &Path, text: &str) -> Result<()> {
    let file = file_for(root, scene);
    if let Some(d) = file.parent() {
        fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    project::write_atomic(&file, text)
}

/// The scene saved properly, so any copy kept for it is out of date.
pub fn clear(root: &Path, scene: &Path) {
    let file = file_for(root, scene);
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
/// just deleted; one whose scene no longer exists is left alone on disk.
pub fn pending(p: &Project) -> Vec<Pending> {
    let base = dir(&p.root);
    let mut files = Vec::new();
    collect(&base, &mut files);
    files.sort();
    let mut out = Vec::new();
    for file in files {
        let Ok(rel) = file.strip_prefix(&base) else { continue };
        let scene = p.root.join(rel);
        let Some(node) = p.nodes.iter().find(|n| n.kind == Kind::Scene && n.path == scene) else {
            continue;
        };
        let Ok(text) = fs::read_to_string(&file) else { continue };
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
            recovered_words: body.split_whitespace().count(),
            text,
        });
    }
    out
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
        let d = std::env::temp_dir().join(format!("grimoire-recovery-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("manuscript/01-Act-One")).unwrap();
        fs::write(d.join("manuscript/01-Act-One/01-Gravel.md"), "---\ntitle: Gravel\n---\n\nThe lot was empty.\n").unwrap();
        d
    }

    #[test]
    fn a_kept_copy_that_differs_is_offered_and_an_identical_one_is_dropped() {
        let d = book("offer");
        let p = Project::load(&d).unwrap();
        let scene = d.join("manuscript/01-Act-One/01-Gravel.md");

        keep(&d, &scene, "---\ntitle: Gravel\n---\n\nThe lot was empty except for the truck.\n").unwrap();
        let found = pending(&p);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Gravel");
        assert_eq!((found[0].saved_words, found[0].recovered_words), (4, 8));

        // Saved for real: the kept copy now matches and quietly goes away.
        keep(&d, &scene, &p.nodes.iter().find(|n| n.path == scene).unwrap().file_text()).unwrap();
        assert!(pending(&p).is_empty());
        assert!(!file_for(&d, &scene).exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn clearing_removes_the_copy() {
        let d = book("clear");
        let scene = d.join("manuscript/01-Act-One/01-Gravel.md");
        keep(&d, &scene, "anything").unwrap();
        assert!(file_for(&d, &scene).exists());
        clear(&d, &scene);
        assert!(!file_for(&d, &scene).exists());
        fs::remove_dir_all(&d).unwrap();
    }
}
