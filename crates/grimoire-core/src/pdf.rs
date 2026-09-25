//! PDF, by way of LibreOffice: the manuscript DOCX converted headless, so the
//! PDF is page for page what an agent would see opening the Word file.
//!
//! A PDF writer of our own would be a second typesetter to keep in step with
//! the DOCX; LibreOffice already lays the DOCX out, and is free on every
//! system. Found on PATH (`soffice`, `libreoffice`), in the usual install
//! places on macOS and Windows, or as the Flatpak
//! (`org.libreoffice.LibreOffice`). When none is there, [`find`] says so and
//! the caller says how to get it — never a silent nothing.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// How to reach LibreOffice on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Office {
    /// An `soffice` binary.
    Binary(PathBuf),
    /// The Flatpak, run as `flatpak run <id>` — its own command is
    /// `libreoffice`, which takes soffice's arguments.
    Flatpak,
}

const FLATPAK_ID: &str = "org.libreoffice.LibreOffice";

/// What to tell someone who has no LibreOffice.
pub const HOW_TO_GET: &str = "PDF needs LibreOffice (free): libreoffice.org, or `flatpak install flathub org.libreoffice.LibreOffice`";

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// LibreOffice, if this machine has it.
pub fn find() -> Option<Office> {
    for name in ["soffice", "libreoffice", "soffice.exe"] {
        if let Some(p) = on_path(name) {
            return Some(Office::Binary(p));
        }
    }
    let fixed = [
        "/Applications/LibreOffice.app/Contents/MacOS/soffice",
        "C:\\Program Files\\LibreOffice\\program\\soffice.exe",
        "C:\\Program Files (x86)\\LibreOffice\\program\\soffice.exe",
    ];
    if let Some(p) = fixed.iter().map(PathBuf::from).find(|p| p.is_file()) {
        return Some(Office::Binary(p));
    }
    let flatpak_installed = on_path("flatpak").is_some()
        && Command::new("flatpak")
            .args(["info", FLATPAK_ID])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    flatpak_installed.then_some(Office::Flatpak)
}

/// Convert `docx` to a PDF beside it, and return the PDF's path. Runs
/// LibreOffice with a profile of its own, so an open LibreOffice window (or a
/// locked profile) can't make the conversion quietly do nothing.
///
/// The Flatpak sees the home folder but not everything else (not `/tmp`, not
/// every mounted drive), so for it the DOCX goes through a folder in the
/// cache and the PDF is copied back beside the original.
pub fn convert(office: &Office, docx: &Path) -> Result<PathBuf> {
    if *office == Office::Flatpak {
        let stage = crate::paths::home()
            .join(".cache")
            .join("grimoire")
            .join(format!("pdf-{}", std::process::id()));
        std::fs::create_dir_all(&stage).with_context(|| format!("creating {}", stage.display()))?;
        let name = docx.file_name().context("the manuscript has no name")?;
        let staged = stage.join(name);
        let result = std::fs::copy(docx, &staged)
            .with_context(|| format!("copying to {}", staged.display()))
            .and_then(|_| convert_here(office, &staged))
            .and_then(|made| {
                let out = docx.with_extension("pdf");
                let bytes = std::fs::read(&made).context("reading the PDF")?;
                crate::atomic::write(&out, &bytes)?;
                Ok(out)
            });
        let _ = std::fs::remove_dir_all(&stage);
        return result;
    }
    convert_here(office, docx)
}

/// [`convert`], with the PDF written in the DOCX's own folder.
fn convert_here(office: &Office, docx: &Path) -> Result<PathBuf> {
    let dir = docx
        .parent()
        .context("the manuscript has no folder to put the PDF in")?;
    let stem = docx
        .file_stem()
        .context("the manuscript has no name")?
        .to_string_lossy()
        .to_string();
    let out = dir.join(format!("{stem}.pdf"));
    let profile = std::env::temp_dir().join(format!("grimoire-lo-{}", std::process::id()));
    // A file URL on every system: file:///tmp/… and file:///C:/Users/….
    let slashed = profile.display().to_string().replace('\\', "/");
    let profile_url = if slashed.starts_with('/') {
        format!("file://{slashed}")
    } else {
        format!("file:///{slashed}")
    };
    let mut cmd = match office {
        Office::Binary(p) => Command::new(p),
        Office::Flatpak => {
            let mut c = Command::new("flatpak");
            c.args(["run", FLATPAK_ID]);
            c
        }
    };
    cmd.arg(format!("-env:UserInstallation={profile_url}"))
        .args([
            "--headless",
            "--norestore",
            "--convert-to",
            "pdf",
            "--outdir",
        ])
        .arg(dir)
        .arg(docx);
    let before = std::fs::metadata(&out).and_then(|m| m.modified()).ok();
    let result = cmd.output().context("couldn't start LibreOffice")?;
    let _ = std::fs::remove_dir_all(&profile);
    let after = std::fs::metadata(&out).and_then(|m| m.modified()).ok();
    if !result.status.success() || after.is_none() || after == before {
        let why = String::from_utf8_lossy(&result.stderr);
        let why = why
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        bail!(
            "LibreOffice didn't write the PDF{}",
            if why.is_empty() {
                String::new()
            } else {
                format!(" ({why})")
            }
        );
    }
    Ok(out)
}
