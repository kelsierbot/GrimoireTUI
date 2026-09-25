//! Small preferences that aren't a theme or a music source.
//! `~/.config/grimoire/settings.toml`, one `key = value` per line.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Underline misspellings as you write. On unless turned off.
    pub spellcheck: bool,
    /// A symbol beside each row of the tree. Off unless turned on.
    pub icons: bool,
    /// The widest a line of prose runs, in columns, centred in the editor.
    /// 0 lets it fill the pane.
    pub line_width: usize,
    /// In focus mode, the line being written stays near the middle of the
    /// screen. On unless turned off.
    pub typewriter: bool,
}

/// A comfortable measure for prose: about twelve words a line.
pub const LINE_WIDTH: usize = 72;

impl Default for Settings {
    fn default() -> Self {
        Settings {
            spellcheck: true,
            icons: false,
            line_width: LINE_WIDTH,
            typewriter: true,
        }
    }
}

fn path() -> PathBuf {
    crate::paths::home()
        .join(".config")
        .join("grimoire")
        .join("settings.toml")
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read_to_string(path())
            .map(|s| Settings::parse(&s))
            .unwrap_or_default()
    }

    fn parse(s: &str) -> Settings {
        let mut out = Settings::default();
        for line in s.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match k.trim() {
                "spellcheck" => out.spellcheck = v.trim() != "false",
                "icons" => out.icons = v.trim() == "true",
                "line_width" => {
                    if let Ok(n) = v.trim().parse() {
                        out.line_width = n;
                    }
                }
                "typewriter" => out.typewriter = v.trim() != "false",
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
        std::fs::write(p, self.to_text())
    }

    fn to_text(&self) -> String {
        format!(
            "spellcheck = {}\nicons = {}\n\
             # Widest a line of prose runs, in columns; 0 fills the pane.\n\
             line_width = {}\ntypewriter = {}\n",
            self.spellcheck, self.icons, self.line_width, self.typewriter
        )
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

    #[test]
    fn line_width_defaults_to_a_readable_measure() {
        assert_eq!(Settings::parse("").line_width, LINE_WIDTH);
        assert_eq!(Settings::parse("line_width = 0\n").line_width, 0);
        assert_eq!(Settings::parse("line_width = 88\n").line_width, 88);
        // Nonsense keeps the default rather than collapsing to nothing.
        assert_eq!(
            Settings::parse("line_width = wide\n").line_width,
            LINE_WIDTH
        );
    }

    #[test]
    fn typewriter_is_on_unless_switched_off() {
        assert!(Settings::parse("").typewriter);
        assert!(!Settings::parse("typewriter = false\n").typewriter);
    }

    #[test]
    fn every_setting_survives_a_save() {
        let s = Settings {
            spellcheck: false,
            icons: true,
            line_width: 60,
            typewriter: false,
        };
        assert_eq!(Settings::parse(&s.to_text()), s);
    }
}
