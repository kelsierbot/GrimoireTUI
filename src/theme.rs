//! Colour themes.
//!
//! Nine presets drawn from the palettes people already run in their
//! terminals, plus a custom slot. A theme is a flat set of named roles — no
//! inheritance, no derivation — so the custom editor can just be a list of
//! swatches you type hex into.

use ratatui::style::Color;
use std::path::PathBuf;

/// Every colour the interface uses. Twelve roles, in the order the custom
/// editor lists them.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub name: String,
    pub accent: Color,
    pub text: Color,
    pub dim: Color,
    pub border: Color,
    pub sel: Color,
    pub warn: Color,
    pub sun: Color,
    pub moon: Color,
    pub foliage: Color,
    pub bark: Color,
    pub bloom: Color,
    pub turf: Color,
}

/// Field order for the custom editor and for (de)serialisation.
pub const ROLES: [&str; 12] = [
    "accent",
    "text",
    "dim",
    "border",
    "selection",
    "warning",
    "sun",
    "moon",
    "foliage",
    "bark",
    "bloom",
    "turf",
];

impl Theme {
    pub fn role(&self, i: usize) -> Color {
        match i {
            0 => self.accent,
            1 => self.text,
            2 => self.dim,
            3 => self.border,
            4 => self.sel,
            5 => self.warn,
            6 => self.sun,
            7 => self.moon,
            8 => self.foliage,
            9 => self.bark,
            10 => self.bloom,
            _ => self.turf,
        }
    }

    pub fn set_role(&mut self, i: usize, c: Color) {
        match i {
            0 => self.accent = c,
            1 => self.text = c,
            2 => self.dim = c,
            3 => self.border = c,
            4 => self.sel = c,
            5 => self.warn = c,
            6 => self.sun = c,
            7 => self.moon = c,
            8 => self.foliage = c,
            9 => self.bark = c,
            10 => self.bloom = c,
            _ => self.turf = c,
        }
    }
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb(
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}

pub fn hex_of(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => "#000000".into(),
    }
}

pub fn parse_hex(s: &str) -> Option<Color> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(rgb(v))
}

macro_rules! theme {
    ($name:literal, $accent:expr, $text:expr, $dim:expr, $border:expr, $sel:expr, $warn:expr,
     $sun:expr, $moon:expr, $foliage:expr, $bark:expr, $bloom:expr, $turf:expr) => {
        Theme {
            name: String::new(), // filled by presets()
            accent: rgb($accent),
            text: rgb($text),
            dim: rgb($dim),
            border: rgb($border),
            sel: rgb($sel),
            warn: rgb($warn),
            sun: rgb($sun),
            moon: rgb($moon),
            foliage: rgb($foliage),
            bark: rgb($bark),
            bloom: rgb($bloom),
            turf: rgb($turf),
        }
    };
}

/// The nine presets. Order is the picker order.
pub fn presets() -> Vec<Theme> {
    let mut v = vec![
        //       accent    text      dim       border    sel       warn      sun       moon      foliage   bark      bloom     turf
        (
            "Grimoire",
            theme!(
                "", 0xd6ad60, 0xdedcd4, 0x6c6c7a, 0x464654, 0x30303c, 0xc87864, 0xe8b65c, 0xacb6d4,
                0x5c7860, 0x705c48, 0xc68a9e, 0x465242
            ),
        ),
        (
            "Gruvbox Dark",
            theme!(
                "", 0xd79921, 0xebdbb2, 0x928374, 0x504945, 0x3c3836, 0xcc241d, 0xfabd2f, 0x83a598,
                0x98971a, 0xa87c4f, 0xd3869b, 0x4f5340
            ),
        ),
        (
            "Nord",
            theme!(
                "", 0x88c0d0, 0xd8dee9, 0x616e88, 0x3b4252, 0x434c5e, 0xbf616a, 0xebcb8b, 0x81a1c1,
                0xa3be8c, 0x8a7660, 0xb48ead, 0x4a5a4a
            ),
        ),
        (
            "Dracula",
            theme!(
                "", 0xbd93f9, 0xf8f8f2, 0x6272a4, 0x44475a, 0x44475a, 0xff5555, 0xf1fa8c, 0x8be9fd,
                0x50fa7b, 0x9b6a4a, 0xff79c6, 0x3d5a45
            ),
        ),
        (
            "Solarized Dark",
            theme!(
                "", 0xb58900, 0x93a1a1, 0x586e75, 0x0b3c47, 0x073642, 0xdc322f, 0xcb9b00, 0x268bd2,
                0x859900, 0x9c6a3c, 0xd33682, 0x445b3a
            ),
        ),
        (
            "Catppuccin Mocha",
            theme!(
                "", 0xcba6f7, 0xcdd6f4, 0x7f849c, 0x313244, 0x45475a, 0xf38ba8, 0xf9e2af, 0x89b4fa,
                0xa6e3a1, 0xb08968, 0xf5c2e7, 0x4c6b52
            ),
        ),
        (
            "Tokyo Night",
            theme!(
                "", 0x7aa2f7, 0xc0caf5, 0x565f89, 0x292e42, 0x33467c, 0xf7768e, 0xe0af68, 0x7dcfff,
                0x9ece6a, 0xa07a52, 0xbb9af7, 0x445a3c
            ),
        ),
        (
            "Everforest",
            theme!(
                "", 0xdbbc7f, 0xd3c6aa, 0x859289, 0x3d484d, 0x475258, 0xe67e80, 0xe69875, 0x7fbbb3,
                0xa7c080, 0x9c7a5c, 0xd699b6, 0x4a5a48
            ),
        ),
        // Deep woods. `accent` drives focused borders, titles, the open-scene
        // marker and the progress bar, so it has to be green or the whole
        // interface reads as whatever colour it is. Cyan is kept for the two
        // places it stays rare — the break-time moon and the flowers.
        (
            "Lost Forest",
            theme!(
                "", 0x7cc47f, 0xcdddc6, 0x5f7a63, 0x24332a, 0x2e4436, 0xd1745e, 0xd9c87e, 0x7fd4c8,
                0x4a8250, 0x5f4c3a, 0x68c2b4, 0x2f4a37
            ),
        ),
    ];
    v.iter_mut().for_each(|(n, t)| t.name = (*n).to_string());
    v.into_iter().map(|(_, t)| t).collect()
}

pub fn default_theme() -> Theme {
    presets().remove(0)
}

pub fn config_path() -> PathBuf {
    crate::home()
        .join(".config")
        .join("grimoire")
        .join("theme.toml")
}

/// Load the saved theme. A named preset is looked up fresh so preset tweaks
/// reach existing users; "Custom" is read swatch by swatch.
pub fn load() -> Theme {
    let Ok(s) = std::fs::read_to_string(config_path()) else {
        return default_theme();
    };
    let mut name = String::new();
    let mut fields: Vec<(String, String)> = Vec::new();
    for line in s.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim().to_string(), v.trim().trim_matches('"').to_string());
        if k == "name" {
            name = v;
        } else {
            fields.push((k, v));
        }
    }

    if name != "Custom" {
        if let Some(t) = presets().into_iter().find(|t| t.name == name) {
            return t;
        }
        return default_theme();
    }

    let mut t = default_theme();
    t.name = "Custom".into();
    for (k, v) in fields {
        if let (Some(i), Some(c)) = (ROLES.iter().position(|r| *r == k), parse_hex(&v)) {
            t.set_role(i, c);
        }
    }
    t
}

pub fn save(t: &Theme) -> std::io::Result<()> {
    let path = config_path();
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut out = format!("name = \"{}\"\n", t.name);
    if t.name == "Custom" {
        for (i, role) in ROLES.iter().enumerate() {
            out.push_str(&format!("{role} = \"{}\"\n", hex_of(t.role(i))));
        }
    }
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_is_named_and_distinct() {
        let p = presets();
        assert_eq!(p.len(), 9);
        assert!(p.iter().all(|t| !t.name.is_empty()));
        for i in 0..p.len() {
            for j in i + 1..p.len() {
                assert_ne!(p[i].name, p[j].name);
                assert_ne!(p[i].accent, p[j].accent, "{} vs {}", p[i].name, p[j].name);
            }
        }
    }

    /// Lost Forest has to stay green-led with cyan only as a rare contrast —
    /// cheap guard against the palette drifting back to cyan-dominant.
    #[test]
    fn lost_forest_is_green_led_with_cyan_only_as_contrast() {
        let t = presets()
            .into_iter()
            .find(|t| t.name == "Lost Forest")
            .expect("Lost Forest missing");

        let chan = |c: Color| match c {
            Color::Rgb(r, g, b) => (r as i32, g as i32, b as i32),
            _ => panic!("themes must be truecolor"),
        };

        // Accent is what the eye sees most, so it must read green: green
        // clearly ahead of red, and NOT cyan (blue must stay below green).
        let (r, g, b) = chan(t.accent);
        assert!(g > r + 30, "accent {:?} is not green enough", t.accent);
        assert!(g > b + 30, "accent {:?} has drifted to cyan", t.accent);

        // Every structural and woodland role is green-dominant.
        for (name, c) in [
            ("text", t.text),
            ("dim", t.dim),
            ("border", t.border),
            ("selection", t.sel),
            ("foliage", t.foliage),
            ("turf", t.turf),
        ] {
            let (r, g, b) = chan(c);
            assert!(g > r && g > b, "{name} {c:?} should be green-dominant");
        }

        // Foliage must sit darker than the accent or the tree pane blurs into it.
        let lum = |c| {
            let (r, g, b) = chan(c);
            r + g + b
        };
        assert!(
            lum(t.foliage) < lum(t.accent),
            "foliage should be darker than accent"
        );

        // Cyan survives, but only where it is rare: the moon and the flowers.
        for (name, c) in [("moon", t.moon), ("bloom", t.bloom)] {
            let (r, g, b) = chan(c);
            assert!(b > r + 30 && g > r + 30, "{name} {c:?} should read cyan");
        }

        // Warning has to stay warm or it vanishes into the trees.
        let (r, g, b) = chan(t.warn);
        assert!(r > g && r > b, "warning {:?} should be warm", t.warn);
    }

    #[test]
    fn hex_round_trips() {
        for t in presets() {
            for i in 0..ROLES.len() {
                let c = t.role(i);
                assert_eq!(parse_hex(&hex_of(c)), Some(c));
            }
        }
    }

    #[test]
    fn bad_hex_is_rejected() {
        for bad in ["", "#fff", "#gggggg", "12345", "#1234567", "nope"] {
            assert!(parse_hex(bad).is_none(), "{bad} should not parse");
        }
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn role_get_set_covers_every_index() {
        let mut t = default_theme();
        for i in 0..ROLES.len() {
            let c = parse_hex("#010203").unwrap();
            t.set_role(i, c);
            assert_eq!(t.role(i), c, "role {} ({}) did not round-trip", i, ROLES[i]);
        }
    }
}
