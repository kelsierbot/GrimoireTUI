//! The boxes that come up over the desk. Each module holds one overlay (or a
//! tight group of them): the keys it answers to and how it is drawn, side by
//! side. A child of `app`, so the key handlers work on the App's own state.

use super::*;
use crate::music::State as MusicState;
use crate::ui::{
    blend, centred, hint_line, hint_spans, key_style, pane_block, thousands, truncate, wrap_words,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

mod cards;
mod command;
mod conflicts;
mod find;
mod looks;
mod marks;
mod menu;
mod naming;
mod player;
mod readers;
mod recover;
mod session_list;
mod spelling;
mod versions;

impl App {
    /// A key while an overlay is up goes to that overlay alone.
    pub fn on_overlay_key(&mut self, key: Key) {
        match self.overlay {
            Overlay::None => {}
            Overlay::Marks { .. } => self.on_marks_key(key),
            Overlay::Sprint { .. } => self.on_sprint_key(key),
            Overlay::Palette { .. } => self.on_palette_key(key),
            Overlay::Find { .. } => self.on_find_key(key),
            Overlay::FindBook { .. } => self.on_find_book_key(key),
            Overlay::Cork { .. } => self.on_cork_key(key),
            Overlay::SessionsOff => self.on_sessions_off_key(key),
            Overlay::Sessions { .. } => self.on_sessions_key(key),
            Overlay::SessionDiff { .. } => self.on_session_diff_key(key),
            Overlay::Export { .. } => self.on_export_key(key),
            Overlay::Look { .. } => self.on_look_key(key),
            Overlay::Author { .. } => self.on_author_key(key),
            Overlay::Spelling { .. } => self.on_spelling_key(key),
            Overlay::Names { .. } => self.on_names_key(key),
            Overlay::Recover { .. } => self.on_recover_key(key),
            Overlay::History { .. } => self.on_history_key(key),
            Overlay::Menu { .. } | Overlay::Settings { .. } => self.on_menu_key(key),
            Overlay::Sources { .. } => self.on_sources_key(key),
            Overlay::Themes { .. } => self.on_themes_key(key),
            Overlay::Custom { .. } => self.on_custom_key(key),
            Overlay::Player { .. } => self.on_player_key(key),
            Overlay::Create { .. } => self.on_create_key(key),
            Overlay::Rename { .. } => self.on_rename_key(key),
            Overlay::Confirm { .. } => self.on_confirm_key(key),
            Overlay::Conflicts { .. } => self.on_conflicts_key(key),
            Overlay::Settle { .. } => self.on_settle_key(key),
        }
    }
}

/// How a list answers the keys that move through it.
#[derive(Debug, Clone, Copy)]
struct Nav {
    /// j and k move too — not in a list you type a filter into.
    letters: bool,
    /// Past either end, go round to the other.
    wrap: bool,
    /// PgUp/PgDn move by ten and Home/End go to the ends — unless the
    /// overlay pages through something else with them.
    pages: bool,
}

/// An ordinary list.
const LIST: Nav = Nav {
    letters: true,
    wrap: false,
    pages: true,
};
/// A short menu that goes round.
const RING: Nav = Nav {
    letters: true,
    wrap: true,
    pages: false,
};
/// A list under a box you type into: letters are for typing.
const TYPED: Nav = Nav {
    letters: false,
    wrap: false,
    pages: true,
};
/// A list beside something PgUp/PgDn scroll.
const LINES: Nav = Nav {
    letters: true,
    wrap: false,
    pages: false,
};

/// Move `sel` through a list of `len` rows for ↑↓ — and j/k, PgUp/PgDn and
/// Home/End as `nav` allows. True if `key` was one of them, so the overlay
/// can stop there.
fn list_nav(key: Key, sel: &mut usize, len: usize, nav: Nav) -> bool {
    let step: isize = match key {
        Key::Down => 1,
        Key::Up => -1,
        Key::Char('j') if nav.letters => 1,
        Key::Char('k') if nav.letters => -1,
        Key::PageDown if nav.pages => 10,
        Key::PageUp if nav.pages => -10,
        Key::Home if nav.pages => {
            *sel = 0;
            return true;
        }
        Key::End if nav.pages => {
            *sel = len.saturating_sub(1);
            return true;
        }
        _ => return false,
    };
    if len == 0 {
        *sel = 0;
    } else if nav.wrap {
        *sel = (*sel as isize + step).rem_euclid(len as isize) as usize;
    } else {
        *sel = (*sel as isize + step).clamp(0, len as isize - 1) as usize;
    }
    true
}

/// Draw whichever overlay is up, over the desk.
pub fn draw(f: &mut Frame, app: &App, area: Rect, t: &Theme) {
    match &app.overlay {
        Overlay::None => {}
        Overlay::Marks { query, sel, rows } => marks::draw_marks(f, area, query, *sel, rows, t),
        Overlay::Sprint {
            words,
            minutes,
            on_minutes,
            ..
        } => marks::draw_sprint(f, area, words, minutes, *on_minutes, t),
        Overlay::Palette { .. } => command::draw_palette(f, app, area, t),
        Overlay::Find { .. } => find::draw_find(f, app, area, t),
        Overlay::FindBook { .. } => find::draw_find_book(f, app, area, t),
        Overlay::SessionsOff => session_list::draw_sessions_off(f, app, area, t),
        Overlay::Sessions { .. } => session_list::draw_sessions(f, app, area, t),
        Overlay::SessionDiff { .. } => session_list::draw_session_diff(f, app, area, t),
        Overlay::Export { .. } => readers::draw_export(f, app, area, t),
        Overlay::Look { .. } => readers::draw_look(f, app, area, t),
        Overlay::Author { .. } => readers::draw_author(f, app, area, t),
        Overlay::Spelling { .. } => spelling::draw_spelling(f, app, area, t),
        Overlay::Cork {
            scope,
            sel,
            pov,
            typing,
        } => cards::draw_cork(
            f,
            app,
            area,
            t,
            scope,
            *sel,
            pov.as_deref(),
            typing.as_ref(),
        ),
        Overlay::Names { .. } => spelling::draw_names(f, app, area, t),
        Overlay::Recover { .. } => recover::draw_recover(f, app, area, t),
        Overlay::History { .. } => versions::draw_history(f, app, area, t),
        Overlay::Menu { .. } | Overlay::Settings { .. } => menu::draw_menu(f, app, area, t),
        Overlay::Player { .. } => player::draw_player(f, app, area, t),
        Overlay::Create { .. } => naming::draw_create(f, app, area, t),
        Overlay::Rename { .. } => naming::draw_rename(f, app, area, t),
        Overlay::Confirm { .. } => naming::draw_confirm(f, app, area, t),
        Overlay::Conflicts { .. } => conflicts::draw_conflicts(f, app, area, t),
        Overlay::Settle { .. } => conflicts::draw_settle(f, app, area, t),
        Overlay::Sources { .. } => looks::draw_sources(f, app, area, t),
        Overlay::Themes { .. } => looks::draw_themes(f, app, area, t),
        Overlay::Custom { .. } => looks::draw_custom(f, app, area, t),
    }
}

/// "Today 9:14 pm", "Yesterday 6:02 pm", "Tue 16 Sep 9:14 pm".
fn when_label(dt: chrono::DateTime<chrono::Local>) -> String {
    let today = chrono::Local::now().date_naive();
    let clock = dt.format("%-I:%M %P").to_string();
    let day = dt.date_naive();
    if day == today {
        format!("Today {clock}")
    } else if Some(day) == today.pred_opt() {
        format!("Yesterday {clock}")
    } else {
        format!("{} {clock}", dt.format("%a %-d %b"))
    }
}

/// Lay a word diff out as wrapped lines. Returns the lines and the index of
/// the first line with a change, so the view can open where it matters.
fn diff_lines(
    pieces: &[grimoire_core::history::Piece],
    width: usize,
    t: &Theme,
) -> (Vec<Line<'static>>, usize) {
    use grimoire_core::history::Piece;

    struct Wrap {
        width: usize,
        lines: Vec<Line<'static>>,
        cur: Vec<Span<'static>>,
        used: usize,
        changed: bool,
        first_change: Option<usize>,
    }
    impl Wrap {
        fn end_line(&mut self) {
            if self.changed && self.first_change.is_none() {
                self.first_change = Some(self.lines.len());
            }
            self.lines.push(Line::from(std::mem::take(&mut self.cur)));
            self.used = 0;
            self.changed = false;
        }
        fn push(&mut self, token: &str, style: Style, changed: bool) {
            let w = token.chars().count();
            let space = token.chars().all(char::is_whitespace);
            if self.used + w > self.width && self.used > 0 {
                self.end_line();
                if space {
                    return;
                }
            }
            self.cur.push(Span::styled(token.to_string(), style));
            self.used += w;
            self.changed |= changed;
        }
    }

    let mut wrap = Wrap {
        width: width.max(10),
        lines: Vec::new(),
        cur: Vec::new(),
        used: 0,
        changed: false,
        first_change: None,
    };
    let plain = Style::default().fg(t.text);
    let gone = Style::default()
        .fg(t.warn)
        .add_modifier(Modifier::CROSSED_OUT);
    let new = Style::default()
        .fg(t.accent)
        .add_modifier(Modifier::UNDERLINED);

    for p in pieces {
        let (text, style, changed) = match p {
            Piece::Same(s) => (s.as_str(), plain, false),
            Piece::Removed(s) => (s.as_str(), gone, true),
            Piece::Added(s) => (s.as_str(), new, true),
        };
        let mut token = String::new();
        for ch in text.chars() {
            if ch == '\n' {
                if !token.is_empty() {
                    wrap.push(&token, style, changed);
                    token.clear();
                }
                wrap.changed |= changed;
                wrap.end_line();
                continue;
            }
            let boundary = token
                .chars()
                .last()
                .is_some_and(|c| c.is_whitespace() != ch.is_whitespace());
            if boundary {
                wrap.push(&token, style, changed);
                token.clear();
            }
            token.push(ch);
        }
        if !token.is_empty() {
            wrap.push(&token, style, changed);
        }
    }
    if !wrap.cur.is_empty() {
        wrap.end_line();
    }
    let first = wrap.first_change.unwrap_or(0);
    (wrap.lines, first)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved(key: Key, from: usize, len: usize, nav: Nav) -> Option<usize> {
        let mut sel = from;
        list_nav(key, &mut sel, len, nav).then_some(sel)
    }

    #[test]
    fn a_list_stops_at_its_ends_and_a_ring_goes_round() {
        assert_eq!(moved(Key::Down, 4, 5, LIST), Some(4));
        assert_eq!(moved(Key::Up, 0, 5, LIST), Some(0));
        assert_eq!(moved(Key::Down, 4, 5, RING), Some(0));
        assert_eq!(moved(Key::Up, 0, 5, RING), Some(4));
    }

    #[test]
    fn letters_move_only_where_nothing_is_typed() {
        assert_eq!(moved(Key::Char('j'), 1, 5, LIST), Some(2));
        assert_eq!(moved(Key::Char('k'), 1, 5, LIST), Some(0));
        assert_eq!(moved(Key::Char('j'), 1, 5, TYPED), None);
        assert_eq!(moved(Key::Down, 1, 5, TYPED), Some(2));
    }

    #[test]
    fn pages_and_ends_unless_the_overlay_pages_something_else() {
        assert_eq!(moved(Key::PageDown, 3, 40, LIST), Some(13));
        assert_eq!(moved(Key::PageDown, 35, 40, LIST), Some(39));
        assert_eq!(moved(Key::PageUp, 3, 40, LIST), Some(0));
        assert_eq!(moved(Key::End, 3, 40, LIST), Some(39));
        assert_eq!(moved(Key::Home, 30, 40, LIST), Some(0));
        assert_eq!(moved(Key::PageDown, 3, 40, LINES), None);
        assert_eq!(moved(Key::End, 3, 40, LINES), None);
    }

    #[test]
    fn an_empty_list_or_a_selection_past_the_end_lands_in_range() {
        assert_eq!(moved(Key::Down, 0, 0, LIST), Some(0));
        assert_eq!(moved(Key::Down, 0, 0, RING), Some(0));
        assert_eq!(moved(Key::Up, 9, 3, TYPED), Some(2));
    }
}
