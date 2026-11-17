//! Where this book is kept: the folder it is, what's in it, whether there's
//! a copy of it anywhere, and the things to do about that. A book is a
//! folder of plain files, which is easy to lose track of; this is the one
//! place that says where it is and makes it one file when one file is wanted.

use super::*;
use grimoire_core::shelf;

/// What can be done from the box, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeptRow {
    OpenFolder,
    SaveCopy,
    ShowCopies,
    MoveHome,
}

impl KeptRow {
    fn label(self) -> &'static str {
        match self {
            KeptRow::OpenFolder => "Open the folder",
            KeptRow::SaveCopy => "Save a copy: the whole book as one file",
            KeptRow::ShowCopies => "Show the copies",
            KeptRow::MoveHome => "Move it to your books folder",
        }
    }

    fn key(self) -> char {
        match self {
            KeptRow::OpenFolder => 'o',
            KeptRow::SaveCopy => 's',
            KeptRow::ShowCopies => 'c',
            KeptRow::MoveHome => 'm',
        }
    }
}

impl App {
    /// Is this book in the books folder already?
    fn kept_at_home(&self) -> bool {
        let real = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        real(&self.project.root).parent() == Some(real(&self.books_home).as_path())
    }

    pub(crate) fn kept_rows(&self) -> Vec<KeptRow> {
        let mut rows = vec![KeptRow::OpenFolder, KeptRow::SaveCopy];
        if shelf::copies_dir(&self.books_home).is_dir() {
            rows.push(KeptRow::ShowCopies);
        }
        if !self.kept_at_home() {
            rows.push(KeptRow::MoveHome);
        }
        rows
    }

    pub fn open_kept(&mut self) {
        let facts = shelf::facts(&self.project.root);
        self.overlay = Overlay::Kept {
            sel: 0,
            files: facts.files,
            bytes: facts.bytes,
        };
    }

    pub(super) fn on_kept_key(&mut self, key: Key) {
        let rows = self.kept_rows();
        let Overlay::Kept { sel, .. } = &mut self.overlay else {
            return;
        };
        if list_nav(key, sel, rows.len(), LIST) {
            return;
        }
        let chosen = match key {
            Key::Enter | Key::Char(' ') => rows.get(*sel).copied(),
            Key::Char(c) => rows
                .iter()
                .copied()
                .find(|r| r.key() == c.to_ascii_lowercase()),
            Key::Esc => {
                self.overlay = Overlay::None;
                return;
            }
            _ => None,
        };
        match chosen {
            Some(KeptRow::OpenFolder) => {
                let root = self.project.root.clone();
                self.show_in_file_manager(&root, "the book's folder");
            }
            Some(KeptRow::SaveCopy) => self.save_copy(),
            Some(KeptRow::ShowCopies) => {
                let dir = shelf::copies_dir(&self.books_home);
                self.show_in_file_manager(&dir, "your copies");
            }
            Some(KeptRow::MoveHome) => self.move_book_home(),
            None => {}
        }
    }

    fn show_in_file_manager(&mut self, dir: &Path, what: &str) {
        self.msg = match super::about::open_url(&dir.to_string_lossy()) {
            Ok(()) => format!("opened {what}"),
            Err(_) => format!("couldn't open it here · it's {}", books::pretty(dir)),
        };
    }

    /// The whole book as one file, in the copies folder, named for the book
    /// and today. Everything is saved first, so the copy is the book as it
    /// stands.
    pub fn save_copy(&mut self) {
        self.flush();
        if !self.commit_saves() {
            return;
        }
        let into = shelf::copies_dir(&self.books_home);
        let today = chrono::Local::now().date_naive();
        self.msg =
            match shelf::save_copy(&self.project.root, &self.project.meta.title, &into, today) {
                Ok(file) => format!(
                    "saved a copy: {} · in {}",
                    file.file_name().unwrap_or_default().to_string_lossy(),
                    books::pretty(&into)
                ),
                Err(e) => format!("couldn't save a copy: {e:#}"),
            };
    }

    /// Put the book in the books folder and carry on writing in it there.
    pub fn move_book_home(&mut self) {
        if self.kept_at_home() {
            self.msg = "it's already in your books folder".into();
            return;
        }
        // Everything written and put away first, as when the book is closed.
        if !self.try_quit() {
            self.msg = format!("{} · so the book stays where it is", self.msg);
            return;
        }
        let from = self.project.root.clone();
        let home = self.books_home.clone();
        let dest = match shelf::move_to(&from, &home) {
            Ok(d) => d,
            Err(e) => {
                self.msg = format!("{e:#}");
                return;
            }
        };
        self.overlay = Overlay::None;
        match self.load_book(&dest) {
            Ok(()) => {
                self.msg = format!("moved · this book now lives in {}", books::pretty(&dest));
            }
            Err(why) => {
                // It moved but won't open from there: put it back, so the
                // book that's open is still a book that's there.
                let _ = fs::rename(&dest, &from);
                self.msg = format!("{why} · so it's back where it was");
            }
        }
    }
}

/// "416 KB", "2.1 MB".
fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1_048_576 => format!("{:.1} MB", b as f64 / 1_048_576.0),
        b if b >= 1024 => format!("{} KB", b / 1024),
        b => format!("{b} bytes"),
    }
}

pub(super) fn draw_kept(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Kept { sel, files, bytes } = &app.overlay else {
        return;
    };
    let rows = app.kept_rows();
    let width = 66.min(area.width.saturating_sub(2)).max(30);
    let box_area = centred(area, width, rows.len() as u16 + 14);
    f.render_widget(Clear, box_area);
    let block = pane_block("WHERE THIS BOOK IS KEPT", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let iw = inner.width as usize;
    let dim = Style::default().fg(t.dim);
    let text = Style::default().fg(t.text);

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            format!(
                " {}",
                truncate(&app.project.meta.title, iw.saturating_sub(2))
            ),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(" {}", short_path(&app.project.root, iw.saturating_sub(2))),
            text,
        )),
        Line::from(""),
    ];
    let what = format!(
        "A folder of plain text files: {} file{}, {}. Every scene is its own file, so any editor can read it.",
        thousands(*files),
        if *files == 1 { "" } else { "s" },
        size(*bytes)
    );
    for l in wrap_to(&what, iw.saturating_sub(2)) {
        lines.push(Line::from(Span::styled(format!(" {l}"), dim)));
    }
    // Is it anywhere but here?
    let synced = match app.cloud {
        Some(c) => (format!("Synced by {}.", c.label()), dim),
        None => (
            "Only on this computer: it isn't in a synced folder.".to_string(),
            Style::default().fg(t.warn),
        ),
    };
    lines.push(Line::from(Span::styled(format!(" {}", synced.0), synced.1)));
    let copies = shelf::copies_dir(&app.books_home);
    let last = shelf::newest_copy(&copies, &app.project.meta.title, &app.project.root);
    lines.push(Line::from(Span::styled(
        match last {
            Some((_, when)) => {
                let dt: chrono::DateTime<chrono::Local> = when.into();
                format!(" Last copy saved: {}.", when_label(dt).to_lowercase())
            }
            None => " No copy has been saved yet.".to_string(),
        },
        if last.is_some() {
            dim
        } else {
            Style::default().fg(t.warn)
        },
    )));
    lines.push(Line::from(""));

    let sel = (*sel).min(rows.len().saturating_sub(1));
    for (i, row) in rows.iter().enumerate() {
        let on = i == sel;
        let label = row.label();
        let pad = iw.saturating_sub(3 + label.chars().count() + 2);
        let l = Line::from(vec![
            Span::styled(
                if on { " ▸ " } else { "   " },
                Style::default().fg(t.accent),
            ),
            Span::styled(
                label,
                Style::default().fg(if on { t.accent } else { t.text }),
            ),
            Span::raw(" ".repeat(pad)),
            Span::styled(row.key().to_string(), Style::default().fg(t.sun)),
            Span::raw(" "),
        ]);
        lines.push(if on {
            l.style(Style::default().bg(t.sel))
        } else {
            l
        });
    }
    lines.push(Line::from(""));
    let says = match rows.get(sel) {
        Some(KeptRow::OpenFolder) => "Shows the book's folder in your file manager.".to_string(),
        Some(KeptRow::SaveCopy) => format!(
            "One .zip of everything, named for the book and today, in {}. Keep it somewhere safe or send it: Grimoire on any computer opens it.",
            books::pretty(&copies)
        ),
        Some(KeptRow::ShowCopies) => "Opens the folder your copies are saved in.".to_string(),
        Some(KeptRow::MoveHome) => format!(
            "Puts this book in {}, with your other books. Nothing in it changes.",
            books::pretty(&app.books_home)
        ),
        None => String::new(),
    };
    let mut said = wrap_to(&says, iw.saturating_sub(2));
    said.truncate(3);
    while said.len() < 3 {
        said.push(String::new());
    }
    for l in said {
        lines.push(Line::from(Span::styled(
            format!(" {l}"),
            Style::default().fg(t.dim).add_modifier(Modifier::ITALIC),
        )));
    }
    while lines.len() < (inner.height as usize).saturating_sub(1) {
        lines.push(Line::from(""));
    }
    lines.truncate((inner.height as usize).saturating_sub(1));
    lines.push(hint_line(" j/k move   ↵ do it   esc close", t));
    f.render_widget(Paragraph::new(lines), inner);
}

/// Break `text` into lines no wider than `width`, at spaces.
fn wrap_to(text: &str, width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let len = line.chars().count();
        if len > 0 && len + 1 + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// `p` as ~/…, cut down from the left to `width` characters.
fn short_path(p: &Path, width: usize) -> String {
    let s = books::pretty(p);
    let n = s.chars().count();
    if n <= width {
        s
    } else {
        format!("…{}", s.chars().skip(n + 1 - width).collect::<String>())
    }
}
