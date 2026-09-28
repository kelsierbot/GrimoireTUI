//! The Esc menu and its groups: the discoverable way to everything. The
//! first screen is short; each group is one step down, and whatever row is
//! highlighted says in a line what it does.

use super::*;

impl App {
    /// The rows of the menu screen that's up: the first one, or a group.
    pub fn menu_rows(&self) -> Vec<(String, Action)> {
        match &self.overlay {
            Overlay::Sub { sub, .. } => self.submenu(*sub),
            _ => self.menu(),
        }
    }

    /// The highlighted row's action, while the menu is up.
    pub fn menu_highlight(&self) -> Option<Action> {
        let (Overlay::Menu { sel } | Overlay::Sub { sel, .. }) = &self.overlay else {
            return None;
        };
        self.menu_rows().into_iter().nth(*sel).map(|(_, a)| a)
    }

    pub(super) fn on_menu_key(&mut self, key: Key) {
        let sub = match &self.overlay {
            Overlay::Sub { sub, .. } => Some(*sub),
            _ => None,
        };
        let items: Vec<Action> = self.menu_rows().into_iter().map(|(_, a)| a).collect();
        let (Overlay::Menu { sel } | Overlay::Sub { sel, .. }) = &mut self.overlay else {
            return;
        };
        if list_nav(key, sel, items.len(), RING) {
            return;
        }
        match key {
            Key::Enter | Key::Char(' ') | Key::Right | Key::Char('l') => {
                let (at, action) = (*sel, items[*sel].clone());
                if action == Action::MenuBack {
                    self.menu_back(sub);
                    return;
                }
                // A setting switched on or off stays in its group, so the
                // change shows on its row.
                let stay = action.is_setting_toggle();
                self.run_action(action);
                if let Some(sub) = sub
                    && stay
                    && self.overlay == Overlay::None
                {
                    self.overlay = Overlay::Sub { sub, sel: at };
                }
            }
            Key::Esc | Key::Left | Key::Char('h') if sub.is_some() => self.menu_back(sub),
            Key::Esc => self.overlay = Overlay::None,
            _ => {}
        }
    }

    /// Up out of a group, onto the row that led into it.
    fn menu_back(&mut self, from: Option<Sub>) {
        let sel = from
            .and_then(|sub| {
                self.menu()
                    .iter()
                    .position(|(_, a)| *a == Action::Submenu(sub))
            })
            .unwrap_or(0);
        self.overlay = Overlay::Menu { sel };
    }
}

/// What a menu row does, in a line, and the help topic that says more.
pub fn about_row(action: &Action, part: &str) -> (String, &'static str) {
    let (says, topic): (&str, &str) = match action {
        Action::FindAnything => (
            "Type a few letters to jump to any scene or note, or to run anything Grimoire can do.",
            "getting-started",
        ),
        Action::Conflicts => (
            "A sync left two copies of a scene that disagree. Pick which words to keep.",
            "sync",
        ),
        Action::Submenu(Sub::Book) => (
            "Add scenes, chapters and parts, or rename and delete them.",
            "outline",
        ),
        Action::Submenu(Sub::Writing) => (
            "Focus mode, sprints, your notes, and other help while you write.",
            "writing",
        ),
        Action::Submenu(Sub::Settings) => (
            "Themes, music, spellcheck, and how the page and the manuscript look.",
            "settings",
        ),
        Action::Submenu(Sub::Help) => (
            "The help, which version this is, the license, and a way to chip in.",
            "getting-started",
        ),
        Action::Export => (
            "Turn the book into a Word file, a PDF, an EPUB or a paperback, ready for readers.",
            "compile",
        ),
        Action::NewBook => (
            "A new novel, short story or blank book, or bring in a draft you already have. This one is saved first.",
            "books",
        ),
        Action::OpenBook => (
            "Switch to another book you've written in, or open one from its folder.",
            "books",
        ),
        Action::Quit => (
            "Everything is saved first. Next time, this book opens right where you left it.",
            "getting-started",
        ),
        Action::Corkboard => (
            "Every scene as an index card, to see the book at a glance and jot a synopsis, whose eyes, and how far along.",
            "corkboard",
        ),
        Action::NewScene => (
            "A new scene at the end of the chapter you're in. You name it first.",
            "outline",
        ),
        Action::NewChapter => (
            "A new chapter at the end of this part, already named for you.",
            "outline",
        ),
        Action::NewPart => ("", "outline"),
        Action::NewFolder => (
            "A folder beside the one selected, for anything that isn't a chapter.",
            "outline",
        ),
        Action::Rename => (
            "A new name for the scene or folder you're on. Its file is renamed to match.",
            "outline",
        ),
        Action::Delete => (
            "Moves it to the Trash at the bottom of the outline. It asks first, by name.",
            "outline",
        ),
        Action::Goals => (
            "How many words the book is aiming for, and each day. Try a month of drafting: 50,000 words.",
            "sprints",
        ),
        Action::NextTk => (
            "Jumps to the next TK or note after the cursor, going round to the top at the end.",
            "notes",
        ),
        Action::OpenCodex => (
            "With the cursor on a name from your notebook, opens that character's or place's note beside the page.",
            "notebook",
        ),
        Action::Timer => (
            "The Pomodoro: twenty-five minutes of writing, then five of rest. Again pauses it.",
            "pomodoro",
        ),
        Action::TimerReset => ("Sets the Pomodoro back to the start.", "pomodoro"),
        Action::SaveSession => (
            "Writes this sitting into your writing sessions now, instead of when you quit.",
            "sessions",
        ),
        Action::Compile => (
            "The whole manuscript as one Markdown file, in the book's exports folder.",
            "compile",
        ),
        Action::ProjectMap => (
            "Rewrites project.md, a one-page map of the book: every part, chapter and scene, with word counts.",
            "compile",
        ),
        Action::Details => (
            "This scene's status, POV, synopsis and word target, and whether it goes in exports. i in the outline.",
            "outline",
        ),
        Action::Restore => (
            "Takes what's highlighted in the Trash back to where it was, in its old folder under its old name.",
            "outline",
        ),
        Action::Italic => (
            "Italics on the selected words, or the word at the cursor. Again takes them off. They're *stars* in the file.",
            "writing",
        ),
        Action::Bold => (
            "Bold on the selected words, or the word at the cursor. Again takes it off. It's **double stars** in the file.",
            "writing",
        ),
        Action::MoveUp => (
            "Moves the scene or folder you're on up one place. At the top of a chapter, a scene crosses into the one before.",
            "outline",
        ),
        Action::MoveDown => (
            "Moves the scene or folder you're on down one place. At the bottom of a chapter, a scene crosses into the next.",
            "outline",
        ),
        Action::History => (
            "Every saved version of this scene, to compare with now or bring back. Nothing is ever lost.",
            "history",
        ),
        Action::FindInBook => (
            "Search every scene and note for a word or phrase, and replace it everywhere if you like.",
            "find",
        ),
        Action::Sessions => (
            "Your writing sittings, like a diary: when you wrote, where, and how many words. Each one can be looked back at.",
            "sessions",
        ),
        Action::FindInScene => (
            "Find a word in this scene, and replace it if you like. Press it again to search the whole book.",
            "find",
        ),
        Action::CheckNames => (
            "Finds names spelled two ways, like Oren and Orin, so a character stays the same person all the way through.",
            "notebook",
        ),
        Action::MoveHistoryOut => (
            "Moves this book's writing history out of the synced folder, where sync can't damage it.",
            "sessions",
        ),
        Action::FocusMode => (
            "Hides everything but the page, so it's just you and the words. Esc still opens this menu.",
            "focus",
        ),
        Action::BesidePicker => (
            "Shows another scene next to this one, to read from while you write.",
            "beside",
        ),
        Action::NotesList => (
            "Every note and TK (your mark for \"fill this in later\") in the book, in one list.",
            "notes",
        ),
        Action::NextDraft => (
            "Opens the next scene that isn't marked revised or done yet. Handy for a revision pass.",
            "revision",
        ),
        Action::EchoWords => (
            "Lights up a word you've used again too soon, like \"the lamp… the lamp\" a line apart. Easy to miss, easy to fix.",
            "revision",
        ),
        Action::StartSprint => (
            "A timed burst of writing with a word goal. The status bar keeps count.",
            "sprints",
        ),
        Action::EndSprint => ("Ends the sprint now.", "sprints"),
        Action::ReadAloud => (
            "Reads the scene out loud from where you are, highlighting each sentence. Hearing it catches what the eye skips.",
            "revision",
        ),
        Action::Voices => (
            "Which voice reads aloud: a natural Piper voice if you have one, or your computer's own.",
            "revision",
        ),
        Action::Progress => (
            "Your words each week, your pace lately, and roughly when you'll reach your goal. No streaks.",
            "sprints",
        ),
        Action::MusicPlayer => (
            "The queue, your playlists, search, and every control.",
            "music",
        ),
        Action::Themes => (
            "Pick your colours. Each one shows as you move; Esc puts the old one back.",
            "themes",
        ),
        Action::MusicSource => (
            "Where music comes from: YouTube Music, Spotify, Jellyfin or Plex.",
            "music",
        ),
        Action::MusicToggle => (
            "Music is off until you want it. On, it gets a pane of its own.",
            "music",
        ),
        Action::Spellcheck => (
            "Underlines words that might be misspelt as you write.",
            "spelling",
        ),
        Action::SpellingLanguage => (
            "Which English spellcheck knows: American, British, Canadian or Australian. Each press tries the next.",
            "spelling",
        ),
        Action::Icons => (
            "A little symbol beside each row of the outline.",
            "settings",
        ),
        Action::LineWidth => (
            "How long a line of prose can run. Each press tries the next width.",
            "settings",
        ),
        Action::Typewriter => (
            "In focus mode, keeps the line you're writing near the middle of the screen.",
            "focus",
        ),
        Action::ManuscriptLook => (
            "How the exported manuscript looks: the font, paper, spacing and chapter headings.",
            "compile",
        ),
        Action::AuthorDetails => (
            "Your name and contact details, for the manuscript's title page.",
            "compile",
        ),
        Action::Help => (
            "Everything Grimoire does, in plain words. Type to search it.",
            "getting-started",
        ),
        Action::About => (
            "Which version this is, and who made it: Catfinity Studios, makers of Catfinity.",
            "getting-started",
        ),
        Action::License => (
            "Free to use, even for work you sell. Credit is all it asks.",
            "license",
        ),
        Action::Donate => (
            "Grimoire is free. A tip on Ko-fi helps it keep growing, and is never required.",
            "getting-started",
        ),
        Action::MenuBack => ("Back to the first menu.", "getting-started"),
        _ => ("", "getting-started"),
    };
    let says = if *action == Action::NewPart {
        format!("A new {part} at the end of the manuscript: the biggest division of the book.")
    } else {
        says.to_string()
    };
    (says, topic)
}

/// Break `text` into lines no wider than `width`, at spaces.
fn wrap(text: &str, width: usize) -> Vec<String> {
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

/// Room for this many lines of the highlighted row's explanation.
const ABOUT_LINES: usize = 3;
const MENU_W: u16 = 50;

pub(super) fn draw_menu(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let (Overlay::Menu { sel } | Overlay::Sub { sel, .. }) = &app.overlay else {
        return;
    };
    let sub = match &app.overlay {
        Overlay::Sub { sub, .. } => Some(*sub),
        _ => None,
    };
    let rows = app.menu_rows();
    // Rows, a gap, the explanation, the key hints.
    let box_area = centred(area, MENU_W, rows.len() as u16 + 4 + 1 + ABOUT_LINES as u16);
    f.render_widget(Clear, box_area);
    let title = match sub {
        None => "GRIMOIRE",
        Some(Sub::Book) => "GRIMOIRE › THIS BOOK",
        Some(Sub::Writing) => "GRIMOIRE › WRITING TOOLS",
        Some(Sub::Settings) => "GRIMOIRE › SETTINGS",
        Some(Sub::Help) => "GRIMOIRE › HELP & ABOUT",
    };
    let block = pane_block(title, true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);

    // A short terminal gives up the explanation before any rows, and shows
    // the rows around the highlight rather than cutting off the end.
    let spare = (inner.height as usize).saturating_sub(rows.len() + 2);
    let about_room = spare.saturating_sub(1).min(ABOUT_LINES);
    let room = (inner.height as usize)
        .saturating_sub(2 + if about_room > 0 { about_room + 1 } else { 0 })
        .max(1);
    let start = (*sel + 1).saturating_sub(room);
    let mut lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(room)
        .map(|(i, (label, _))| {
            let on = i == *sel;
            Line::from(vec![
                Span::styled(
                    if on { " ▸ " } else { "   " },
                    Style::default().fg(if on { t.accent } else { t.dim }),
                ),
                Span::styled(
                    label.clone(),
                    Style::default().fg(if on { t.accent } else { t.text }),
                ),
            ])
            .style(if on {
                Style::default().bg(t.sel)
            } else {
                Style::default()
            })
        })
        .collect();
    if about_room > 0
        && let Some((_, action)) = rows.get(*sel)
    {
        let (says, _) = about_row(action, &app.project.meta.part_noun().to_lowercase());
        lines.push(Line::from(""));
        let mut said = wrap(&says, inner.width.saturating_sub(2) as usize);
        said.truncate(about_room);
        for _ in said.len()..about_room {
            said.push(String::new());
        }
        for l in said {
            lines.push(Line::from(Span::styled(
                format!(" {l}"),
                Style::default().fg(t.dim).add_modifier(Modifier::ITALIC),
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(hint_line(
        if sub.is_some() {
            " j/k move   ↵ choose   ? more   esc back"
        } else {
            " j/k move   ↵ choose   ? more   esc close"
        },
        t,
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
