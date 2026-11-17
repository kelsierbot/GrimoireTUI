//! Where books are kept, and a book as one file.
//!
//! A book is a folder of plain Markdown files, which is what lets it sync,
//! keep its history and be read without Grimoire. A folder is harder to
//! find, send and put somewhere safe than a file is, so:
//!
//! - new books go in one **books folder** (`~/Documents/Grimoire Books`)
//!   unless the writer says where;
//! - **a copy** is the whole book as one `.zip`, named for the book and the
//!   day, in `Copies` inside the books folder (or wherever it's asked for);
//! - Grimoire **opens a copy**: it's unpacked into the books folder as a
//!   book of its own, never over anything, and the copy is left as it was;
//! - a book kept somewhere else can be **moved** into the books folder.

use anyhow::{Context, Result, bail};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

/// The folder new books go in when nobody says otherwise.
pub fn home() -> PathBuf {
    crate::paths::home()
        .join("Documents")
        .join("Grimoire Books")
}

/// Where copies of books are saved, inside the books folder.
pub fn copies_dir(home: &Path) -> PathBuf {
    home.join("Copies")
}

/// What the very first book on a computer is called, until it's renamed.
pub const FIRST_BOOK: &str = "My First Book";

/// The books in the books folder, the one worked on most recently first.
/// (Brought over from another computer, they're found without being told.)
pub fn books_in(home: &Path) -> Vec<PathBuf> {
    let touched = |root: &Path| {
        [
            root.join(".grimoire/resume.md"),
            root.join("novel.toml"),
            root.join("manuscript"),
        ]
        .iter()
        .filter_map(|p| fs::metadata(p).ok()?.modified().ok())
        .max()
    };
    let mut found: Vec<(Option<std::time::SystemTime>, PathBuf)> = fs::read_dir(home)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_book(p))
        .map(|p| (touched(&p), p))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().map(|(_, p)| p).collect()
}

/// Is this folder a book?
pub fn is_book(p: &Path) -> bool {
    p.join("manuscript").is_dir()
}

/// Does this look like a copy of a book: a `.zip` file?
pub fn is_copy(p: &Path) -> bool {
    p.is_file()
        && p.extension()
            .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("zip"))
}

/// `dir/name`, or `dir/name 2`, `name 3`… when that's taken: something new
/// never lands on something that's there.
pub fn unique(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && e.eq_ignore_ascii_case("zip") => {
            (s.to_string(), format!(".{e}"))
        }
        _ => (name.to_string(), String::new()),
    };
    let mut path = dir.join(name);
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} {n}{ext}"));
        n += 1;
    }
    path
}

/// What a book is on disk, in numbers a person can check.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Facts {
    pub files: usize,
    pub bytes: u64,
}

/// Every file of the book, in a steady order, as (path on disk, path inside
/// the book with `/` between folders). Grimoire's own leftovers that a copy
/// has no use for are left out: temporary files mid-save, and the words
/// recovery is holding (they're in the scenes once they're saved).
fn files_of(root: &Path) -> Result<Vec<(PathBuf, String)>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, String)>) -> Result<()> {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .flatten()
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let inside: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect();
            let name = inside.last().cloned().unwrap_or_default();
            if name.ends_with(".tmp") || name == ".DS_Store" {
                continue;
            }
            if inside.first().map(String::as_str) == Some(".grimoire")
                && inside.get(1).map(String::as_str) == Some("recovery")
            {
                continue;
            }
            // A link is followed only to a file: a folder linked from
            // somewhere else isn't part of the book.
            let kind = e.file_type()?;
            if kind.is_dir() {
                walk(root, &path, out)?;
            } else if path.is_file() {
                out.push((path, inside.join("/")));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

/// How many files the book is and how much room they take.
pub fn facts(root: &Path) -> Facts {
    let mut f = Facts::default();
    for (path, _) in files_of(root).unwrap_or_default() {
        f.files += 1;
        f.bytes += fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    }
    f
}

/// A name for the book that's safe in a file name anywhere, and still reads
/// as its title.
fn file_name_for(title: &str, root: &Path) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = cleaned.trim_matches(['.', ' ']).to_string();
    if cleaned.is_empty() {
        root.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Book".into())
    } else {
        cleaned
    }
}

/// Save the whole book at `root` as one file in `into`: `<Title> <date>.zip`
/// (a number after it if there's one from today already). Inside is the
/// book's own folder, so unpacking it by hand gives the book back too. The
/// file appears only once it's complete. Returns where it is.
pub fn save_copy(
    root: &Path,
    title: &str,
    into: &Path,
    today: chrono::NaiveDate,
) -> Result<PathBuf> {
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;

    if !is_book(root) {
        bail!("{} isn't a book", root.display());
    }
    fs::create_dir_all(into).with_context(|| format!("creating {}", into.display()))?;
    let name = file_name_for(title, root);
    let dest = unique(into, &format!("{name} {}.zip", today.format("%Y-%m-%d")));
    let folder = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| name.clone());

    let files = files_of(root)?;
    let tmp = into.join(format!(
        ".{}.{}.tmp",
        dest.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let result = (|| -> Result<()> {
        let out = fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        let mut z = zip::ZipWriter::new(std::io::BufWriter::new(out));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        for (path, inside) in &files {
            let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
            z.start_file(format!("{folder}/{inside}"), opts)?;
            z.write_all(&bytes)?;
        }
        let mut w = z.finish()?;
        w.flush()?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // Checked before it's given its name: every file is in it.
    let kept = fs::File::open(&tmp)
        .ok()
        .and_then(|f| zip::ZipArchive::new(f).ok())
        .map(|z| z.len());
    if kept != Some(files.len()) {
        let _ = fs::remove_file(&tmp);
        bail!("the copy came out incomplete, so it wasn't kept");
    }
    fs::rename(&tmp, &dest).with_context(|| format!("naming {}", dest.display()))?;
    Ok(dest)
}

/// The newest copy of this book in `dir`, if there is one: its file and
/// when it was made.
pub fn newest_copy(
    dir: &Path,
    title: &str,
    root: &Path,
) -> Option<(PathBuf, std::time::SystemTime)> {
    let lead = format!("{} ", file_name_for(title, root));
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with(&lead) && n.to_lowercase().ends_with(".zip")
        })
        .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
        .max_by_key(|(_, when)| *when)
}

/// The most a copy may unpack to. A book of a million words is a few
/// megabytes; this is only here so a file that isn't a book can't fill a disk.
const MOST: u64 = 4 * 1024 * 1024 * 1024;

/// Open a copy: unpack the book in `copy` into `home` as a book of its own,
/// under its own folder name (numbered if that's taken). Nothing already
/// there is touched, and the copy stays as it was. Returns the new book.
pub fn open_copy(copy: &Path, home: &Path) -> Result<PathBuf> {
    let not_ours = || {
        format!(
            "{} isn't a copy of a Grimoire book",
            copy.file_name().unwrap_or_default().to_string_lossy()
        )
    };
    let file = fs::File::open(copy).with_context(|| format!("opening {}", copy.display()))?;
    let mut z = zip::ZipArchive::new(file).with_context(not_ours)?;

    // Every entry, by a path that can't climb out of where it's put.
    let mut entries: Vec<(usize, PathBuf)> = Vec::new();
    let mut total: u64 = 0;
    for i in 0..z.len() {
        let e = z.by_index(i)?;
        if e.is_dir() {
            continue;
        }
        let Some(path) = e.enclosed_name() else {
            bail!(
                "{}: it holds a file that would land outside the book",
                not_ours()
            );
        };
        if path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            bail!(
                "{}: it holds a file that would land outside the book",
                not_ours()
            );
        }
        total = total.saturating_add(e.size());
        if total > MOST {
            bail!("{}: it's far too big to be a book", not_ours());
        }
        entries.push((i, path));
    }
    // The book is the folder that has `manuscript` in it: the copy's one
    // top folder (as Grimoire makes them), or the copy itself if someone
    // zipped up the inside of a book.
    let top = entries
        .iter()
        .find_map(|(_, p)| {
            let parts: Vec<_> = p.components().collect();
            parts
                .iter()
                .position(|c| c.as_os_str() == "manuscript")
                .filter(|&at| at <= 1 && parts.len() > at + 1)
                .map(|at| parts[..at].iter().collect::<PathBuf>())
        })
        .with_context(not_ours)?;
    let name = match top.file_name() {
        Some(n) => n.to_string_lossy().to_string(),
        None => copy
            .file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Book".into()),
    };
    fs::create_dir_all(home).with_context(|| format!("creating {}", home.display()))?;
    let dest = unique(home, &name);
    // Unpacked beside where it's going, then named: a copy that fails
    // half-way leaves no half a book.
    let staging = home.join(format!(".opening-{}-{}", std::process::id(), name));
    let _ = fs::remove_dir_all(&staging);
    let result = (|| -> Result<()> {
        for (i, path) in &entries {
            let Ok(rel) = path.strip_prefix(&top) else {
                continue;
            };
            let to = staging.join(rel);
            if let Some(dir) = to.parent() {
                fs::create_dir_all(dir)?;
            }
            let mut e = z.by_index(*i)?;
            let mut bytes = Vec::with_capacity(e.size() as usize);
            e.read_to_end(&mut bytes)?;
            fs::write(&to, bytes).with_context(|| format!("writing {}", to.display()))?;
        }
        if !is_book(&staging) {
            bail!(not_ours());
        }
        fs::rename(&staging, &dest).with_context(|| format!("naming {}", dest.display()))?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    Ok(dest)
}

/// Move the book at `root` into `home`, under the same folder name
/// (numbered if that's taken). One rename: the book is in one place or the
/// other, never half in both. A book on another disk can't be moved that
/// way and is left where it is; a copy opened in the books folder does it
/// instead. Returns where the book is now.
pub fn move_to(root: &Path, home: &Path) -> Result<PathBuf> {
    if !is_book(root) {
        bail!("{} isn't a book", root.display());
    }
    let here = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    fs::create_dir_all(home).with_context(|| format!("creating {}", home.display()))?;
    let home_real = fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    if here.parent() == Some(home_real.as_path()) {
        bail!("it's already in your books folder");
    }
    if home_real.starts_with(&here) {
        bail!("the books folder is inside this book, so it can't move there");
    }
    let name = here
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .context("this book's folder has no name")?;
    let dest = unique(&home_real, &name);
    fs::rename(&here, &dest).map_err(|e| {
        anyhow::anyhow!(
            "it couldn't be moved in one step ({e}), so it's left where it is. \
             Save a copy and open the copy instead: that puts it in your books folder"
        )
    })?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grimoire-shelf-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn book(dir: &Path, name: &str) -> PathBuf {
        let root = dir.join(name);
        let ch = root.join("manuscript/01-Act-One/01-Chapter-One");
        fs::create_dir_all(&ch).unwrap();
        fs::write(root.join("novel.toml"), "title = \"Ash & Ember: A Tale\"\n").unwrap();
        fs::write(
            ch.join("01-Gravel.md"),
            "---\ntitle: Gravel\n---\n\nThe rain came sideways.\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("characters")).unwrap();
        fs::write(
            root.join("characters/01-Wren.md"),
            "Wren keeps the ledgers.\n",
        )
        .unwrap();
        fs::create_dir_all(root.join(".grimoire/recovery")).unwrap();
        fs::write(root.join(".grimoire/recovery/held.md"), "unsaved").unwrap();
        fs::write(root.join(".grimoire/book-id"), "abc123\n").unwrap();
        root
    }

    fn day() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 11, 16).unwrap()
    }

    #[test]
    fn a_copy_is_one_file_and_opens_as_the_same_book() {
        let d = tmp("round");
        let root = book(&d, "Ash and Ember");
        let home = d.join("Grimoire Books");
        let copy = save_copy(&root, "Ash & Ember: A Tale", &copies_dir(&home), day()).unwrap();
        // Named for the book and the day, with nothing a file name can't hold.
        assert_eq!(
            copy.file_name().unwrap().to_string_lossy(),
            "Ash & Ember A Tale 2026-11-16.zip"
        );
        assert!(is_copy(&copy));
        // No temporary file is left beside it.
        assert_eq!(fs::read_dir(copies_dir(&home)).unwrap().count(), 1);

        let opened = open_copy(&copy, &home).unwrap();
        assert_eq!(opened, home.join("Ash and Ember"));
        assert!(is_book(&opened));
        let scene = "manuscript/01-Act-One/01-Chapter-One/01-Gravel.md";
        assert_eq!(
            fs::read_to_string(opened.join(scene)).unwrap(),
            fs::read_to_string(root.join(scene)).unwrap()
        );
        assert!(opened.join("characters/01-Wren.md").is_file());
        assert!(
            opened.join(".grimoire/book-id").is_file(),
            "its history id comes too"
        );
        assert!(
            !opened.join(".grimoire/recovery").exists(),
            "words held for recovery aren't part of a copy"
        );
        // The copy is still there, and the original is untouched.
        assert!(copy.is_file() && root.join(scene).is_file());
    }

    #[test]
    fn nothing_is_ever_written_over() {
        let d = tmp("unique");
        let root = book(&d, "Ash and Ember");
        let home = d.join("Grimoire Books");
        let first = save_copy(&root, "Ash", &copies_dir(&home), day()).unwrap();
        let second = save_copy(&root, "Ash", &copies_dir(&home), day()).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            second.file_name().unwrap().to_string_lossy(),
            "Ash 2026-11-16 2.zip"
        );
        // Opened twice: two books, the second numbered.
        let a = open_copy(&first, &home).unwrap();
        fs::write(a.join("novel.toml"), "title = \"changed\"\n").unwrap();
        let b = open_copy(&first, &home).unwrap();
        assert_eq!(b, home.join("Ash and Ember 2"));
        assert_eq!(
            fs::read_to_string(a.join("novel.toml")).unwrap(),
            "title = \"changed\"\n"
        );
        // The newest copy is found by the book's name.
        assert_eq!(
            newest_copy(&copies_dir(&home), "Ash", &root).map(|c| c.0.is_file()),
            Some(true)
        );
        assert!(newest_copy(&copies_dir(&home), "Another Book", &root).is_none());
    }

    #[test]
    fn a_zip_that_is_not_a_book_is_refused_and_leaves_nothing() {
        let d = tmp("refuse");
        let home = d.join("Grimoire Books");
        // Not a zip at all.
        let junk = d.join("notes.zip");
        fs::write(&junk, "not a zip").unwrap();
        assert!(open_copy(&junk, &home).is_err());
        // A zip with no manuscript in it.
        let other = d.join("photos.zip");
        {
            let mut z = zip::ZipWriter::new(fs::File::create(&other).unwrap());
            z.start_file(
                "holiday/readme.txt",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            z.write_all(b"hello").unwrap();
            z.finish().unwrap();
        }
        let err = open_copy(&other, &home).unwrap_err().to_string();
        assert!(err.contains("isn't a copy of a Grimoire book"), "{err}");
        let left: Vec<_> = fs::read_dir(&home)
            .map(|r| r.flatten().collect())
            .unwrap_or_default();
        assert!(left.is_empty(), "nothing half-made is left: {left:?}");
    }

    #[test]
    fn the_inside_of_a_book_zipped_by_hand_opens_too() {
        let d = tmp("inside");
        let home = d.join("Grimoire Books");
        let hand = d.join("My Novel.zip");
        {
            let mut z = zip::ZipWriter::new(fs::File::create(&hand).unwrap());
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("manuscript/01-One/01-Scene.md", o).unwrap();
            z.write_all(b"Once.").unwrap();
            z.start_file("novel.toml", o).unwrap();
            z.write_all(b"title = \"My Novel\"\n").unwrap();
            z.finish().unwrap();
        }
        let opened = open_copy(&hand, &home).unwrap();
        assert_eq!(opened, home.join("My Novel"));
        assert!(opened.join("manuscript/01-One/01-Scene.md").is_file());
    }

    #[test]
    fn the_books_folder_lists_its_books_and_nothing_else() {
        let d = tmp("list");
        let home = d.join("Grimoire Books");
        assert!(books_in(&home).is_empty(), "no folder yet is no books");
        book(&home, "Ash and Ember");
        book(&home, "The Long Night");
        fs::create_dir_all(copies_dir(&home)).unwrap();
        fs::write(home.join("notes.txt"), "not a book").unwrap();
        let mut names: Vec<String> = books_in(&home)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["Ash and Ember", "The Long Night"]);
    }

    #[test]
    fn a_book_moves_into_the_books_folder_whole() {
        let d = tmp("move");
        let root = book(&d.join("code/project"), "Ash and Ember");
        let home = d.join("Grimoire Books");
        let before = facts(&root);
        assert_eq!(before.files, 4, "recovery's held words aren't counted");
        let now = move_to(&root, &home).unwrap();
        assert_eq!(now.file_name().unwrap(), "Ash and Ember");
        assert!(now.starts_with(fs::canonicalize(&home).unwrap()));
        assert!(!root.exists(), "it's in one place, not two");
        assert_eq!(facts(&now), before);
        // Already there: said, and nothing moves.
        let err = move_to(&now, &home).unwrap_err().to_string();
        assert!(err.contains("already in your books folder"), "{err}");
        // A second book of the same name lands beside it, numbered.
        let twin = book(&d.join("elsewhere"), "Ash and Ember");
        let moved = move_to(&twin, &home).unwrap();
        assert_eq!(moved.file_name().unwrap(), "Ash and Ember 2");
    }
}
