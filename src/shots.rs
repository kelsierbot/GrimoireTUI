//! The README's screenshots, drawn by the real interface.
//!
//! `GRIMOIRE_SHOTS=<file.json> cargo test --locked shots -- --ignored`
//! builds a sample book, drives the desk into each scene below, and writes
//! every frame as cells (symbol, colours, style) for `tools/screenshots.py`
//! to paint. Nothing reads or writes ~/.config.

use super::*;
use crate::app::Setup;
use crate::music::{Repeat, State as MusicState, Track};
use grimoire_core::settings::Settings;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use std::fs;
use std::time::Duration;

const SCENE_ONE: &str = "The rain had stopped an hour before Wren reached the archive, but the \
gravel still held it, every stone slick and bright under the one working lamp. She stood at the \
edge of the lot with her hands in her pockets and counted the windows. Eleven dark. One lit, on \
the third floor, where the salt records were kept.

Oren had promised he would be gone by nine. It was a quarter past ten.

She crossed the lot slowly, the way you cross ice, and let herself in by the loading door. \
Inside, the air tasted of paper and brine. The ledgers lined the walls in their grey jackets, \
each spine stamped with a year and a tide. Somewhere above her a drawer slid shut. %% make the \
drawer matter later %%

\"You're early,\" said a voice from the stairs, which was a lie, and they both knew it.

Wren did not look up. She took the TK from her coat and set it on the counter between them, \
where the light could reach it.

\"Then we'd better start,\" she said.";

const SCENE_TWO: &str = "The third floor smelled of the sea. Oren had left the window open again, \
and the tide tables on the long desk had curled at their corners like leaves. Wren weighted them \
flat with whatever came to hand: a tin of pencils, a brass rule, a stone somebody had brought in \
from the beach and never taken home.

She found the ledger she wanted by its smell before she found it by its year. Salt got into \
everything here, but the old books kept it best, the way a shell keeps the sound. She opened it \
on the desk and turned the pages with the side of her hand, the way her father had taught her, \
so the corners wouldn't tear.

The entries ran in two hands. The first was her father's, square and patient. The second was \
smaller, slanted, in a brown ink that had faded to the colour of tea, and it began in the middle \
of a line, as if whoever wrote it had taken the pen out of his fingers.

\"You're not supposed to have that,\" Oren said from the door.

\"Then you shouldn't have left it where the light could find it.\"";

const CHAPTER_TWO: &str = "By morning the ledgers had been moved. Not far — a shelf to the left, \
a year out of order — but Wren saw it the moment she came in, the way you notice a missing tooth.

She didn't touch them. She made tea on the ring in the back room and drank it standing at the \
window, watching the lot fill with gulls and then empty again when the fish van left. The tide \
was out. The mud shone like a road nobody was allowed to walk on.

When she came back the ledgers were where they belonged, and the brass rule she'd left on the \
desk was lying across the gap where they had been, exactly level, exactly as long as the space \
was wide.

She measured it twice to be sure. Then she sat down, opened the book at the page with the brown \
ink, and began to copy it out word by word, because whatever was happening in the archive, it \
was happening to that page first.";

/// A book called The Salt Archive, with a few scenes written and two people
/// in its notebook. `typo` misspells one word of the first scene.
fn sample_book(tag: &str, typo: bool) -> PathBuf {
    let root = match std::env::var("GRIMOIRE_SHOTS_BOOK") {
        // For tools/screenshots.py's page renders: the book, kept.
        Ok(dir) if tag == "keep" => PathBuf::from(dir),
        _ => std::env::temp_dir().join(format!("grimoire-shots-{tag}-{}", std::process::id())),
    };
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    project::scaffold(&root).unwrap();
    let toml = root.join("novel.toml");
    let meta: String = fs::read_to_string(&toml)
        .unwrap()
        .lines()
        .map(|l| {
            if l.starts_with("title =") {
                "title = \"The Salt Archive\"".to_string()
            } else if l.starts_with("author =") {
                "author = \"A. Writer\"".to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&toml, meta).unwrap();
    // A title page's contact block — plainly fictional.
    grimoire_core::submission::save(
        &root,
        "Avery Marlowe",
        &grimoire_core::submission::Contact {
            legal_name: "Avery M. Marlowe".into(),
            address: vec!["12 Harbour Road".into(), "Saltmarsh".into()],
            phone: "555-0142".into(),
            email: "avery@example.com".into(),
            agent: Vec::new(),
        },
        &grimoire_core::submission::Manuscript::default(),
    )
    .unwrap();
    fs::create_dir_all(root.join(".grimoire")).unwrap();
    fs::write(root.join(".grimoire/progress.toml"), "").unwrap();

    let ch1 = root.join("manuscript/01-Part-One/01-Chapter-One");
    let ch2 = root.join("manuscript/01-Part-One/02-Chapter-Two");
    let write = |path: PathBuf, title: &str, status: &str, synopsis: &str, body: &str| {
        let front = format!(
            "---\ntitle: \"{title}\"\npov: Wren\nstatus: {status}\nsynopsis: {synopsis}\ntarget: 1500\n---\n\n"
        );
        fs::write(path, format!("{front}{body}\n")).unwrap();
    };
    let one = if typo {
        SCENE_ONE.replace("slick and bright", "slick and brigt")
    } else {
        SCENE_ONE.to_string()
    };
    write(
        ch1.join("01-Scene-One.md"),
        "Scene One",
        "draft",
        "Wren comes back to the archive at night.",
        &one,
    );
    write(
        ch1.join("02-Scene-Two.md"),
        "Scene Two",
        "draft",
        "The third floor.",
        SCENE_TWO,
    );
    write(
        ch2.join("01-Scene-One.md"),
        "Scene One",
        "outline",
        "The ledgers have moved.",
        CHAPTER_TWO,
    );
    for (name, what) in [
        (
            "wren",
            "Archivist's daughter. Counts things when she's frightened.",
        ),
        (
            "oren",
            "Keeps the salt records. Never where he said he'd be.",
        ),
    ] {
        let title = format!("{}{}", name[..1].to_uppercase(), &name[1..]);
        fs::write(
            root.join("characters").join(format!("{name}.md")),
            format!("---\ntitle: {title}\n---\n{what}\n"),
        )
        .unwrap();
    }
    root
}

struct Shot {
    app: App,
    term: Terminal<TestBackend>,
    armed: bool,
    root: PathBuf,
}

impl Shot {
    fn new(tag: &str, theme: &str, w: u16, h: u16, typo: bool) -> Shot {
        let root = sample_book(tag, typo);
        let theme = theme::presets()
            .into_iter()
            .find(|t| t.name == theme)
            .unwrap_or_else(|| panic!("no theme {theme}"));
        let setup = Setup {
            music: music::Config {
                enabled: true,
                ..music::Config::default()
            },
            theme,
            settings: Settings::default(),
            background: false,
        };
        let mut app = App::with(Project::load(&root).unwrap(), setup).unwrap();
        // A song playing, repeat on, a day's writing behind it.
        app.music.state = MusicState::Playing(Track {
            title: "Low Tide".into(),
            artist: "The Harbour Lights".into(),
            progress: 131.0,
            duration: 244.0,
            playing: true,
        });
        app.music.modes.repeat = Some(Repeat::All);
        app.music.modes.volume = Some(100);
        app.baseline = app.project.total_words().saturating_sub(173);
        app.pomo.run_for_shots(Duration::from_secs(12 * 60 + 20));
        app.frame = 57;
        let mut s = Shot {
            app,
            term: Terminal::new(TestBackend::new(w, h)).unwrap(),
            armed: false,
            root,
        };
        s.draw();
        s
    }

    fn draw(&mut self) {
        let app = &mut self.app;
        self.term.draw(|f| ui::draw(f, app)).unwrap();
    }

    fn key(&mut self, code: KeyCode, mods: KeyModifiers) {
        on_key(&mut self.app, KeyEvent::new(code, mods), &mut self.armed);
        self.draw();
    }

    fn visualizer(&mut self) {
        self.app.pane_mode = scene::Mode::Visualizer;
        let n = self.app.viz.levels.len();
        let levels: Vec<f32> = (0..n)
            .map(|i| {
                let x = i as f32 / n as f32;
                (0.25 + 0.55 * (x * 9.0).sin().abs() * (1.0 - 0.6 * x) + 0.12 * (x * 23.0).cos())
                    .clamp(0.05, 0.95)
            })
            .collect();
        let peaks = levels.iter().map(|l| (l + 0.14).min(1.0)).collect();
        self.app.viz.set_for_shots(levels, peaks);
        self.draw();
    }

    fn at(&self, needle: &str) -> (u16, u16) {
        let buf = self.term.backend().buffer();
        let n: Vec<char> = needle.chars().collect();
        for y in 0..buf.area.height {
            let row: Vec<char> = (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
                .collect();
            for x in 31..row.len().saturating_sub(n.len()) {
                if row[x..x + n.len()] == n[..] {
                    return (x as u16, y);
                }
            }
        }
        panic!("{needle:?} not on screen");
    }

    fn text_rows(&self) -> Vec<String> {
        let buf = self.term.backend().buffer();
        (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    fn shows(&self, s: &str) -> bool {
        self.text_rows().iter().any(|r| r.contains(s))
    }

    fn row_with(&self, s: &str) -> String {
        self.text_rows()
            .into_iter()
            .find(|r| r.contains(s))
            .unwrap_or_default()
    }

    /// The frame as JSON: `crop` is the part to cut out for a grid, if any.
    fn json(&self, name: &str, crop: Option<Rect>) -> String {
        let buf = self.term.backend().buffer();
        let hex = |c: Color| match c {
            Color::Rgb(r, g, b) => format!("\"#{r:02x}{g:02x}{b:02x}\""),
            _ => "null".into(),
        };
        let mut rows = Vec::new();
        for y in 0..buf.area.height {
            let mut cells = Vec::new();
            for x in 0..buf.area.width {
                let c = &buf[(x, y)];
                let m = c.modifier;
                let style = (m.contains(Modifier::BOLD) as u8)
                    | (m.contains(Modifier::ITALIC) as u8) << 1
                    | (m.contains(Modifier::UNDERLINED) as u8) << 2;
                let sym = c.symbol().replace('\\', "\\\\").replace('"', "\\\"");
                cells.push(format!("[\"{sym}\",{},{},{style}]", hex(c.fg), hex(c.bg)));
            }
            rows.push(format!("[{}]", cells.join(",")));
        }
        let crop = crop.map_or("null".into(), |r| {
            format!("[{},{},{},{}]", r.x, r.y, r.width, r.height)
        });
        format!(
            "{{\"name\":\"{name}\",\"theme\":\"{}\",\"w\":{},\"h\":{},\"crop\":{crop},\"rows\":[{}]}}",
            self.app.theme.name.replace('"', "\\\""),
            buf.area.width,
            buf.area.height,
            rows.join(",")
        )
    }

    /// The Pomodoro/Visualizer pane with its frame.
    fn pane(&self) -> Rect {
        let r = self.app.rect_scene;
        Rect::new(
            r.x.saturating_sub(1),
            r.y.saturating_sub(1),
            r.width + 2,
            r.height + 2,
        )
    }
}

impl Drop for Shot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
#[ignore]
fn shots() {
    let Ok(out) = std::env::var("GRIMOIRE_SHOTS") else {
        return;
    };
    let (w, h) = (150, 44);
    let mut frames = Vec::new();
    let none = KeyModifiers::NONE;

    // The desk, writing, in the house theme.
    let mut s = Shot::new("hero", "Lost Forest", w, h, false);
    s.key(KeyCode::Tab, none);
    frames.push(s.json("desk", None));

    // Rainbow, mid-session.
    let mut s = Shot::new("rainbow", "Rainbow", w, h, false);
    s.key(KeyCode::Tab, none);
    frames.push(s.json("rainbow", None));

    // Custom: Kanagawa's world with Synthwave '84's visualizer.
    let mut s = Shot::new("custom", "Lost Forest", w, h, false);
    s.key(KeyCode::F(9), none);
    for _ in 0..24 {
        if s.shows("● Custom…") {
            break;
        }
        s.key(KeyCode::Up, none);
    }
    s.key(KeyCode::Enter, none);
    assert!(
        matches!(s.app.overlay, crate::app::Overlay::Custom { .. }),
        "the custom editor is open"
    );
    for (row, want) in [("pomodoro", "Kanagawa"), ("visualizer", "Synthwave '84")] {
        for _ in 0..16 {
            if s.shows(&format!("▸ {row}")) {
                break;
            }
            s.key(KeyCode::Down, none);
        }
        for _ in 0..24 {
            if s.row_with(&format!("▸ {row}")).contains(want) {
                break;
            }
            s.key(KeyCode::Right, none);
        }
    }
    s.visualizer();
    frames.push(s.json("custom", None));

    // Focus mode.
    let mut s = Shot::new("focus", "Kanagawa", w, h, false);
    s.key(KeyCode::Tab, none);
    s.app.toggle_focus_mode();
    s.draw();
    frames.push(s.json("focus", None));

    // The menu.
    let mut s = Shot::new("menu", "Tokyo Night", w, h, false);
    s.key(KeyCode::Tab, none);
    s.key(KeyCode::Esc, none);
    frames.push(s.json("menu", None));

    // The help, opened with F1 from the page: on the topic for writing.
    let mut s = Shot::new("help", "Catppuccin Mocha", w, h, false);
    s.key(KeyCode::Tab, none);
    s.key(KeyCode::F(1), none);
    assert!(s.shows("HELP"));
    frames.push(s.json("help", None));

    // Click a misspelt word.
    let mut s = Shot::new("spell", "Rosé Pine", w, h, true);
    s.app.load_speller_now();
    s.key(KeyCode::Tab, none);
    let (x, y) = s.at("brigt");
    s.app.on_click(x + 1, y);
    s.draw();
    frames.push(s.json("spelling", None));

    // The export dialog, author details filled in.
    let mut s = Shot::new("export", "Grimoire", w, h, false);
    s.key(KeyCode::Tab, none);
    s.key(KeyCode::Esc, none);
    for _ in 0..24 {
        if s.shows("▸ Export…") {
            break;
        }
        s.key(KeyCode::Down, none);
    }
    s.key(KeyCode::Enter, none);
    frames.push(s.json("export", None));

    // A Dropbox conflict copy beside its scene, being settled.
    let mut s = Shot::new("conflicts", "Nord", w, h, false);
    let ch1 = s.root.join("manuscript/01-Part-One/01-Chapter-One");
    let theirs = fs::read_to_string(ch1.join("01-Scene-One.md"))
        .unwrap()
        .replace(
            "It was a quarter past ten.",
            "It was nearly eleven, and raining again.",
        );
    fs::write(
        ch1.join("01-Scene-One (Avery's conflicted copy 2026-09-25).md"),
        theirs,
    )
    .unwrap();
    s.app.reload_for_shots();
    s.key(KeyCode::Tab, none);
    s.key(KeyCode::Esc, none);
    for _ in 0..24 {
        if s.shows("▸ Settle conflicts") {
            break;
        }
        s.key(KeyCode::Down, none);
    }
    s.key(KeyCode::Enter, none);
    s.key(KeyCode::Enter, none);
    frames.push(s.json("conflicts", None));

    // A book kept for the page renders (manuscript and paperback PDFs).
    if std::env::var("GRIMOIRE_SHOTS_BOOK").is_ok() {
        let _ = sample_book("keep", false);
    }

    // Every theme's Pomodoro and Visualizer, for the grids.
    for t in theme::presets() {
        let mut s = Shot::new("grid", &t.name, 120, 42, false);
        let pane = s.pane();
        frames.push(s.json(&format!("pomodoro/{}", t.name), Some(pane)));
        s.visualizer();
        let pane = s.pane();
        frames.push(s.json(&format!("visualizer/{}", t.name), Some(pane)));
    }

    fs::write(out, format!("[{}]", frames.join(",\n"))).unwrap();
}
