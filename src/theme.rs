//! Colour themes.
//!
//! Nineteen presets drawn from the palettes people already run in their
//! terminals — and one rainbow — plus a custom slot. A theme is a flat set of named roles — no
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
    /// The Pomodoro world to draw, by the name of the preset it belongs to;
    /// `None` is the theme's own ([`Theme::world`]). Only Custom sets it.
    pub pomodoro: Option<String>,
    /// The Visualizer look, the same way ([`Theme::look`]).
    pub visualizer: Option<String>,
}

/// What Custom draws when it hasn't chosen: the glade the house theme has,
/// and the Visualizer as it first was.
pub const CUSTOM_WORLD: &str = "Lost Forest";
pub const ORIGINAL_LOOK: &str = "Original";

/// The Pomodoro worlds a Custom theme can borrow: every preset's, by name.
pub fn pomodoro_choices() -> Vec<String> {
    presets().into_iter().map(|t| t.name).collect()
}

/// The Visualizer looks a Custom theme can borrow: the original, then every
/// preset's, by name.
pub fn visualizer_choices() -> Vec<String> {
    std::iter::once(ORIGINAL_LOOK.to_string())
        .chain(presets().into_iter().map(|t| t.name))
        .collect()
}

/// The choice after (or before) `current`, round the list.
pub fn step_choice(choices: &[String], current: &str, forward: bool) -> String {
    let n = choices.len().max(1);
    let i = choices.iter().position(|c| c == current).unwrap_or(0);
    let j = if forward {
        (i + 1) % n
    } else {
        (i + n - 1) % n
    };
    choices.get(j).cloned().unwrap_or_default()
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
    /// Whose Pomodoro world this theme draws: its pick, else its own (a
    /// preset's name), and for Custom the glade.
    pub fn world(&self) -> &str {
        match &self.pomodoro {
            Some(p) => p,
            None if self.name == "Custom" => CUSTOM_WORLD,
            None => &self.name,
        }
    }

    /// Whose Visualizer look this theme draws: its pick, else its own, and
    /// for Custom the original.
    pub fn look(&self) -> &str {
        match &self.visualizer {
            Some(v) => v,
            None if self.name == "Custom" => ORIGINAL_LOOK,
            None => &self.name,
        }
    }

    /// Choose the Pomodoro world; the default is stored as no choice at all.
    pub fn pick_world(&mut self, name: String) {
        self.pomodoro = (name != CUSTOM_WORLD).then_some(name);
    }

    /// Choose the Visualizer look; the original is stored as no choice.
    pub fn pick_look(&mut self, name: String) {
        self.visualizer = (name != ORIGINAL_LOOK).then_some(name);
    }

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

/// The one preset that isn't a flat palette.
pub const RAINBOW: &str = "Rainbow";

impl Theme {
    /// Drawn as a spectrum rather than in its accent: see `ui::paint_rainbow`.
    pub fn is_rainbow(&self) -> bool {
        self.name == RAINBOW
    }
}

/// A colour on the wheel (`hue` in degrees), bright enough to read on a dark
/// terminal without shouting.
pub fn hue(hue: f32) -> Color {
    let (s, l) = (0.9_f32, 0.68_f32);
    let h = hue.rem_euclid(360.0) / 60.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::Rgb(to(r), to(g), to(b))
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
            pomodoro: None,
            visualizer: None,
        }
    };
}

/// The presets. Order is the picker order.
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
        // Rosé Pine: rose as the accent, pine for the trees (it has no green),
        // foam for the moon, iris for the flowers.
        (
            "Rosé Pine",
            theme!(
                "", 0xebbcba, 0xe0def4, 0x6e6a86, 0x403d52, 0x524f67, 0xeb6f92, 0xf6c177, 0x9ccfd8,
                0x31748f, 0x8f6f66, 0xc4a7e7, 0x2a3f4a
            ),
        ),
        (
            "One Dark",
            theme!(
                "", 0x61afef, 0xabb2bf, 0x5c6370, 0x3e4451, 0x3e4451, 0xe06c75, 0xe5c07b, 0x56b6c2,
                0x98c379, 0x9a7550, 0xc678dd, 0x3f5238
            ),
        ),
        // Monokai's pink leads, so the warning takes its orange.
        (
            "Monokai",
            theme!(
                "", 0xf92672, 0xf8f8f2, 0x75715e, 0x49483e, 0x49483e, 0xfd971f, 0xe6db74, 0x66d9ef,
                0xa6e22e, 0x8f6f3e, 0xae81ff, 0x4a5a2a
            ),
        ),
        (
            "Kanagawa",
            theme!(
                "", 0x7e9cd8, 0xdcd7ba, 0x727169, 0x363646, 0x2d4f67, 0xe46876, 0xe6c384, 0x7fb4ca,
                0x98bb6c, 0x8a6d4b, 0xd27e99, 0x3a4a34
            ),
        ),
        (
            "Ayu Mirage",
            theme!(
                "", 0xffcc66, 0xcccac2, 0x707a8c, 0x363c4a, 0x2f3b54, 0xff6666, 0xffd173, 0x73d0ff,
                0x87d96c, 0xa37a4c, 0xdfbfff, 0x3b4d35
            ),
        ),
        (
            "Night Owl",
            theme!(
                "", 0x7fdbca, 0xd6deeb, 0x637777, 0x1d3b53, 0x1d3b53, 0xef5350, 0xecc48d, 0x82aaff,
                0xaddb67, 0x9c7656, 0xc792ea, 0x2e4a3a
            ),
        ),
        (
            "Material Palenight",
            theme!(
                "", 0xc792ea, 0xa6accd, 0x676e95, 0x3a3f58, 0x444267, 0xf07178, 0xffcb6b, 0x89ddff,
                0xc3e88d, 0xa07b5c, 0xff9cac, 0x3d4a3a
            ),
        ),
        // Neon on a purple night: the ground is the grid's own violet.
        (
            "Synthwave '84",
            theme!(
                "", 0xff7edb, 0xf4eee4, 0x848bbd, 0x495495, 0x463465, 0xfe4450, 0xfede5d, 0x36f9f6,
                0x72f1b8, 0xb0735a, 0xf97e72, 0x34294f
            ),
        ),
        (
            "GitHub Dark",
            theme!(
                "", 0x58a6ff, 0xe6edf3, 0x7d8590, 0x30363d, 0x1c3a5e, 0xf85149, 0xd29922, 0x79c0ff,
                0x3fb950, 0x8b6a4a, 0xd2a8ff, 0x2a4a32
            ),
        ),
        // Every role its own hue, red to violet, so the clearing is a rainbow
        // by itself; the accent is then repainted as a slow-moving spectrum
        // wherever it's drawn (see `ui::paint_rainbow`).
        (
            RAINBOW,
            theme!(
                "", 0xa78bff, 0xf2f0ff, 0x8f8aa8, 0x4a4560, 0x3d3560, 0xff5f5f, 0xffd23c, 0x5fd7ff,
                0x5fe07a, 0xff9a3c, 0xff6ec7, 0x6a55d8
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
    load_from(&config_path())
}

/// [`load`], from `path`.
pub fn load_from(path: &std::path::Path) -> Theme {
    let Ok(s) = std::fs::read_to_string(path) else {
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
        } else if k == "pomodoro" && pomodoro_choices().contains(&v) {
            // A name no preset has any more is no choice: the default.
            t.pick_world(v);
        } else if k == "visualizer" && visualizer_choices().contains(&v) {
            t.pick_look(v);
        }
    }
    t
}

pub fn save(t: &Theme) -> std::io::Result<()> {
    save_to(&config_path(), t)
}

/// [`save`], to `path`.
pub fn save_to(path: &std::path::Path, t: &Theme) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut out = format!("name = \"{}\"\n", t.name);
    if t.name == "Custom" {
        for (i, role) in ROLES.iter().enumerate() {
            out.push_str(&format!("{role} = \"{}\"\n", hex_of(t.role(i))));
        }
        if let Some(p) = &t.pomodoro {
            out.push_str(&format!("pomodoro = \"{p}\"\n"));
        }
        if let Some(v) = &t.visualizer {
            out.push_str(&format!("visualizer = \"{v}\"\n"));
        }
    }
    grimoire_core::atomic::write_io(path, out.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("grimoire-theme-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("theme.toml")
    }

    #[test]
    fn a_custom_themes_picks_survive_saving() {
        let path = scratch("picks");
        let mut t = default_theme();
        t.name = "Custom".into();
        t.pick_world("Kanagawa".into());
        t.pick_look("Synthwave '84".into());
        save_to(&path, &t).unwrap();
        let back = load_from(&path);
        assert_eq!(back.world(), "Kanagawa");
        assert_eq!(back.look(), "Synthwave '84");
        assert_eq!(back, t);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_older_custom_theme_loads_as_it_always_did() {
        let path = scratch("old");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = String::from("name = \"Custom\"\n");
        for (i, role) in ROLES.iter().enumerate() {
            file.push_str(&format!(
                "{role} = \"{}\"\n",
                hex_of(default_theme().role(i))
            ));
        }
        std::fs::write(&path, &file).unwrap();
        let t = load_from(&path);
        assert_eq!(
            (t.pomodoro.as_deref(), t.visualizer.as_deref()),
            (None, None)
        );
        assert_eq!((t.world(), t.look()), (CUSTOM_WORLD, ORIGINAL_LOOK));
        // A name no preset has any more is no pick at all.
        std::fs::write(
            &path,
            file + "pomodoro = \"Atlantis\"\nvisualizer = \"Nope\"\n",
        )
        .unwrap();
        let t = load_from(&path);
        assert_eq!((t.world(), t.look()), (CUSTOM_WORLD, ORIGINAL_LOOK));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn presets_draw_their_own_world_and_look() {
        for p in presets() {
            assert_eq!(
                (p.pomodoro.as_deref(), p.visualizer.as_deref()),
                (None, None)
            );
            assert_eq!(p.world(), p.name);
            assert_eq!(p.look(), p.name);
        }
        // Picking the default stores no pick; every preset can be borrowed.
        let mut t = default_theme();
        t.name = "Custom".into();
        t.pick_world(CUSTOM_WORLD.into());
        t.pick_look(ORIGINAL_LOOK.into());
        assert_eq!(
            (t.pomodoro.as_deref(), t.visualizer.as_deref()),
            (None, None)
        );
        assert_eq!(pomodoro_choices().len(), presets().len());
        assert_eq!(visualizer_choices().len(), presets().len() + 1);
        let c = visualizer_choices();
        assert_eq!(
            step_choice(&c, ORIGINAL_LOOK, false),
            "Rainbow",
            "round the list"
        );
    }

    #[test]
    fn every_preset_is_named_and_distinct() {
        let p = presets();
        assert_eq!(p.len(), 19);
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

    /// The rainbow's roles really are a spectrum: every colourful role a
    /// different hue, spread right round the wheel.
    #[test]
    fn rainbow_roles_span_the_spectrum() {
        let t = presets()
            .into_iter()
            .find(|t| t.is_rainbow())
            .expect("Rainbow missing");
        let hue_of = |c: Color| {
            let Color::Rgb(r, g, b) = c else {
                panic!("truecolor")
            };
            let (r, g, b) = (r as f32, g as f32, b as f32);
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            let d = max - min;
            let h = if max == r {
                60.0 * ((g - b) / d).rem_euclid(6.0)
            } else if max == g {
                60.0 * ((b - r) / d + 2.0)
            } else {
                60.0 * ((r - g) / d + 4.0)
            };
            (h / 30.0).round() as i32 % 12
        };
        let mut seen: Vec<i32> = [
            t.warn, t.bark, t.sun, t.foliage, t.moon, t.turf, t.accent, t.bloom,
        ]
        .into_iter()
        .map(hue_of)
        .collect();
        seen.sort();
        seen.dedup();
        assert!(seen.len() >= 7, "rainbow roles share hues: {seen:?}");
        assert!(presets().iter().filter(|t| t.is_rainbow()).count() == 1);
    }

    #[test]
    fn the_wheel_goes_round() {
        assert_eq!(hue(0.0), hue(360.0));
        let Color::Rgb(r, g, b) = hue(0.0) else {
            panic!()
        };
        assert!(r > g && r > b, "0° is red");
        let Color::Rgb(r, g, b) = hue(120.0) else {
            panic!()
        };
        assert!(g > r && g > b, "120° is green");
        let Color::Rgb(r, g, b) = hue(240.0) else {
            panic!()
        };
        assert!(b > r && b > g, "240° is blue");
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
