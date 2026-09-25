//! Which sync service, if any, a book's folder lives in.
//!
//! Only for saying so, and for one warning: writing-session history kept
//! inside a synced folder (see `sessions`). Nothing else behaves differently —
//! conflict copies are recognised by name whatever put them there.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    Dropbox,
    GoogleDrive,
    PCloud,
    Box,
    OneDrive,
    ICloud,
}

impl Client {
    pub fn label(self) -> &'static str {
        match self {
            Client::Dropbox => "Dropbox",
            Client::GoogleDrive => "Google Drive",
            Client::PCloud => "pCloud",
            Client::Box => "Box",
            Client::OneDrive => "OneDrive",
            Client::ICloud => "iCloud Drive",
        }
    }
}

/// The sync service `root` sits inside, judged by the folders on the way to it
/// (each client's usual names, on Linux, macOS and Windows), the marker files
/// some leave at their top, and — on Linux — a pCloud Drive mount.
pub fn detect(root: &Path) -> Option<Client> {
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let home = crate::paths::home();
    by_names(&root, &home)
        .or_else(|| by_markers(&root))
        .or_else(|| by_mount(&root))
}

/// Folder names along the path. `home` decides the one ambiguous case: a
/// folder called just "Box" counts only straight under the home folder, where
/// Box Drive puts it.
fn by_names(root: &Path, home: &Path) -> Option<Client> {
    let parts: Vec<String> = root
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    for (i, part) in parts.iter().enumerate() {
        let p = part.as_str();
        let lower = p.to_lowercase();
        // macOS File Provider: ~/Library/CloudStorage/<Client>-<account>
        if i > 0 && parts[i - 1] == "CloudStorage" {
            let found = match lower.split(['-', ' ']).next().unwrap_or("") {
                "dropbox" => Some(Client::Dropbox),
                "googledrive" => Some(Client::GoogleDrive),
                "pcloud" => Some(Client::PCloud),
                "box" => Some(Client::Box),
                "onedrive" => Some(Client::OneDrive),
                _ => None,
            };
            if found.is_some() {
                return found;
            }
        }
        if p == "Dropbox" || p.starts_with("Dropbox (") {
            return Some(Client::Dropbox);
        }
        if p == "Google Drive" || p == "My Drive" || p == "GoogleDrive" {
            return Some(Client::GoogleDrive);
        }
        if p == "pCloudDrive" || p == "pCloud Drive" || p == "pCloud Sync" {
            return Some(Client::PCloud);
        }
        if p == "Box Drive" || p == "Box Sync" || p == "Box-Box" {
            return Some(Client::Box);
        }
        if p == "Box" && root.starts_with(home.join("Box")) {
            return Some(Client::Box);
        }
        if p == "OneDrive" || p.starts_with("OneDrive - ") || p.starts_with("OneDrive-") {
            return Some(Client::OneDrive);
        }
        if p == "Mobile Documents" || p == "iCloud Drive" || p == "iCloudDrive" {
            return Some(Client::ICloud);
        }
    }
    None
}

/// Files a client leaves at the top of its folder.
fn by_markers(root: &Path) -> Option<Client> {
    let mut dir: Option<&Path> = Some(root);
    while let Some(d) = dir {
        if d.join(".dropbox").is_file() || d.join(".dropbox.cache").is_dir() {
            return Some(Client::Dropbox);
        }
        if d.join(".tmp.drivedownload").exists() || d.join(".tmp.driveupload").exists() {
            return Some(Client::GoogleDrive);
        }
        dir = d.parent();
    }
    None
}

/// Linux: the pCloud Drive FUSE mount (`fsname=pCloud.fs`) the book is under.
fn by_mount(root: &Path) -> Option<Client> {
    let mounts = std::fs::read_to_string("/proc/self/mounts").ok()?;
    let mut best: Option<(usize, bool)> = None;
    for line in mounts.lines() {
        let mut f = line.split_whitespace();
        let (Some(source), Some(point)) = (f.next(), f.next()) else {
            continue;
        };
        let point = PathBuf::from(point.replace("\\040", " "));
        if root.starts_with(&point) {
            let len = point.as_os_str().len();
            if best.is_none_or(|(l, _)| len > l) {
                best = Some((len, source.contains("pCloud")));
            }
        }
    }
    best.and_then(|(_, pcloud)| pcloud.then_some(Client::PCloud))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(p: &str) -> Option<Client> {
        by_names(Path::new(p), Path::new("/home/ann"))
    }

    #[test]
    fn each_client_is_known_by_its_folders() {
        assert_eq!(at("/home/ann/Dropbox/Novels/Salt"), Some(Client::Dropbox));
        assert_eq!(at("/home/ann/Dropbox (Acme)/Salt"), Some(Client::Dropbox));
        assert_eq!(
            at("/Users/ann/Library/CloudStorage/Dropbox/Salt"),
            Some(Client::Dropbox)
        );
        assert_eq!(
            at("/Users/ann/Library/CloudStorage/GoogleDrive-ann@example.com/My Drive/Salt"),
            Some(Client::GoogleDrive)
        );
        assert_eq!(at("G:/My Drive/Salt"), Some(Client::GoogleDrive));
        assert_eq!(at("/home/ann/pCloudDrive/Salt"), Some(Client::PCloud));
        assert_eq!(at("/Users/ann/pCloud Drive/Salt"), Some(Client::PCloud));
        assert_eq!(
            at("/Users/ann/Library/CloudStorage/Box-Box/Salt"),
            Some(Client::Box)
        );
        assert_eq!(at("/home/ann/Box/Salt"), Some(Client::Box));
        assert_eq!(
            at("/home/ann/Documents/Box/Salt"),
            None,
            "a box is just a box"
        );
        assert_eq!(at("/home/ann/OneDrive - Acme/Salt"), Some(Client::OneDrive));
        assert_eq!(
            at("/Users/ann/Library/CloudStorage/OneDrive-Personal/Salt"),
            Some(Client::OneDrive)
        );
        assert_eq!(
            at("/Users/ann/Library/Mobile Documents/com~apple~CloudDocs/Salt"),
            Some(Client::ICloud)
        );
        assert_eq!(at("C:/Users/ann/iCloudDrive/Salt"), Some(Client::ICloud));
        assert_eq!(at("/home/ann/Documents/Grimoire"), None);
    }

    #[test]
    fn a_dropbox_marker_is_enough() {
        let d = std::env::temp_dir().join(format!("grimoire-cloud-{}", std::process::id()));
        let book = d.join("Novels/Salt");
        std::fs::create_dir_all(&book).unwrap();
        assert_eq!(by_markers(&book), None);
        std::fs::write(d.join(".dropbox"), "{}").unwrap();
        assert_eq!(by_markers(&book), Some(Client::Dropbox));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
