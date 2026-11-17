//! One-line boxes: the keys they all take the same way, and the caret they
//! draw. A name, a synopsis or a search can be mended in the middle instead
//! of rubbed out from the end.

use super::*;

/// What a box does with a key. `caret` is the place in `buf`, counted in
/// characters; `None` is the end, where typing starts. True when the key was
/// one of the box's own (typing, `←` `→`, `Home`, `End`, `Backspace`,
/// `Delete`), whether or not it changed anything.
pub(crate) fn edit(
    buf: &mut String,
    caret: &mut Option<usize>,
    key: Key,
    cap: usize,
    takes: impl Fn(char) -> bool,
) -> bool {
    let len = buf.chars().count();
    let at = caret.unwrap_or(len).min(len);
    let byte = |s: &str, i: usize| s.char_indices().nth(i).map_or(s.len(), |(b, _)| b);
    let to = match key {
        Key::Char(c) if !c.is_control() && takes(c) => {
            if len < cap {
                buf.insert(byte(buf, at), c);
                at + 1
            } else {
                at
            }
        }
        Key::Backspace if at > 0 => {
            buf.remove(byte(buf, at - 1));
            at - 1
        }
        Key::Delete if at < len => {
            buf.remove(byte(buf, at));
            at
        }
        Key::Backspace | Key::Delete => at,
        Key::Left => at.saturating_sub(1),
        Key::Right => (at + 1).min(len),
        Key::Home => 0,
        Key::End => len,
        _ => return false,
    };
    *caret = (to < buf.chars().count()).then_some(to);
    true
}

/// The box's words with the caret drawn in them, in at most `room` columns:
/// the view slides along so the caret always shows. At the end the caret is
/// the block it always was; inside the words it sits on its letter.
pub(crate) fn spans(
    buf: &str,
    caret: Option<usize>,
    room: usize,
    words: Style,
    t: &Theme,
) -> Vec<Span<'static>> {
    let chars: Vec<char> = buf.chars().collect();
    let len = chars.len();
    let at = caret.unwrap_or(len).min(len);
    let room = room.max(2);
    let start = (at + 1).saturating_sub(room);
    let end = len.min(start + room - usize::from(at == len));
    let text = |a: usize, b: usize| chars[a.min(b)..b].iter().collect::<String>();
    let mut out = vec![Span::styled(text(start, at), words)];
    if at < len {
        out.push(Span::styled(
            chars[at].to_string(),
            Style::default().fg(t.sel).bg(t.accent),
        ));
        out.push(Span::styled(text(at + 1, end.max(at + 1)), words));
    } else {
        out.push(Span::styled("█", Style::default().fg(t.accent)));
    }
    out
}

/// For a box whose words wrap over several rows: the same words with the
/// caret marked in them, a thin bar before its letter or the block at the end.
pub(crate) fn marked(buf: &str, caret: Option<usize>) -> String {
    let len = buf.chars().count();
    match caret.filter(|&at| at < len) {
        Some(at) => {
            let cut = buf.char_indices().nth(at).map_or(buf.len(), |(b, _)| b);
            format!("{}▏{}", &buf[..cut], &buf[cut..])
        }
        None => format!("{buf}█"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(start: &str, keys: &[Key]) -> (String, Option<usize>) {
        let (mut buf, mut caret) = (start.to_string(), None);
        for k in keys {
            edit(&mut buf, &mut caret, *k, 60, |_| true);
        }
        (buf, caret)
    }

    #[test]
    fn a_name_is_mended_in_the_middle() {
        // "Chaptr One": back five, put the e in.
        let back = [Key::Left; 5];
        let (buf, caret) = typed("Chaptr One", &[&back[..], &[Key::Char('e')]].concat());
        assert_eq!(buf, "Chapter One");
        assert_eq!(caret, Some(6));
        let (buf, _) = typed("Chapter Onee", &[Key::Left, Key::Backspace]);
        assert_eq!(buf, "Chapter One");
        let (buf, caret) = typed("xChapter", &[Key::Home, Key::Delete]);
        assert_eq!((buf.as_str(), caret), ("Chapter", Some(0)));
        // Past either end nothing happens, and nothing breaks.
        let (buf, caret) = typed(
            "é",
            &[Key::Left, Key::Left, Key::Backspace, Key::End, Key::Delete],
        );
        assert_eq!((buf.as_str(), caret), ("é", None));
    }

    #[test]
    fn a_full_box_takes_no_more_and_a_number_box_only_digits() {
        let (mut buf, mut caret) = ("12".to_string(), None);
        assert!(!edit(&mut buf, &mut caret, Key::Char('x'), 3, |c| c
            .is_ascii_digit()));
        edit(&mut buf, &mut caret, Key::Char('3'), 3, |c| {
            c.is_ascii_digit()
        });
        edit(&mut buf, &mut caret, Key::Char('4'), 3, |c| {
            c.is_ascii_digit()
        });
        assert_eq!(buf, "123");
    }

    #[test]
    fn the_view_slides_to_keep_the_caret_in_sight() {
        let t = crate::theme::presets().remove(0);
        let shown = |caret| {
            spans("abcdefghij", caret, 5, Style::default(), &t)
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        assert_eq!(shown(None), "ghij█");
        assert_eq!(shown(Some(0)), "abcde");
        assert_eq!(shown(Some(7)), "defgh");
    }
}
