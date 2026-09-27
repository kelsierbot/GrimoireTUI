//! Books: starting a new one, bringing in a draft written elsewhere, and
//! switching to another. The book that's open is saved and put away first,
//! just as quitting would, and the music, the Pomodoro and the look carry
//! across.

use grimoire_core::import;

use super::*;

/// What a typed folder would do, said before ↵ is pressed.
enum Where {
    Nothing,
    /// Free, or an empty folder: a new book goes here.
    Free(PathBuf),
    /// Already a book.
    Book(PathBuf),
    /// A folder with other things in it, or a file that isn't a draft.
    Taken(PathBuf),
    /// A Word, Markdown or text file to bring in as a new book.
    Draft(PathBuf),
}

fn look(beside: &Path, typed: &str) -> Where {
    let Some(p) = books::typed_path(beside, typed) else {
        return Where::Nothing;
    };
    if books::is_book(&p) {
        Where::Book(p)
    } else if import::is_draft(&p) {
        Where::Draft(p)
    } else if p.is_file() || fs::read_dir(&p).is_ok_and(|mut d| d.next().is_some()) {
        Where::Taken(p)
    } else {
        Where::Free(p)
    }
}

/// Typing into a one-line box: printable keys add, Backspace takes away.
fn edit(buf: &mut String, key: Key) {
    match key {
        Key::Char(c) if !c.is_control() && buf.chars().count() < 120 => buf.push(c),
        Key::Backspace => {
            buf.pop();
        }
        _ => {}
    }
}

impl App {
    /// The rows of *Open another book…*: every other book this machine has
    /// had open, newest first. Folder and new-book rows follow them.
    pub fn other_books(&self) -> Vec<PathBuf> {
        let Some(state) = &self.state_file else {
            return Vec::new();
        };
        books::recent(state)
            .into_iter()
            .filter(|p| *p != self.project.root)
            .collect()
    }

    pub(super) fn on_new_book_key(&mut self, key: Key) {
        let Overlay::NewBook { buf } = &mut self.overlay else {
            return;
        };
        match key {
            Key::Enter => {
                let typed = buf.clone();
                self.create_book(&typed);
            }
            Key::Esc => self.overlay = Overlay::None,
            k => edit(buf, k),
        }
    }

    pub(super) fn on_books_key(&mut self, key: Key) {
        let others = self.other_books();
        let Overlay::Books { sel } = &mut self.overlay else {
            return;
        };
        // The books, then "a book in another folder", "bring in a draft" and
        // "a new book".
        let rows = others.len() + 3;
        if list_nav(key, sel, rows, RING) {
            return;
        }
        match key {
            Key::Enter | Key::Char(' ') | Key::Right | Key::Char('l') => {
                let at = *sel;
                if let Some(root) = others.get(at) {
                    let root = root.clone();
                    self.overlay = Overlay::None;
                    let _ = self.open_book(&root);
                } else if at == others.len() {
                    self.overlay = Overlay::BookPath {
                        buf: String::new(),
                        draft: false,
                    };
                } else if at == others.len() + 1 {
                    self.overlay = Overlay::BookPath {
                        buf: String::new(),
                        draft: true,
                    };
                } else {
                    self.overlay = Overlay::NewBook { buf: String::new() };
                }
            }
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
    }

    pub(super) fn on_book_path_key(&mut self, key: Key) {
        let Overlay::BookPath { buf, draft } = &mut self.overlay else {
            return;
        };
        let draft = *draft;
        match key {
            Key::Enter => match look(&self.project.root, buf) {
                Where::Book(root) => {
                    self.overlay = Overlay::None;
                    let _ = self.open_book(&root);
                }
                Where::Draft(file) => self.bring_in(&file),
                Where::Nothing => {}
                Where::Free(p) | Where::Taken(p) => {
                    self.msg = if draft {
                        format!(
                            "{} isn't a draft Grimoire can read: a Word file (.docx), Markdown (.md) or text (.txt)",
                            books::pretty(&p)
                        )
                    } else {
                        format!("there's no book in {}", books::pretty(&p))
                    };
                }
            },
            Key::Esc => self.overlay = Overlay::None,
            k => edit(buf, k),
        }
    }

    /// Start a whole new book from what was typed, and switch to it. It opens
    /// on the page that shows how a book is laid out.
    fn create_book(&mut self, typed: &str) {
        let root = match look(&self.project.root, typed) {
            Where::Nothing => return,
            Where::Taken(p) => {
                self.msg = format!(
                    "{} already has other things in it · try another name",
                    books::pretty(&p)
                );
                return;
            }
            Where::Book(p) => {
                self.overlay = Overlay::None;
                if self.open_book(&p) {
                    self.msg = format!(
                        "there was already a book at {}, so here it is",
                        books::pretty(&p)
                    );
                }
                return;
            }
            Where::Draft(file) => {
                self.bring_in(&file);
                return;
            }
            Where::Free(p) => p,
        };
        if let Err(e) = fs::create_dir_all(&root)
            .map_err(anyhow::Error::from)
            .and_then(|()| project::scaffold(&root))
        {
            self.msg = format!("couldn't start a book at {}: {e:#}", books::pretty(&root));
            return;
        }
        self.overlay = Overlay::None;
        if self.open_book(&root) {
            self.msg = format!(
                "{} is ready · this first page shows how a book is laid out · Tab to read it, Esc for the menu",
                self.project.meta.title
            );
        }
    }

    /// Bring in the draft at `file` as a new book beside the one that's open,
    /// and switch to it.
    fn bring_in(&mut self, file: &Path) {
        let draft = match import::read(file) {
            Ok(d) => d,
            Err(e) => {
                self.msg = format!("{e:#}");
                return;
            }
        };
        let here = self.project.root.clone();
        let root = books::beside(here.parent().unwrap_or(&here), &draft.title);
        if let Err(e) = import::make_book(&root, &draft) {
            self.msg = format!("couldn't bring it in: {e:#}");
            return;
        }
        self.overlay = Overlay::None;
        if self.open_book(&root) {
            self.msg = format!(
                "brought in {}: {}{} · it lives in {}",
                draft.title,
                books::tally(&draft),
                if draft.before.trim().is_empty() {
                    ""
                } else {
                    " · what came before the first chapter is in Notes"
                },
                books::pretty(&root)
            );
        }
    }

    /// Put this book away and open the one at `root` in its place. Anything
    /// that stops it (a save that can't happen, a folder that isn't a book)
    /// leaves this book open and says why. True if it switched.
    pub fn open_book(&mut self, root: &Path) -> bool {
        match self.switch_book(root) {
            Ok(switched) => switched,
            Err(why) => {
                self.msg = why;
                false
            }
        }
    }

    fn switch_book(&mut self, root: &Path) -> Result<bool, String> {
        let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let here = fs::canonicalize(&self.project.root).unwrap_or(self.project.root.clone());
        if root == here {
            self.msg = "that's the book you have open".into();
            return Ok(false);
        }
        if !books::is_book(&root) {
            return Err(format!(
                "{} isn't a book: there's no manuscript folder in it",
                books::pretty(&root)
            ));
        }
        if !self.try_quit() {
            return Err(format!("{} · so this book stays open for now", self.msg));
        }
        let stranded = project::restore_stranded(&root);
        let updated = project::upgrade(&root).unwrap_or_else(|e| {
            vec![format!(
                "not all of it could be brought up to date yet ({e:#})"
            )]
        });
        let cant = |e: anyhow::Error| format!("couldn't open {}: {e:#}", books::pretty(&root));
        let project = Project::load(&root).map_err(cant)?;
        let setup = Setup {
            // Inert: the music already playing comes across instead.
            music: music::Config {
                enabled: false,
                ..music::Config::default()
            },
            theme: self.theme.clone(),
            settings: Settings {
                spellcheck: self.spell_on,
                icons: self.icons_on,
                line_width: self.line_width,
                typewriter: self.typewriter,
            },
            background: self.background,
        };
        let mut next = App::with(project, setup).map_err(cant)?;
        std::mem::swap(&mut next.music, &mut self.music);
        std::mem::swap(&mut next.viz, &mut self.viz);
        std::mem::swap(&mut next.pomo, &mut self.pomo);
        next.pane_mode = self.pane_mode;
        next.scene_visible = self.scene_visible;
        next.music_visible = self.music_visible;
        next.super_keys = self.super_keys;
        next.screen = self.screen;
        next.frame = self.frame;
        next.state_file = self.state_file.take();
        if self.speller.is_some() {
            next.speller = self.speller.take();
            next.speller_rx = None;
        }
        let left = std::mem::replace(self, next).project.root;
        if let Some(state) = &self.state_file {
            // The one left behind goes on the list too, to come back to.
            books::remember(state, &left);
            books::remember(state, &root);
        }
        if !stranded.is_empty() {
            self.msg = App::stranded_note(&stranded);
        } else if !updated.is_empty() {
            self.msg = format!("book updated: {}", updated.join(" · "));
        } else if self.msg.is_empty() {
            self.msg = format!("opened {}", self.project.meta.title);
        } else {
            self.msg = format!("opened {} · {}", self.project.meta.title, self.msg);
        }
        Ok(true)
    }
}

/// `p` as ~/…, cut down from the left to `width` characters.
fn short_path(p: &Path, width: usize) -> String {
    let s = books::pretty(p);
    let n = s.chars().count();
    if n <= width {
        return s;
    }
    let tail: String = s.chars().skip(n + 1 - width).collect();
    format!("…{tail}")
}

/// One line of typing, with where it will go (or what's there) under it,
/// and the keys that work.
fn typed_box(
    f: &mut Frame,
    area: Rect,
    t: &Theme,
    (title, lead): (&str, &str),
    buf: &str,
    (below, keys): (Line, &str),
) {
    let box_area = centred(area, 64, 9);
    f.render_widget(Clear, box_area);
    let block = pane_block(title, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    // A long path shows its end, where the typing is.
    let room = (inner.width as usize).saturating_sub(5);
    let n = buf.chars().count();
    let shown: String = if n > room {
        format!("…{}", buf.chars().skip(n + 1 - room).collect::<String>())
    } else {
        buf.to_string()
    };
    let lines = vec![
        Line::from(Span::styled(format!(" {lead}"), Style::default().fg(t.dim))),
        Line::from(""),
        Line::from(vec![
            Span::styled(" ▸ ", Style::default().fg(t.accent)),
            Span::styled(shown, Style::default().fg(t.text)),
            Span::styled("█", Style::default().fg(t.accent)),
        ]),
        Line::from(""),
        below,
        Line::from(""),
        hint_line(keys, t),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

pub(super) fn draw_new_book(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::NewBook { buf } = &app.overlay else {
        return;
    };
    let dim = Style::default().fg(t.dim);
    let under = match look(&app.project.root, buf) {
        Where::Nothing => (
            Line::from(Span::styled(
                " parts, chapters and scenes come ready to fill in",
                dim,
            )),
            " type a name   esc cancel",
        ),
        Where::Free(p) => (
            Line::from(vec![
                Span::styled(" it'll live in ", dim),
                Span::styled(short_path(&p, 46), Style::default().fg(t.accent)),
            ]),
            " ↵ start it   esc cancel",
        ),
        Where::Book(p) => (
            Line::from(Span::styled(
                format!(" there's already a book there: {}", books::title(&p)),
                dim,
            )),
            " ↵ open that one   esc cancel",
        ),
        Where::Taken(_) => (
            Line::from(Span::styled(
                " that folder is already there, and has other things in it",
                Style::default().fg(t.warn),
            )),
            " try another name   esc cancel",
        ),
        Where::Draft(p) => draft_line(&p, t),
    };
    typed_box(
        f,
        area,
        t,
        (
            "START A NEW BOOK",
            "What's it called? Or type the path to a draft to bring it in.",
        ),
        buf,
        under,
    );
}

pub(super) fn draw_book_path(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::BookPath { buf, draft } = &app.overlay else {
        return;
    };
    let dim = Style::default().fg(t.dim);
    let under = match look(&app.project.root, buf) {
        Where::Nothing if *draft => (
            Line::from(Span::styled(
                " it comes in split into chapters and scenes",
                dim,
            )),
            " type or drop in a file   esc cancel",
        ),
        Where::Nothing => (Line::from(""), " type a folder   esc cancel"),
        Where::Draft(p) => draft_line(&p, t),
        Where::Book(p) => (
            Line::from(vec![
                Span::styled(" ✓ ", Style::default().fg(t.accent)),
                Span::styled(books::title(&p), Style::default().fg(t.text)),
            ]),
            " ↵ open it   esc cancel",
        ),
        Where::Free(_) | Where::Taken(_) if *draft => (
            Line::from(Span::styled(
                " not a draft yet: a Word file, Markdown or text",
                dim,
            )),
            " keep typing   esc cancel",
        ),
        Where::Free(_) | Where::Taken(_) => (
            Line::from(Span::styled(" no book there yet", dim)),
            " keep typing   esc cancel",
        ),
    };
    typed_box(
        f,
        area,
        t,
        if *draft {
            (
                "BRING IN A DRAFT",
                "A Word file (.docx), Markdown (.md) or text (.txt)",
            )
        } else {
            (
                "OPEN A BOOK",
                "The book's folder, like ~/Documents/My Novel",
            )
        },
        buf,
        under,
    );
}

/// A draft that's there to bring in, by its file name.
fn draft_line<'a>(p: &Path, t: &Theme) -> (Line<'a>, &'static str) {
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    (
        Line::from(vec![
            Span::styled(" ✓ ", Style::default().fg(t.accent)),
            Span::styled(
                name.chars().take(52).collect::<String>(),
                Style::default().fg(t.text),
            ),
        ]),
        " ↵ bring it in as a new book   esc cancel",
    )
}

pub(super) fn draw_books(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Books { sel } = &app.overlay else {
        return;
    };
    let others = app.other_books();
    let mut rows: Vec<(String, String)> = others
        .iter()
        .map(|p| (books::title(p), books::pretty(p)))
        .collect();
    rows.push(("A book in another folder…".into(), String::new()));
    rows.push(("Bring in a draft…".into(), "Word, Markdown or text".into()));
    rows.push(("Start a new book…".into(), String::new()));
    let width = 64;
    let empty = others.is_empty();
    let box_area = centred(area, width, rows.len() as u16 + 5 + u16::from(empty));
    f.render_widget(Clear, box_area);
    let block = pane_block("OPEN ANOTHER BOOK", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    let mut lines: Vec<Line> = Vec::new();
    if empty {
        lines.push(Line::from(Span::styled(
            " No other books yet. Ones you open show up here.",
            Style::default().fg(t.dim),
        )));
    }
    let room = (inner.height as usize)
        .saturating_sub(2 + usize::from(empty))
        .max(1);
    let start = (*sel + 1).saturating_sub(room);
    let name_w = (inner.width as usize).saturating_sub(4) / 2;
    for (i, (name, path)) in rows.iter().enumerate().skip(start).take(room) {
        let on = i == *sel;
        let name: String = name.chars().take(name_w.max(10)).collect();
        lines.push(
            Line::from(vec![
                Span::styled(
                    if on { " ▸ " } else { "   " },
                    Style::default().fg(if on { t.accent } else { t.dim }),
                ),
                Span::styled(
                    format!("{name:<w$} ", w = name_w),
                    Style::default().fg(if on { t.accent } else { t.text }),
                ),
                Span::styled(path.clone(), Style::default().fg(t.dim)),
            ])
            .style(if on {
                Style::default().bg(t.sel)
            } else {
                Style::default()
            }),
        );
    }
    lines.push(Line::from(""));
    lines.push(hint_line(" j/k move   ↵ open   esc close", t));
    f.render_widget(Paragraph::new(lines), inner);
}
