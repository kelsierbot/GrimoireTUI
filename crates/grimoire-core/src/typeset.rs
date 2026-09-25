//! Typography for a finished book: what a word processor's autocorrect would
//! have done as you typed, done once at compile time instead.
//!
//! - `"` and `'` become curly quotes and apostrophes, opening or closing by
//!   what's around them — including the apostrophes that open a word ('tis,
//!   'em, rock 'n' roll, the '90s), which a naive pass gets backwards.
//! - `--` (and `---`) become an em dash with no spaces around it.
//! - A hyphen with a space either side, between words, becomes a dash too:
//!   closed up as an em dash (US) or spaced as an en dash (UK), by setting.
//! - `...` becomes an ellipsis.
//!
//! Markdown's own marks (`*`, `_`, `\`) are looked through when deciding which
//! way a quote faces, so `"*Wait*"` opens and closes where it should. A line
//! that is a scene break (`#`, `***`, `---`) is left exactly as it is; the
//! caller never passes those in.
//!
//! [`plain`] undoes it for the classic Courier manuscript, which keeps
//! straight quotes and `--`.

use std::borrow::Cow;

/// What a spaced hyphen between words becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpacedHyphen {
    /// `word—word`: the US book convention.
    #[default]
    Em,
    /// `word – word`: the UK one.
    En,
}

/// Words that begin with an apostrophe standing for letters left out. The
/// apostrophe before them is a closing mark (’), never an opening quote.
const ELIDED: [&str; 22] = [
    "tis", "twas", "twere", "twill", "twould", "em", "cause", "cos", "til", "till", "bout",
    "round", "n", "cept", "kay", "nuff", "fraid", "gainst", "neath", "twixt", "ere", "sup",
];

/// Characters Markdown uses for emphasis and escapes: looked through, as if
/// they weren't there, when deciding which way a quote faces.
fn is_mark(c: char) -> bool {
    matches!(c, '*' | '_' | '\\')
}

/// Does a quote after `c` open? After nothing, a space, or opening
/// punctuation (including a dash and another opening quote), it does.
fn opens_after(c: Option<char>) -> bool {
    match c {
        None => true,
        Some(c) => {
            c.is_whitespace()
                || matches!(
                    c,
                    '(' | '[' | '{' | '—' | '–' | '-' | '/' | '“' | '‘' | '"' | '\''
                )
        }
    }
}

/// `line` with book typography. Borrowed when there's nothing to change.
pub fn line(text: &str, hyphen: SpacedHyphen) -> Cow<'_, str> {
    if !text.chars().any(|c| matches!(c, '"' | '\'' | '-' | '.')) {
        return Cow::Borrowed(text);
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    // The last character written that wasn't a Markdown mark.
    let prev_real = |out: &String| out.chars().rev().find(|&c| !is_mark(c));
    let next_real = |from: usize| chars[from..].iter().copied().find(|&c| !is_mark(c));

    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                let before = prev_real(&out);
                out.push(if opens_after(before) { '“' } else { '”' });
                i += 1;
            }
            '\'' => {
                let before = prev_real(&out);
                let after = next_real(i + 1);
                let word_after: String = chars[i + 1..]
                    .iter()
                    .skip_while(|c| is_mark(**c))
                    .take_while(|c| c.is_alphanumeric())
                    .collect();
                let elided = opens_after(before)
                    && (ELIDED.contains(&word_after.to_lowercase().as_str())
                        // '90s, '08
                        || (word_after.len() >= 2
                            && word_after.chars().take(2).all(|d| d.is_ascii_digit())));
                let apostrophe = before.is_some_and(char::is_alphanumeric)
                    && after.is_some_and(char::is_alphanumeric);
                out.push(if elided || apostrophe || !opens_after(before) {
                    '’'
                } else {
                    '‘'
                });
                i += 1;
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                // `--` or `---`, spaces around it taken in: an em dash, closed up.
                let mut end = i;
                while chars.get(end) == Some(&'-') {
                    end += 1;
                }
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push('—');
                i = end;
                while chars.get(i) == Some(&' ') && i + 1 < chars.len() {
                    i += 1;
                }
            }
            '-' if i > 0
                && chars[i - 1] == ' '
                && chars.get(i + 1) == Some(&' ')
                && out
                    .trim_end()
                    .chars()
                    .last()
                    .is_some_and(|c| !c.is_whitespace())
                && chars[i + 2..].iter().any(|c| !c.is_whitespace()) =>
            {
                match hyphen {
                    SpacedHyphen::Em => {
                        while out.ends_with(' ') {
                            out.pop();
                        }
                        out.push('—');
                        i += 1;
                        while chars.get(i) == Some(&' ') {
                            i += 1;
                        }
                    }
                    SpacedHyphen::En => {
                        out.push('–');
                        i += 1;
                    }
                }
            }
            '.' if chars.get(i + 1) == Some(&'.') && chars.get(i + 2) == Some(&'.') => {
                out.push('…');
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    if out == text {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(out)
    }
}

/// A scene body with book typography, line by line, scene breaks untouched.
pub fn body(text: &str, hyphen: SpacedHyphen, is_break: impl Fn(&str) -> bool) -> Cow<'_, str> {
    let mut changed = false;
    let lines: Vec<Cow<'_, str>> = text
        .split('\n')
        .map(|l| {
            if is_break(l.trim()) {
                return Cow::Borrowed(l);
            }
            let t = line(l, hyphen);
            changed |= matches!(t, Cow::Owned(_));
            t
        })
        .collect();
    if changed {
        Cow::Owned(lines.join("\n"))
    } else {
        Cow::Borrowed(text)
    }
}

/// Back to what a typewriter could type: straight quotes, `--`, `...`. For
/// the classic Courier manuscript.
pub fn plain(text: &str) -> Cow<'_, str> {
    if !text
        .chars()
        .any(|c| matches!(c, '“' | '”' | '‘' | '’' | '—' | '–' | '…'))
    {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '“' | '”' => out.push('"'),
            '‘' | '’' => out.push('\''),
            '—' => out.push_str("--"),
            '–' => out.push('-'),
            '…' => out.push_str("..."),
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> String {
        line(s, SpacedHyphen::Em).into_owned()
    }

    #[test]
    fn double_quotes_open_and_close() {
        assert_eq!(t("\"Wait,\" she said."), "“Wait,” she said.");
        assert_eq!(t("He said, \"Go.\""), "He said, “Go.”");
        assert_eq!(t("(\"quoted\")"), "(“quoted”)");
    }

    #[test]
    fn markdown_marks_are_looked_through() {
        assert_eq!(t("\"*Wait*\" she said."), "“*Wait*” she said.");
        assert_eq!(t("*\"Wait\"* she said."), "*“Wait”* she said.");
    }

    #[test]
    fn apostrophes_and_single_quotes() {
        assert_eq!(t("don't O'Brien's"), "don’t O’Brien’s");
        assert_eq!(t("'Hello,' she said."), "‘Hello,’ she said.");
        assert_eq!(t("\"She said 'no.'\""), "“She said ‘no.’”");
        assert_eq!(t("the dogs' bowls"), "the dogs’ bowls");
    }

    #[test]
    fn leading_apostrophes_face_the_right_way() {
        assert_eq!(t("'Tis the season"), "’Tis the season");
        assert_eq!(t("give 'em hell"), "give ’em hell");
        assert_eq!(t("rock 'n' roll"), "rock ’n’ roll");
        assert_eq!(t("back in the '90s"), "back in the ’90s");
        assert_eq!(t("the class of '08"), "the class of ’08");
    }

    #[test]
    fn dashes_and_ellipses() {
        assert_eq!(t("wait--no"), "wait—no");
        assert_eq!(t("wait -- no"), "wait—no");
        assert_eq!(t("wait---no"), "wait—no");
        assert_eq!(t("wait - no"), "wait—no");
        assert_eq!(
            line("wait - no", SpacedHyphen::En).into_owned(),
            "wait – no"
        );
        assert_eq!(t("well-known"), "well-known", "a hyphen in a word stays");
        assert_eq!(t("and then..."), "and then…");
        assert_eq!(t("- a list?"), "- a list?", "a hyphen opening a line stays");
    }

    #[test]
    fn untouched_text_is_borrowed() {
        assert!(matches!(
            line("Nothing to do here", SpacedHyphen::Em),
            Cow::Borrowed(_)
        ));
        let body_text = "One.\n#\nTwo \"quoted\".";
        let out = body(body_text, SpacedHyphen::Em, |l| l == "#");
        assert_eq!(out, "One.\n#\nTwo “quoted”.");
        assert_eq!(
            body("---\nx", SpacedHyphen::Em, |l| l == "---"),
            "---\nx",
            "a break line is never turned into a dash"
        );
    }

    #[test]
    fn plain_undoes_it() {
        let fancy = t("\"Wait--'tis...\"");
        assert_eq!(plain(&fancy), "\"Wait--'tis...\"");
    }
}
