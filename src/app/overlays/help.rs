//! The help browser: the topics on the left, the one you're reading on the
//! right. Typing searches every topic; a narrow window shows the list, then
//! the article on its own.

use super::*;
use crate::help::{self, Mark, Run};

/// Below this many columns inside the box, the list and the article take
/// turns instead of sitting side by side.
const SIDE_BY_SIDE: u16 = 70;
/// The topic list's width, border included.
const LIST_W: u16 = 30;

impl App {
    /// Open the help on `topic` (an id from [`help::TOPICS`]), or on the
    /// topic for wherever you are. Whatever box was up comes back when the
    /// help closes.
    pub fn open_help(&mut self, topic: Option<&str>) {
        let id = topic.unwrap_or_else(|| self.help_context());
        let back = match std::mem::replace(&mut self.overlay, Overlay::None) {
            Overlay::None | Overlay::Menu { .. } | Overlay::Palette { .. } => None,
            Overlay::Help { back, .. } => back,
            other => Some(Box::new(other)),
        };
        self.help_fit.set(HelpFit::default());
        self.overlay = Overlay::Help {
            query: String::new(),
            sel: help::index(id),
            reading: topic.is_some(),
            scroll: 0,
            back,
        };
    }

    /// The topic that explains where you are.
    pub fn help_context(&self) -> &'static str {
        match &self.overlay {
            Overlay::Settings { .. } => "settings",
            Overlay::Themes { .. } | Overlay::Custom { .. } => "themes",
            Overlay::Sources { .. } | Overlay::Player { .. } => "music",
            Overlay::Export { .. } | Overlay::Look { .. } | Overlay::Author { .. } => "compile",
            Overlay::Conflicts { .. } | Overlay::Settle { .. } => "sync",
            Overlay::Cork { .. } => "corkboard",
            Overlay::History { .. } | Overlay::Recover { .. } => "history",
            Overlay::Marks { .. } => "notes",
            Overlay::Sprint { .. } => "sprints",
            Overlay::Spelling { .. } => "spelling",
            Overlay::Names { .. } => "notebook",
            Overlay::SessionsOff | Overlay::Sessions { .. } | Overlay::SessionDiff { .. } => {
                "sessions"
            }
            Overlay::Find { .. } | Overlay::FindBook { .. } => "find",
            Overlay::Create { .. } | Overlay::Rename { .. } | Overlay::Confirm { .. } => "outline",
            Overlay::Help { .. } | Overlay::Menu { .. } | Overlay::Palette { .. } => {
                "getting-started"
            }
            Overlay::None => match self.focus {
                Focus::Tree => "outline",
                Focus::Editor if self.focus_mode => "focus",
                Focus::Editor => "writing",
                Focus::Codex => "notebook",
                Focus::Beside => "beside",
                Focus::Clearing if self.pane_mode == crate::scene::Mode::Visualizer => "visualizer",
                Focus::Clearing => "pomodoro",
                Focus::Music => "music",
            },
        }
    }

    /// Whether `?` means help in the box that's up: not where it would be
    /// typed into something.
    pub fn question_opens_help(&self) -> bool {
        match &self.overlay {
            Overlay::None => self.focus != Focus::Editor,
            Overlay::Palette { .. }
            | Overlay::Marks { .. }
            | Overlay::Find { .. }
            | Overlay::FindBook { .. }
            | Overlay::Create { .. }
            | Overlay::Rename { .. }
            | Overlay::Sprint { .. }
            | Overlay::Author { .. }
            | Overlay::Help { .. } => false,
            Overlay::Spelling { inline, .. } => !*inline,
            Overlay::Look { sel, .. } => *sel != super::readers::KEYWORD_ROW,
            Overlay::Cork { typing, .. } => typing.is_none(),
            Overlay::Player { typing, .. } => !*typing,
            _ => true,
        }
    }

    /// The wheel over the help reads on through the article. True if the
    /// help is up.
    pub fn help_wheel(&mut self, down: bool, step: usize) -> bool {
        let max = self.help_fit.get().max;
        let Overlay::Help { scroll, .. } = &mut self.overlay else {
            return false;
        };
        *scroll = if down {
            (*scroll + step).min(max)
        } else {
            scroll.saturating_sub(step)
        };
        true
    }

    fn close_help(&mut self) {
        let back = match std::mem::replace(&mut self.overlay, Overlay::None) {
            Overlay::Help { back, .. } => back,
            _ => None,
        };
        if let Some(b) = back {
            self.overlay = *b;
        }
    }

    pub(super) fn on_help_key(&mut self, key: Key) {
        let fit = self.help_fit.get();
        let Overlay::Help {
            query,
            sel,
            reading,
            scroll,
            ..
        } = &mut self.overlay
        else {
            return;
        };
        // The article scrolls from the list too, where it's beside it.
        if (*reading || fit.wide) && scrolled(key, scroll, fit, *reading) {
            return;
        }
        if *reading {
            match key {
                Key::Left | Key::Tab | Key::BackTab | Key::Char('h') => *reading = false,
                // Beside the topics, Esc is the list's Esc; on its own, the
                // article steps back to them first.
                Key::Esc if !fit.wide => *reading = false,
                Key::Esc if !query.is_empty() => query.clear(),
                Key::Esc | Key::F(1) => self.close_help(),
                // Anything else typed starts a search.
                Key::Char(c) if !c.is_control() => {
                    *reading = false;
                    query.push(c);
                    refind(query, sel, scroll);
                }
                Key::Backspace if !query.is_empty() => {
                    *reading = false;
                    query.pop();
                    refind(query, sel, scroll);
                }
                _ => {}
            }
            return;
        }
        // The list: `sel` is a topic index; ↑↓ move it through what's found.
        let found = help::search(query);
        let mut at = found.iter().position(|&i| i == *sel).unwrap_or(0);
        let nav = if fit.wide { ARROWS } else { TYPED };
        if list_nav(key, &mut at, found.len(), nav) {
            if let Some(&i) = found.get(at)
                && i != *sel
            {
                *sel = i;
                *scroll = 0;
            }
            return;
        }
        match key {
            Key::Enter | Key::Right | Key::Tab if !found.is_empty() => {
                if !found.contains(sel) {
                    *sel = found[0];
                    *scroll = 0;
                }
                *reading = true;
            }
            Key::Char(c) if !c.is_control() => {
                query.push(c);
                refind(query, sel, scroll);
            }
            Key::Backspace => {
                query.pop();
                refind(query, sel, scroll);
            }
            Key::Esc if !query.is_empty() => {
                query.clear();
                refind(query, sel, scroll);
            }
            Key::Esc | Key::F(1) => self.close_help(),
            _ => {}
        }
    }
}

/// Only the arrows move through the topics: letters are for the search,
/// and the page keys scroll the article beside them.
const ARROWS: Nav = Nav {
    letters: false,
    wrap: false,
    pages: false,
};

/// The search changed: keep the topic if it's still found, or go to the
/// best match.
fn refind(query: &str, sel: &mut usize, scroll: &mut usize) {
    let found = help::search(query);
    if !found.contains(sel)
        && let Some(&first) = found.first()
    {
        *sel = first;
        *scroll = 0;
    }
}

/// Scroll the article for `key`; true if it was a scrolling key. j, k and
/// Space only scroll inside the article — in the list they're typed.
fn scrolled(key: Key, scroll: &mut usize, fit: HelpFit, reading: bool) -> bool {
    let page = fit.page.max(1);
    let to = match key {
        Key::Down | Key::Char('j') if reading => *scroll + 1,
        Key::Up | Key::Char('k') if reading => scroll.saturating_sub(1),
        Key::Char(' ') if reading => *scroll + page,
        Key::PageDown => *scroll + page,
        Key::PageUp => scroll.saturating_sub(page),
        Key::Home => 0,
        Key::End => fit.max,
        _ => return false,
    };
    *scroll = to.min(fit.max);
    true
}

pub(super) fn draw_help(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    let Overlay::Help {
        query,
        sel,
        reading,
        scroll,
        ..
    } = &app.overlay
    else {
        return;
    };
    let w = area.width.saturating_sub(4).min(118);
    let h = area.height.saturating_sub(2);
    let box_area = centred(area, w, h);
    f.render_widget(Clear, box_area);
    let block = pane_block("HELP", true, t);
    let inner = block.inner(box_area);
    f.render_widget(block, box_area);
    if inner.height < 3 || inner.width < 10 {
        return;
    }
    let [body, foot] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
    let found = help::search(query);
    let wide = inner.width >= SIDE_BY_SIDE;
    app.help_fit.set(HelpFit {
        wide,
        ..app.help_fit.get()
    });

    let (list_area, article_area) = if wide {
        let [l, a] =
            Layout::horizontal([Constraint::Length(LIST_W), Constraint::Min(10)]).areas(body);
        (Some(l), Some(a))
    } else if *reading {
        (None, Some(body))
    } else {
        (Some(body), None)
    };

    if let Some(l) = list_area {
        draw_topics(f, l, query, *sel, &found, !*reading, wide, t);
    }
    if let Some(a) = article_area {
        let topic = if found.contains(sel) || query.is_empty() {
            Some(*sel)
        } else {
            found.first().copied()
        };
        match topic {
            Some(i) => draw_article(f, app, a, i, *scroll, wide, t),
            None => f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "  nothing in the help mentions that",
                    Style::default().fg(t.dim),
                ))),
                a,
            ),
        }
    }

    // The footer: the keys for where you are, and whether there's more of
    // the article below.
    let hint = match (*reading, query.is_empty(), wide) {
        (true, _, true) => " ↑↓ PgDn scroll  ← topics  type to search  esc close",
        (true, _, false) => " ↑↓ PgDn scroll  ← topics  esc back",
        (false, true, true) => " ↑↓ topic  ↵ read  PgDn scroll  type to search  esc close",
        (false, true, false) => " ↑↓ topic  ↵ read  type to search  esc close",
        (false, false, _) => " ↑↓ topic  ↵ read  esc clear the search",
    };
    let mut spans = hint_spans(hint, t);
    let fit = app.help_fit.get();
    if article_area.is_some() && *scroll < fit.max {
        let used: usize = spans.iter().map(|s| s.width()).sum();
        let more = "↓ more ";
        let gap = (foot.width as usize).saturating_sub(used + more.chars().count());
        if gap > 0 {
            spans.push(Span::raw(" ".repeat(gap)));
            spans.push(Span::styled(more, Style::default().fg(t.accent)));
        }
    }
    f.render_widget(Paragraph::new(Line::from(spans)), foot);
}

#[allow(clippy::too_many_arguments)]
fn draw_topics(
    f: &mut Frame,
    area: Rect,
    query: &str,
    sel: usize,
    found: &[usize],
    focused: bool,
    wide: bool,
    t: &Theme,
) {
    let mut block = Block::default();
    if wide {
        block = block
            .borders(Borders::RIGHT)
            .border_style(Style::default().fg(t.border));
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    let width = inner.width as usize;
    let mut lines = vec![
        Line::from(vec![
            Span::styled(" search ", Style::default().fg(t.dim)),
            Span::styled(
                truncate(query, width.saturating_sub(10)),
                Style::default().fg(t.text),
            ),
            Span::styled(
                if focused { "▏" } else { "" },
                Style::default().fg(t.accent),
            ),
        ]),
        Line::from(""),
    ];
    let room = (inner.height as usize).saturating_sub(lines.len());
    let at = found.iter().position(|&i| i == sel).unwrap_or(0);
    let start = (at + 1).saturating_sub(room);
    for &i in found.iter().skip(start).take(room) {
        let on = i == sel;
        let style = if on {
            Style::default()
                .fg(t.accent)
                .bg(t.sel)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text)
        };
        let marker = if on { " ▸ " } else { "   " };
        // The chosen row is lit the whole width of the list.
        let name = truncate(help::TOPICS[i].name, width.saturating_sub(3));
        let pad = width.saturating_sub(3 + name.chars().count());
        lines.push(Line::from(vec![
            Span::styled(marker, style),
            Span::styled(format!("{name}{}", " ".repeat(pad)), style),
        ]));
    }
    if found.is_empty() {
        lines.push(Line::from(Span::styled(
            "   no topic matches",
            Style::default().fg(t.dim),
        )));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

#[allow(clippy::too_many_arguments)]
fn draw_article(
    f: &mut Frame,
    app: &App,
    area: Rect,
    topic: usize,
    scroll: usize,
    wide: bool,
    t: &Theme,
) {
    // A margin either side, so the words don't touch the rules.
    let pad = if wide { 2 } else { 1 };
    let inner = Rect {
        x: area.x + pad,
        y: area.y,
        width: area.width.saturating_sub(pad * 2),
        height: area.height,
    };
    let mut lines = Vec::new();
    if !wide {
        lines.push(Line::from(Span::styled(
            "← topics",
            Style::default().fg(t.dim),
        )));
    }
    lines.extend(lay_out(help::TOPICS[topic].body, inner.width as usize, t));
    let height = inner.height as usize;
    let max = lines.len().saturating_sub(height);
    app.help_fit.set(HelpFit {
        max,
        page: height.saturating_sub(2),
        wide,
    });
    let scroll = scroll.min(max);
    let shown: Vec<Line> = lines.into_iter().skip(scroll).take(height).collect();
    f.render_widget(Paragraph::new(shown), inner);
}

/// An article as lines `width` columns wide.
pub(crate) fn lay_out(body: &str, width: usize, t: &Theme) -> Vec<Line<'static>> {
    let width = width.max(12);
    // A table's head ("Key | Does") says nothing the rows don't.
    let blocks: Vec<help::Block> = help::parse(body)
        .into_iter()
        .filter(|b| !matches!(b, help::Block::Row { head: true, .. }))
        .collect();
    // A table's first column is as wide as its widest key, up to a third.
    let key_w = blocks
        .iter()
        .filter_map(|b| match b {
            help::Block::Row { key, .. } => Some(runs_width(key)),
            _ => None,
        })
        .max()
        .unwrap_or(0)
        .min(width / 3)
        + 2;
    let text = Style::default().fg(t.text);
    let mut out: Vec<Line<'static>> = Vec::new();
    for (n, b) in blocks.iter().enumerate() {
        // A blank line between blocks, but not between a table's rows or a
        // list's bullets.
        let same = n > 0
            && matches!(
                (&blocks[n - 1], b),
                (help::Block::Row { .. }, help::Block::Row { .. })
                    | (help::Block::Bullet(_), help::Block::Bullet(_))
            );
        if n > 0 && !same {
            out.push(Line::from(""));
        }
        match b {
            help::Block::Heading(level, runs) => {
                let mut style = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);
                if *level == 1 {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                out.extend(wrap_runs(runs, width, "", style, t));
            }
            help::Block::Paragraph(runs) => out.extend(wrap_runs(runs, width, "", text, t)),
            help::Block::Bullet(runs) => out.extend(wrap_runs(runs, width, "• ", text, t)),
            help::Block::Row {
                key, text: does, ..
            } => {
                let keys = wrap_runs(key, key_w - 2, "", text, t);
                let words = wrap_runs(does, width.saturating_sub(key_w), "", text, t);
                for i in 0..keys.len().max(words.len()) {
                    let mut spans: Vec<Span<'static>> = Vec::new();
                    let used = keys.get(i).map_or(0, |k| {
                        spans.extend(k.spans.iter().cloned());
                        k.width()
                    });
                    spans.push(Span::raw(" ".repeat(key_w.saturating_sub(used))));
                    if let Some(w) = words.get(i) {
                        spans.extend(w.spans.iter().cloned());
                    }
                    out.push(Line::from(spans));
                }
            }
        }
    }
    out
}

fn runs_width(runs: &[Run]) -> usize {
    runs.iter().map(|r| r.text.chars().count()).sum()
}

/// Word-wrap styled runs into lines at most `width` columns wide. The first
/// line starts with `lead` ("• ") and the rest hang under the words after
/// it. Lines only break at spaces, so a key keeps the comma after it.
fn wrap_runs(runs: &[Run], width: usize, lead: &str, base: Style, t: &Theme) -> Vec<Line<'static>> {
    let width = width.max(4);
    let style_of = |m: Mark| match m {
        Mark::Plain => base,
        Mark::Bold => base.fg(t.text).add_modifier(Modifier::BOLD),
        Mark::Italic => base.add_modifier(Modifier::ITALIC),
        Mark::Code => key_style(t),
    };
    // Words: pieces with no space between them, each piece in one style. A
    // space starts a new word and belongs to it (dropped at a line's start).
    let mut words: Vec<Vec<(String, Style)>> = Vec::new();
    for r in runs {
        let style = style_of(r.mark);
        let mut piece = String::new();
        for ch in r.text.chars() {
            if ch == ' ' && r.mark != Mark::Code {
                if !piece.is_empty() {
                    glue(&mut words, std::mem::take(&mut piece), style);
                }
                words.push(Vec::new());
            }
            piece.push(ch);
        }
        if !piece.is_empty() {
            glue(&mut words, piece, style);
        }
    }
    let hang = lead.chars().count();
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans: Vec<Span<'static>> = vec![Span::styled(lead.to_string(), base)];
    let mut used = hang;
    let mut fresh = true;
    for word in words {
        let mut word = word;
        if fresh && let Some((first, _)) = word.first_mut() {
            *first = first.trim_start().to_string();
        }
        let len: usize = word.iter().map(|(p, _)| p.chars().count()).sum();
        if !fresh && used + len > width {
            lines.push(Line::from(std::mem::take(&mut spans)));
            spans.push(Span::raw(" ".repeat(hang)));
            used = hang;
            if let Some((first, _)) = word.first_mut() {
                *first = first.trim_start().to_string();
            }
        }
        for (piece, style) in word {
            // Only a word wider than the whole line is ever cut.
            let mut piece = piece;
            while used + piece.chars().count() > width {
                let room = width.saturating_sub(used);
                let head: String = piece.chars().take(room).collect();
                piece = piece.chars().skip(room).collect();
                if !head.is_empty() {
                    spans.push(Span::styled(head, style));
                }
                lines.push(Line::from(std::mem::take(&mut spans)));
                spans.push(Span::raw(" ".repeat(hang)));
                used = hang;
            }
            used += piece.chars().count();
            if !piece.is_empty() {
                spans.push(Span::styled(piece, style));
            }
        }
        fresh = used == hang;
    }
    lines.push(Line::from(spans));
    lines
}

/// Add a piece to the last word, starting one if there's none yet.
fn glue(words: &mut Vec<Vec<(String, Style)>>, piece: String, style: Style) {
    match words.last_mut() {
        Some(w) => w.push((piece, style)),
        None => words.push(vec![(piece, style)]),
    }
}
