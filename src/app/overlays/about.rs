//! About Grimoire: which version this is, and who made it — with a way to
//! the studio's site.

use super::*;
use ratatui::layout::Alignment;

/// The studio that makes Grimoire, and where to find it.
pub(crate) const STUDIO: &str = "Catfinity Studios";
pub(crate) const STUDIO_URL: &str = "https://catfinity.com";
/// Where a donation goes.
pub(crate) const KOFI_URL: &str = "https://ko-fi.com/F2F21E0DK0";
const SITE: &str = "grimoiretui.com";
const SOURCE: &str = "github.com/kelsierbot/GrimoireTUI";

impl App {
    pub(crate) fn open_about(&mut self) {
        self.overlay = Overlay::About;
    }

    pub(super) fn on_about_key(&mut self, key: Key) {
        match key {
            Key::Enter | Key::Char('o') => self.open_studio_site(),
            Key::Char('d') => self.overlay = Overlay::Donate,
            Key::Char('l') => self.open_help(Some("license")),
            Key::Esc | Key::Char('q') => self.overlay = Overlay::None,
            _ => {}
        }
    }

    pub(super) fn on_donate_key(&mut self, key: Key) {
        match key {
            Key::Enter | Key::Char('o') => self.open_kofi(),
            Key::Esc | Key::Char('q') => self.overlay = Overlay::None,
            _ => {}
        }
    }

    /// The Ko-fi page, in the browser.
    pub(crate) fn open_kofi(&mut self) {
        self.msg = match open_url(KOFI_URL) {
            Ok(()) => "opening Ko-fi — thank you".to_string(),
            Err(e) => format!("couldn't open a browser ({e}) — it's {KOFI_URL}"),
        };
        self.last_opened = Some(KOFI_URL.to_string());
    }

    /// The studio's site, in the browser; a click on the link lands here too.
    pub(crate) fn open_studio_site(&mut self) {
        self.msg = match open_url(STUDIO_URL) {
            Ok(()) => format!("opening {}", STUDIO_URL.trim_start_matches("https://")),
            Err(e) => format!("couldn't open a browser ({e}) — it's {STUDIO_URL}"),
        };
        self.last_opened = Some(STUDIO_URL.to_string());
    }
}

/// Hand `url` to the system's browser, detached, so Grimoire doesn't wait on
/// it. Tests never launch anything; they read `App::last_opened`.
fn open_url(url: &str) -> std::io::Result<()> {
    if cfg!(test) {
        return Ok(());
    }
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

pub(super) fn draw_about(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let version = env!("CARGO_PKG_VERSION");
    let dim = Style::default().fg(t.dim);
    let text = Style::default().fg(t.text);
    let accent = Style::default().fg(t.accent);
    let centre =
        |s: String, st: Style| Line::from(Span::styled(s, st)).alignment(Alignment::Center);

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        centre(
            "G R I M O I R E".into(),
            accent.add_modifier(Modifier::BOLD),
        ),
        centre("a terminal writing desk for novels".into(), dim),
        centre(format!("version {version}"), text),
        Line::from(""),
        centre("Made by".into(), dim),
        centre(STUDIO.into(), text.add_modifier(Modifier::BOLD)),
        centre(
            format!("↗ {}", STUDIO_URL.trim_start_matches("https://")),
            accent.add_modifier(Modifier::UNDERLINED),
        ),
        Line::from(""),
        centre(SITE.into(), dim),
        centre(SOURCE.into(), dim),
        Line::from(""),
        centre("MIT License: free for commercial use,".into(), dim),
        centre("with credit. Donations welcome, never needed.".into(), dim),
        Line::from(""),
        hint_line(" ↵ catfinity.com   d donate   l license   esc close", t)
            .alignment(Alignment::Center),
    ];
    // The spellbook heads it when there's room.
    let art_h = if area.height >= lines.len() as u16 + 13 {
        crate::ui::BOOK_ART.len() as u16 + 1
    } else {
        0
    };
    let w = 56u16.min(area.width);
    let box_area = centred(area, w, lines.len() as u16 + art_h + 2);
    f.render_widget(Clear, box_area);
    let block = pane_block("ABOUT", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    if art_h > 0 {
        let art = Rect::new(inner.x, inner.y + 1, inner.width, art_h - 1);
        crate::ui::draw_book_art(f, art, t, app.frame);
    }
    let body = Rect::new(
        inner.x,
        inner.y + art_h,
        inner.width,
        inner.height.saturating_sub(art_h),
    );
    // Where the link sits, for a click.
    let link_row = 7u16;
    app.about_link
        .set(Rect::new(body.x, body.y + link_row, body.width, 1));
    lines.truncate(body.height as usize);
    f.render_widget(Paragraph::new(lines), body);
}

pub(super) fn draw_donate(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let dim = Style::default().fg(t.dim);
    let text = Style::default().fg(t.text);
    let accent = Style::default().fg(t.accent);
    let centre = |s: &str, st: Style| {
        Line::from(Span::styled(s.to_string(), st)).alignment(Alignment::Center)
    };
    let lines: Vec<Line> = vec![
        Line::from(""),
        centre("♥  Support Grimoire", accent.add_modifier(Modifier::BOLD)),
        Line::from(""),
        centre("Grimoire is free, and a donation is never", text),
        centre("required. But it is appreciated: it goes", text),
        centre("toward Grimoire's development.", text),
        Line::from(""),
        centre(
            "↗ ko-fi.com/F2F21E0DK0",
            accent.add_modifier(Modifier::UNDERLINED),
        ),
        Line::from(""),
        centre("Thank you for writing with it.", dim),
        Line::from(""),
        hint_line(" ↵ open Ko-fi   esc close", t).alignment(Alignment::Center),
    ];
    let box_area = centred(area, 52u16.min(area.width), lines.len() as u16 + 2);
    f.render_widget(Clear, box_area);
    let block = pane_block("DONATE", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    // Where the link sits, for a click.
    app.about_link
        .set(Rect::new(inner.x, inner.y + 7, inner.width, 1));
    f.render_widget(Paragraph::new(lines), inner);
}
