//! Small preferences that aren't a theme or a music source.
//! `~/.config/grimoire/settings.toml`, one `key = value` per line.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Underline misspellings as you write. On unless turned off.
    pub spellcheck: bool,
    /// A symbol beside each row of the tree. Off unless turned on.
    pub icons: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { spellcheck: true, icons: false }
    }
}

fn path() -> PathBuf {
    crate::paths::home().join(".config").join("grimoire").join("settings.toml")
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read_to_string(path()).map(|s| Settings::parse(&s)).unwrap_or_default()
    }

    fn parse(s: &str) -> Settings {
        let mut out = Settings::default();
        for line in s.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k.trim() {
                "spellcheck" => out.spellcheck = v.trim() != "false",
                "icons" => out.icons = v.trim() == "true",
                _ => {}
            }
        }
        out
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, format!("spellcheck = {}\nicons = {}\n", self.spellcheck, self.icons))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellcheck_is_on_unless_switched_off() {
        assert!(Settings::default().spellcheck);
        assert!(Settings::parse("").spellcheck);
        assert!(!Settings::parse("spellcheck = false\n").spellcheck);
        assert!(Settings::parse("spellcheck = true\n").spellcheck);
    }

    #[test]
    fn tree_icons_are_off_unless_switched_on() {
        assert!(!Settings::default().icons);
        assert!(!Settings::parse("spellcheck = false\n").icons);
        let both = Settings::parse("spellcheck = false\nicons = true\n");
        assert!(both.icons && !both.spellcheck);
    }
}
