//! Where you left off, kept inside the book so it travels with it.
//!
//! `.grimoire/resume.md` records the scene, the paragraph and the column you
//! were at, which computer it was, and when. Open the book anywhere the
//! folder syncs to — or anywhere session history carries it — and Grimoire
//! lands on that sentence. It's Markdown, so it reads fine on its own:
//!
//! ```text
//! ---
//! scene: manuscript/02-Act-Two/14-Chapter-Fourteen/02-The-Crossing.md
//! line: 41
//! column: 17
//! machine: bazzite
//! when: 2026-09-16T21:41:07-04:00
//! ---
//!
//! You were writing The Crossing (Act Two › Chapter Fourteen), paragraph 42,
//! on bazzite — Wednesday 16 September, 9:41 pm.
//! ```

use anyhow::Result;
use chrono::{DateTime, Local};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Resume {
    /// Relative to the book's root, with forward slashes.
    pub scene: String,
    pub line: usize,
    pub column: usize,
    pub machine: String,
    pub when: DateTime<Local>,
}

pub fn path(root: &Path) -> PathBuf {
    root.join(".grimoire").join("resume.md")
}

/// This computer's name, as the writer would recognise it. Looked up once.
pub fn machine_name() -> String {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(lookup_machine_name).clone()
}

fn lookup_machine_name() -> String {
    let from_env = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).ok();
    let name = from_env
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        })
        .unwrap_or_default();
    let name = name.trim().trim_end_matches(".local").to_string();
    if name.is_empty() { "another computer".into() } else { name }
}

pub fn read(root: &Path) -> Option<Resume> {
    let text = std::fs::read_to_string(path(root)).ok()?;
    let (front, _) = crate::project::split_frontmatter(&text);
    let front = front?;
    let get = |key: &str| {
        front.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
        })
    };
    Some(Resume {
        scene: get("scene")?,
        line: get("line").and_then(|v| v.parse().ok()).unwrap_or(0),
        column: get("column").and_then(|v| v.parse().ok()).unwrap_or(0),
        machine: get("machine").unwrap_or_default(),
        when: get("when").and_then(|v| DateTime::parse_from_rfc3339(&v).ok()).map(|d| d.with_timezone(&Local))?,
    })
}

/// Write where you are. `title` and `place` make the sentence under the
/// frontmatter: "The Crossing", "Act Two › Chapter Fourteen".
pub fn write(root: &Path, r: &Resume, title: &str, place: &str) -> Result<()> {
    let dir = root.join(".grimoire");
    std::fs::create_dir_all(&dir)?;
    let where_ = if place.is_empty() { title.to_string() } else { format!("{title} ({place})") };
    let text = format!(
        "---\nscene: {}\nline: {}\ncolumn: {}\nmachine: {}\nwhen: {}\n---\n\nYou were writing {where_}, paragraph {}, on {} — {}.\n",
        r.scene,
        r.line,
        r.column,
        r.machine,
        r.when.to_rfc3339(),
        r.line + 1,
        r.machine,
        r.when.format("%A %-d %B, %-I:%M %P"),
    );
    crate::project::write_atomic(&path(root), &text)
}

/// A scene's path relative to the book, the way resume.md stores it.
pub fn relative(root: &Path, scene: &Path) -> String {
    scene.strip_prefix(root).unwrap_or(scene).to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn where_you_were_reads_back_exactly() {
        let d = std::env::temp_dir().join(format!("grimoire-resume-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let when = Local.with_ymd_and_hms(2026, 9, 16, 21, 41, 7).unwrap();
        let r = Resume {
            scene: "manuscript/02-Act-Two/14-Chapter-Fourteen/02-The-Crossing.md".into(),
            line: 41,
            column: 17,
            machine: "bazzite".into(),
            when,
        };
        write(&d, &r, "The Crossing", "Act Two › Chapter Fourteen").unwrap();
        assert_eq!(read(&d), Some(r));
        let text = std::fs::read_to_string(path(&d)).unwrap();
        assert!(text.contains("You were writing The Crossing (Act Two › Chapter Fourteen), paragraph 42, on bazzite — Wednesday 16 September, 9:41 pm."));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_missing_or_broken_note_is_just_no_resume() {
        let d = std::env::temp_dir().join(format!("grimoire-resume-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".grimoire")).unwrap();
        assert_eq!(read(&d), None);
        std::fs::write(path(&d), "no frontmatter at all").unwrap();
        assert_eq!(read(&d), None);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
