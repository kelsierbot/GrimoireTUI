//! File names every sync client and every desktop will take.
//!
//! A book lives in Dropbox, Google Drive, pCloud, Box, OneDrive or iCloud as
//! often as not, and is opened on Linux, a Mac and Windows. A name one of them
//! accepts can break on another: Linux takes `what?.md` and `wren.md` beside
//! `Wren.md`; Windows refuses the first and a Mac or Box sees the second pair
//! as one file. So every name Grimoire makes — new items, renames, moves —
//! goes through [`stem`] and [`fit`], and is checked with [`clash`] against
//! what is already in the folder.

use std::fs;
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// The most bytes a name's own words may take. File systems allow 255; this
/// leaves room for a number, an extension and a sync client's "(conflicted
/// copy 2026-09-25)".
pub const STEM_MAX: usize = 120;

/// Keep a whole path under this many characters. Windows stops at 260, and
/// sync clients add to a name when they make a conflict copy.
pub const PATH_MAX: usize = 240;

/// Names Windows keeps for devices, with or without an extension.
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The words of a title as a file name: letters, digits and apostrophes in
/// Unicode's composed form (NFC), joined by dashes. Everything any client
/// forbids — `\ / : * ? " < > |`, control characters, leading or trailing
/// dots and spaces — is a separator, so it never reaches the disk. Characters
/// beyond the Basic Multilingual Plane are dropped (Box won't store them), a
/// name Windows reserves gets `-note` after it, and a long title is cut at a
/// word to [`STEM_MAX`].
pub fn stem(title: &str) -> String {
    let composed: String = title.nfc().collect();
    let words: Vec<&str> = composed
        .split(|c: char| !(c.is_alphanumeric() || c == '\'') || c > '\u{FFFF}')
        .filter(|w| !w.is_empty())
        .collect();
    if words.is_empty() {
        return "untitled".into();
    }
    let joined = cut(&words.join("-"), STEM_MAX);
    if is_reserved(&joined) {
        format!("{joined}-note")
    } else {
        joined
    }
}

/// Whether Windows keeps `name` (up to its first dot) for a device.
pub fn is_reserved(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or(name);
    RESERVED.iter().any(|r| r.eq_ignore_ascii_case(base))
}

/// At most `max` bytes of `s`, on a character boundary, at the last dash in
/// the second half if there is one, so a word isn't cut in two.
fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    let head = &s[..end];
    let head = match head.rfind('-') {
        Some(i) if i >= max / 2 => &head[..i],
        _ => head,
    };
    head.trim_end_matches(['-', '\'']).to_string()
}

/// `prefix` + `stem` + `ext` for a file in `dir`, with the stem shortened (at
/// a word, never below eight characters) so the whole path stays under
/// [`PATH_MAX`]. `prefix` is the item's number and dash, if it has one; `ext`
/// includes its dot, or is empty for a folder.
pub fn fit(dir: &Path, prefix: &str, stem: &str, ext: &str) -> String {
    let base = dir.to_string_lossy().chars().count() + 1 + prefix.chars().count() + ext.len();
    let room = PATH_MAX.saturating_sub(base).max(8);
    let stem = if stem.chars().count() > room {
        let bytes: usize = stem.chars().take(room).map(char::len_utf8).sum();
        cut(stem, bytes)
    } else {
        stem.to_string()
    };
    format!("{prefix}{stem}{ext}")
}

/// A name as clients that ignore case and accents see it: decomposed, marks
/// dropped, lower-cased. `Café.md`, `cafe.md` and `CAFÉ.md` all fold to
/// `cafe.md` — Box treats them as one file, and so do Macs and Windows
/// (case), so two of them in one folder is a conflict waiting to happen.
pub fn fold(name: &str) -> String {
    name.nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

/// Something already in `dir` that `name` would collide with on some client:
/// same name ignoring case and accents. `except` is the item being renamed —
/// changing only the capitals of its own name isn't a clash. Hidden entries
/// (`.grimoire`, `.git`) don't count.
pub fn clash(dir: &Path, name: &str, except: Option<&Path>) -> Option<PathBuf> {
    let want = fold(name);
    let rd = fs::read_dir(dir).ok()?;
    for e in rd.flatten() {
        let other = e.file_name().to_string_lossy().to_string();
        if other.starts_with('.') || fold(&other) != want {
            continue;
        }
        let path = e.path();
        if except.is_some_and(|x| x == path || same_file(x, &path)) {
            continue;
        }
        return Some(path);
    }
    None
}

/// Whether two paths are the one file: the same inode on Unix, the same
/// canonical path elsewhere. How a case-insensitive disk answers
/// "`Wren.md` exists" when asked about `wren.md`.
pub fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(x), Ok(y)) = (fs::metadata(a), fs::metadata(b)) {
            return x.dev() == y.dev() && x.ino() == y.ino();
        }
        false
    }
    #[cfg(not(unix))]
    {
        matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
    }
}

/// Two paths that name the same thing once both are in composed form. A Mac
/// may hand a synced folder `Rosé` spelled as `e` + a combining accent; the
/// resume point or history written on Linux spells it as one `é`.
pub fn same_path(a: &Path, b: &Path) -> bool {
    a == b || nfc(a) == nfc(b)
}

/// A path in composed form (NFC), for comparing and for keys.
pub fn nfc(p: &Path) -> PathBuf {
    PathBuf::from(p.to_string_lossy().nfc().collect::<String>())
}

/// Find `path` on disk even when a folder or file along the way is spelled in
/// another normal form than the one asked for. `None` if nothing matches.
pub fn resolve(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return Some(path.to_path_buf());
    }
    let mut out = PathBuf::new();
    for part in path.components() {
        let next = out.join(part.as_os_str());
        if next.exists() || out.as_os_str().is_empty() {
            out = next;
            continue;
        }
        let want: String = part.as_os_str().to_string_lossy().nfc().collect();
        let found = fs::read_dir(&out)
            .ok()?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().nfc().collect::<String>() == want)?;
        out = found.path();
    }
    out.exists().then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_lose_what_any_client_forbids() {
        assert_eq!(stem(r#"what? a:b* <c> |d| "e". "#), "what-a-b-c-d-e");
        assert_eq!(stem("back\\slash/and.dots..."), "back-slash-and-dots");
        assert_eq!(stem("  Wren's letter  "), "Wren's-letter");
        assert_eq!(stem("tab\there\u{7}bell"), "tab-here-bell");
        assert_eq!(stem("???"), "untitled");
    }

    #[test]
    fn reserved_windows_names_get_a_suffix() {
        for r in ["Con", "con", "PRN", "Aux", "nul", "COM1", "lpt9"] {
            let s = stem(r);
            assert!(!is_reserved(&s), "{r} → {s}");
            assert!(s.to_lowercase().starts_with(&r.to_lowercase()));
        }
        assert_eq!(
            stem("Console"),
            "Console",
            "only the whole word is reserved"
        );
        assert!(is_reserved("con.md") && is_reserved("LPT1.txt"));
    }

    #[test]
    fn names_are_composed_and_short() {
        let decomposed = "Rose\u{301}";
        assert_eq!(stem(decomposed), "Ros\u{e9}");
        let long = "word ".repeat(80);
        let s = stem(&long);
        assert!(s.len() <= STEM_MAX && !s.ends_with('-'), "{}", s.len());
        let cjk = "書".repeat(100);
        assert!(stem(&cjk).len() <= STEM_MAX);
        assert_eq!(
            stem("smile 😀 now"),
            "smile-now",
            "no characters Box can't store"
        );
    }

    #[test]
    fn a_deep_folder_shortens_the_name_to_fit_the_path() {
        let dir = PathBuf::from("/x".repeat(100));
        let name = fit(&dir, "07-", &stem(&"chapter ".repeat(20)), ".md");
        assert!(
            dir.join(&name).to_string_lossy().chars().count()
                <= PATH_MAX.max(dir.to_string_lossy().chars().count() + 1 + 3 + 8 + 3)
        );
        assert!(name.starts_with("07-") && name.ends_with(".md"));
    }

    #[test]
    fn folding_ignores_case_and_accents() {
        assert_eq!(fold("Café.md"), fold("cafe\u{301}.MD"));
        assert_eq!(fold("Wren.md"), fold("wren.md"));
        assert_ne!(fold("Wren.md"), fold("Wren-2.md"));
    }
}
