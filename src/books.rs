//! Which books this machine has had open: the last one, which `grimoire`
//! with no arguments opens, and a few before it for *Open another book…*.
//! `~/.config/grimoire/state.toml`, one `key = "value"` per line.

use grimoire_core::paths::home;
use std::path::{Path, PathBuf};

/// How many books *Open another book…* remembers.
const KEEP: usize = 8;

/// Where a first-time book goes if the writer never names one.
pub fn default_root() -> PathBuf {
    home().join("Documents").join("Grimoire")
}

pub fn state_path() -> PathBuf {
    home().join(".config").join("grimoire").join("state.toml")
}

pub fn is_book(p: &Path) -> bool {
    p.join("manuscript").is_dir()
}

/// Shorten the home folder to ~ so printed paths stay readable.
pub fn pretty(p: &Path) -> String {
    let s = p.display().to_string();
    // Windows canonicalises to the \\?\C:\… form; nobody wants to read that.
    let s = s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s);
    let h = home().display().to_string();
    match s.strip_prefix(&h) {
        Some(rest) if h != "." => format!("~{rest}"),
        _ => s,
    }
}

fn values(state: &Path, key: &str) -> Vec<PathBuf> {
    let Ok(s) = std::fs::read_to_string(state) else {
        return Vec::new();
    };
    let lead = format!("{key} = ");
    s.lines()
        .filter_map(|l| l.strip_prefix(&lead))
        .map(|v| v.trim().trim_matches('"'))
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .collect()
}

pub fn last(state: &Path) -> Option<PathBuf> {
    values(state, "last").into_iter().next()
}

/// The books opened here, newest first, leaving out any that have gone.
pub fn recent(state: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in values(state, "last")
        .into_iter()
        .chain(values(state, "recent"))
    {
        if is_book(&p) && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// `root` is the book open now: next launch opens it, and it heads the list.
pub fn remember(state: &Path, root: &Path) {
    if let Some(dir) = state.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut body = format!("last = \"{}\"\n", root.display());
    for p in recent(state)
        .into_iter()
        .filter(|p| p != root)
        .take(KEEP - 1)
    {
        body.push_str(&format!("recent = \"{}\"\n", p.display()));
    }
    let _ = grimoire_core::atomic::write_text(state, &body);
}

/// A book's title from its `novel.toml`, or its folder's name.
pub fn title(root: &Path) -> String {
    std::fs::read_to_string(root.join("novel.toml"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("title = "))
                .map(|v| v.trim().trim_matches('"').to_string())
        })
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| {
            root.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Untitled".into())
        })
}

/// A new folder in `dir` for a book called `title`, numbered if that name
/// is taken.
pub fn beside(dir: &Path, title: &str) -> PathBuf {
    let name = grimoire_core::names::stem(title).replace('-', " ");
    let mut path = dir.join(&name);
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{name} {n}"));
        n += 1;
    }
    path
}

/// "12 chapters, 40 scenes, 84,210 words".
pub fn tally(d: &grimoire_core::import::Draft) -> String {
    let n = |k: usize, one: &str, many: &str| {
        format!(
            "{} {}",
            grimoire_core::manuscript::commas(k),
            if k == 1 { one } else { many }
        )
    };
    format!(
        "{}, {}, {}",
        n(d.chapters(), "chapter", "chapters"),
        n(d.scenes(), "scene", "scenes"),
        n(d.words(), "word", "words")
    )
}

/// What was typed for a book's folder, as a path: `~/…` and `/…` are taken
/// as they are, and a plain name goes beside the book that's open.
pub fn typed_path(beside: &Path, typed: &str) -> Option<PathBuf> {
    // A file dragged into the terminal arrives quoted, or with its spaces
    // escaped.
    let typed = typed.trim().trim_matches(['\'', '"']);
    let unescaped = if cfg!(windows) {
        typed.to_string()
    } else {
        typed.replace("\\ ", " ")
    };
    let typed = unescaped.as_str();
    if typed.is_empty() {
        return None;
    }
    if typed == "~" {
        return Some(home());
    }
    if let Some(rest) = typed.strip_prefix("~/") {
        return Some(home().join(rest));
    }
    let p = PathBuf::from(typed);
    if p.is_absolute() {
        return Some(p);
    }
    Some(beside.parent().unwrap_or(beside).join(typed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_book_leads_and_gone_ones_drop_out() {
        let dir = std::env::temp_dir().join(format!("grimoire-books-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let book = |n: &str| {
            let p = dir.join(n);
            std::fs::create_dir_all(p.join("manuscript")).unwrap();
            p
        };
        let (a, b, c) = (book("a"), book("b"), book("c"));
        let state = dir.join("state.toml");
        for p in [&a, &b, &c, &a] {
            remember(&state, p);
        }
        assert_eq!(last(&state), Some(a.clone()));
        assert_eq!(recent(&state), vec![a.clone(), c.clone(), b.clone()]);
        std::fs::remove_dir_all(&b).unwrap();
        assert_eq!(recent(&state), vec![a, c]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_plain_name_goes_beside_the_open_book() {
        let open = Path::new("/writing/first-novel");
        assert_eq!(
            typed_path(open, "The Long Night"),
            Some(PathBuf::from("/writing/The Long Night"))
        );
        assert_eq!(
            typed_path(open, "/elsewhere/Book"),
            Some(PathBuf::from("/elsewhere/Book"))
        );
        assert_eq!(typed_path(open, "  "), None);
        assert_eq!(
            typed_path(open, "'/drafts/My Novel.docx'"),
            Some(PathBuf::from("/drafts/My Novel.docx"))
        );
        if !cfg!(windows) {
            assert_eq!(
                typed_path(open, "/drafts/My\\ Novel.docx"),
                Some(PathBuf::from("/drafts/My Novel.docx"))
            );
        }
    }
}
