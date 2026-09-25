//! Writing a file so that nothing — a crash, a sync client, another program —
//! ever sees half of it.
//!
//! The new bytes go into a hidden sibling first, are flushed to disk, and are
//! then renamed over the file in one step: whoever looks sees the old version
//! or the new one, never a torn mix. The sibling is named
//! `.~lock.<name>.<pid>.tmp`, a shape pCloud, Dropbox, Box, OneDrive and
//! iCloud all decline to upload (pCloud skips `.~lock.*`, Dropbox `.~*`, Box
//! dot-files and `.tmp`, OneDrive and iCloud `.tmp`), so the book's sync never
//! carries a half-saved scene to another device. Google Drive has no ignore
//! list; the rename is quick enough that it rarely sees one.
//!
//! On Windows, replacing a file fails while another program has it open — a
//! sync client uploading it, the indexer, antivirus. That clears in moments,
//! so the rename is retried with a short backoff. What it never does is fall
//! back to writing the file in place: a sync client could then upload the
//! half-written file to every device. If the rename still can't happen, the
//! temporary copy is removed and the error returned; the caller keeps the
//! words (in memory, and in recovery) and tries again later.

use anyhow::{Context, Result};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Waits between tries at the rename, in milliseconds: about a second and a
/// half in all, long enough for a sync client's upload of a scene to finish.
const BACKOFF_MS: [u64; 6] = [25, 50, 100, 200, 400, 800];

/// The hidden sibling a write goes through: `.~lock.<name>.<pid>.tmp`.
pub fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    path.with_file_name(format!(".~lock.{name}.{}.tmp", std::process::id()))
}

/// Is this one of Grimoire's temporary siblings (from this or an earlier
/// version)? Never shown in the tree, never taken for a scene.
pub fn is_temp(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    (name.starts_with(".~lock.") && name.ends_with(".tmp"))
        || (name.starts_with('.') && name.ends_with(".saving"))
}

/// Replace `path` with `bytes`, all at once. See the module notes.
pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    write_with(path, bytes, &|from, to| fs::rename(from, to), &|ms| {
        std::thread::sleep(Duration::from_millis(ms))
    })
}

/// [`write`] for text.
pub fn write_text(path: &Path, text: &str) -> Result<()> {
    write(path, text.as_bytes())
}

/// [`write`] for callers that speak `io::Result` (settings files).
pub fn write_io(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write(path, bytes).map_err(|e| io::Error::other(format!("{e:#}")))
}

/// [`write`] with the rename and the wait passed in, so tests can make the
/// rename fail the way Windows does without a second process holding the file.
pub(crate) fn write_with(
    path: &Path,
    bytes: &[u8],
    rename: &dyn Fn(&Path, &Path) -> io::Result<()>,
    wait: &dyn Fn(u64),
) -> Result<()> {
    let tmp = temp_path(path);
    let written = (|| -> io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("writing {}", path.display()));
    }
    let mut last = None;
    for pause in std::iter::once(0).chain(BACKOFF_MS) {
        if pause > 0 {
            wait(pause);
        }
        match rename(&tmp, path) {
            Ok(()) => {
                sync_parent(path);
                return Ok(());
            }
            // The folder itself went away: waiting won't bring it back.
            Err(e) if e.kind() == io::ErrorKind::NotFound && !tmp.exists() => {
                last = Some(e);
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    let _ = fs::remove_file(&tmp);
    let e = last.unwrap_or_else(|| io::Error::other("the file couldn't be replaced"));
    Err(e).with_context(|| format!("replacing {}", path.display()))
}

/// Make the rename itself durable, where the platform allows: a crash just
/// after saving then can't bring the old version back.
fn sync_parent(path: &Path) {
    #[cfg(unix)]
    if let Some(dir) = path.parent()
        && let Ok(d) = fs::File::open(if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        })
    {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grimoire-atomic-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_temporary_copy_is_one_every_sync_client_ignores() {
        let t = temp_path(Path::new("/book/01-Scene-One.md"));
        let name = t.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with(".~lock.01-Scene-One.md."), "{name}");
        assert!(name.ends_with(".tmp"), "{name}");
        assert!(is_temp(&t));
        assert!(
            is_temp(Path::new("/book/.01-Scene-One.md.saving")),
            "the old shape too"
        );
        assert!(!is_temp(Path::new("/book/01-Scene-One.md")));
    }

    #[test]
    fn a_write_replaces_the_file_and_leaves_nothing_behind() {
        let d = scratch("replace");
        let f = d.join("scene.md");
        fs::write(&f, "old").unwrap();
        write_text(&f, "new").unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "new");
        assert_eq!(
            fs::read_dir(&d).unwrap().count(),
            1,
            "no temporary copy left"
        );
    }

    #[test]
    fn a_rename_refused_for_a_moment_is_retried() {
        let d = scratch("busy");
        let f = d.join("scene.md");
        fs::write(&f, "old").unwrap();
        let tries = Cell::new(0);
        write_with(
            &f,
            b"new",
            &|a, b| {
                tries.set(tries.get() + 1);
                if tries.get() < 3 {
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                } else {
                    fs::rename(a, b)
                }
            },
            &|_| {},
        )
        .unwrap();
        assert_eq!(tries.get(), 3);
        assert_eq!(fs::read_to_string(&f).unwrap(), "new");
    }

    /// Windows holding the file for good: the save fails, and the file is
    /// left exactly as it was — never half-written in place.
    #[test]
    fn a_rename_that_never_succeeds_leaves_the_file_untouched() {
        let d = scratch("stuck");
        let f = d.join("scene.md");
        fs::write(&f, "the version on disk").unwrap();
        let tries = Cell::new(0);
        let r = write_with(
            &f,
            b"the new words",
            &|_, _| {
                tries.set(tries.get() + 1);
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            },
            &|_| {},
        );
        assert!(r.is_err());
        assert_eq!(tries.get(), 1 + BACKOFF_MS.len(), "tried, then tried again");
        assert_eq!(fs::read_to_string(&f).unwrap(), "the version on disk");
        assert_eq!(
            fs::read_dir(&d).unwrap().count(),
            1,
            "the temporary copy is gone"
        );
    }
}
