//! Revision passes: the scenes still waiting for one, and words that echo.

use std::collections::HashMap;

use crate::notes;
use crate::project::{Kind, Project};

/// Every scene in the order the tree shows them — each section in turn, the
/// manuscript in book order — leaving out the trash.
pub fn scenes_in_order(p: &Project) -> Vec<usize> {
    fn walk(p: &Project, i: usize, out: &mut Vec<usize>) {
        if p.in_trash(i) {
            return;
        }
        match p.nodes[i].kind {
            Kind::Scene => out.push(i),
            _ => {
                for &c in &p.nodes[i].children {
                    walk(p, c, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for &r in &p.roots {
        walk(p, r, &mut out);
    }
    out
}

/// A scene is still in draft until its status says it has been revised: no
/// status, `idea`, `outline`, `draft` or anything unrecognised all count.
pub fn in_draft(status: Option<&str>) -> bool {
    !matches!(
        status.map(|s| s.trim().to_lowercase()).as_deref(),
        Some("revised" | "done")
    )
}

/// Manuscript scenes with words in them that are still in draft, in book order.
pub fn drafts(p: &Project) -> Vec<usize> {
    scenes_in_order(p)
        .into_iter()
        .filter(|&i| {
            let n = &p.nodes[i];
            n.in_manuscript && !n.front_matter && n.words() > 0 && in_draft(n.status.as_deref())
        })
        .collect()
}

/// The next scene after `from`, in book order and wrapping round, that is
/// still in draft. `from` itself comes last, so a book with one draft left
/// finds it. None when every written scene is revised or done.
pub fn next_in_draft(p: &Project, from: Option<usize>) -> Option<usize> {
    let drafts = drafts(p);
    let order = scenes_in_order(p);
    let here = from.and_then(|f| order.iter().position(|&i| i == f));
    let Some(here) = here else {
        return drafts.first().copied();
    };
    let rank = |i: usize| order.iter().position(|&o| o == i).unwrap_or(0);
    drafts
        .iter()
        .copied()
        .find(|&d| rank(d) > here)
        .or_else(|| drafts.first().copied())
}

/// How far back (in words) a repeat still counts as an echo.
pub const ECHO_WINDOW: usize = 40;

/// Words that repeat by design, or too common to notice.
const STOP: &[&str] = &[
    "about",
    "above",
    "after",
    "again",
    "against",
    "along",
    "also",
    "among",
    "another",
    "around",
    "asked",
    "away",
    "back",
    "because",
    "been",
    "before",
    "behind",
    "being",
    "below",
    "between",
    "both",
    "came",
    "come",
    "could",
    "didn't",
    "does",
    "doesn't",
    "done",
    "down",
    "during",
    "each",
    "even",
    "ever",
    "every",
    "from",
    "gave",
    "give",
    "going",
    "gone",
    "have",
    "having",
    "he'd",
    "he's",
    "hers",
    "herself",
    "himself",
    "into",
    "it's",
    "itself",
    "just",
    "know",
    "knew",
    "like",
    "made",
    "make",
    "many",
    "might",
    "more",
    "most",
    "much",
    "must",
    "myself",
    "never",
    "nothing",
    "once",
    "only",
    "onto",
    "other",
    "ourselves",
    "over",
    "said",
    "same",
    "says",
    "she'd",
    "she's",
    "should",
    "since",
    "some",
    "something",
    "still",
    "such",
    "than",
    "that",
    "that's",
    "their",
    "theirs",
    "them",
    "themselves",
    "then",
    "there",
    "there's",
    "these",
    "they",
    "they'd",
    "they're",
    "thing",
    "this",
    "those",
    "though",
    "through",
    "till",
    "toward",
    "towards",
    "under",
    "until",
    "upon",
    "very",
    "want",
    "wanted",
    "wasn't",
    "we'd",
    "we're",
    "well",
    "went",
    "were",
    "weren't",
    "what",
    "what's",
    "when",
    "where",
    "which",
    "while",
    "will",
    "with",
    "within",
    "without",
    "won't",
    "would",
    "wouldn't",
    "yeah",
    "your",
    "yours",
    "yourself",
];

/// "walked", "walking", "walks" → "walk": close enough to hear the echo.
fn stem(word: &str) -> String {
    let w = word.trim_end_matches("'s").trim_end_matches("’s");
    for (suffix, min) in [("ing", 6), ("ed", 5), ("es", 5), ("ly", 5), ("s", 4)] {
        if w.len() >= min && w.ends_with(suffix) && !(suffix == "s" && w.ends_with("ss")) {
            return w[..w.len() - suffix.len()].to_string();
        }
    }
    w.to_string()
}

/// Words that echo in a scene: a word of four letters or more (or its near
/// forms) used again within [`ECHO_WINDOW`] words. Per line, the char ranges
/// of every echoing word. Common words, anything in `skip` (the notebook's
/// names), and words inside notes are left alone.
pub fn echoes(lines: &[String], skip: &[String]) -> Vec<Vec<(usize, usize)>> {
    let skip: Vec<String> = skip
        .iter()
        .flat_map(|n| n.split_whitespace().map(|w| stem(&w.to_lowercase())))
        .collect();
    let marks = notes::line_spans(lines);
    let mut out = vec![Vec::new(); lines.len()];
    let mut last: HashMap<String, (usize, usize, usize, usize)> = HashMap::new();
    let mut n = 0usize;
    for (li, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut c = 0;
        while c < chars.len() {
            if !chars[c].is_alphabetic() {
                c += 1;
                continue;
            }
            let start = c;
            while c < chars.len()
                && (chars[c].is_alphabetic()
                    || ((chars[c] == '\'' || chars[c] == '’')
                        && chars.get(c + 1).is_some_and(|x| x.is_alphabetic())))
            {
                c += 1;
            }
            let end = c;
            if marks[li].iter().any(|&(s, e, _)| start < e && end > s) {
                continue;
            }
            n += 1;
            let word: String = chars[start..end].iter().collect::<String>().to_lowercase();
            if word.chars().count() < 4 || STOP.contains(&word.as_str()) {
                continue;
            }
            let key = stem(&word);
            if key.chars().count() < 4 || skip.contains(&key) {
                continue;
            }
            if let Some(&(pn, pl, ps, pe)) = last.get(&key)
                && n - pn <= ECHO_WINDOW
            {
                if !out[pl].contains(&(ps, pe)) {
                    out[pl].push((ps, pe));
                }
                out[li].push((start, end));
            }
            last.insert(key, (n, li, start, end));
        }
    }
    out
}

/// How many words echo, counted once each.
pub fn echo_count(echoes: &[Vec<(usize, usize)>]) -> usize {
    echoes.iter().map(Vec::len).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn lines(s: &str) -> Vec<String> {
        s.split('\n').map(str::to_string).collect()
    }

    #[test]
    fn a_word_repeated_close_by_echoes_and_far_apart_does_not() {
        let e = echoes(
            &lines("The lantern swung. She lifted the lantern higher."),
            &[],
        );
        assert_eq!(e[0], vec![(4, 11), (34, 41)]);

        let far = format!(
            "The lantern swung. {} Then the lantern.",
            "so a ".repeat(25)
        );
        assert!(echoes(&lines(&far), &[])[0].is_empty());
    }

    #[test]
    fn near_forms_echo_but_common_words_and_names_do_not() {
        let e = echoes(
            &lines("She walked in.\nWalking out, Wren said that Wren would."),
            &["Wren".into()],
        );
        assert_eq!(e[0], vec![(4, 10)]);
        assert_eq!(e[1], vec![(0, 7)]);
        let none = echoes(&lines("They said that. They said that."), &[]);
        assert!(none.iter().all(Vec::is_empty));
    }

    #[test]
    fn words_in_notes_are_not_echoes() {
        let e = echoes(
            &lines("The gravel crunched. %% more gravel here? %% Done."),
            &[],
        );
        assert!(e[0].is_empty(), "{:?}", e[0]);
    }

    fn put(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn book(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("grimoire-revision-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        put(
            &d,
            "manuscript/01-Chapter-One/01-A.md",
            "---\nstatus: revised\n---\n\nWords.\n",
        );
        put(
            &d,
            "manuscript/01-Chapter-One/02-B.md",
            "---\nstatus: draft\n---\n\nWords.\n",
        );
        put(&d, "manuscript/01-Chapter-One/03-C.md", "");
        put(&d, "manuscript/02-Chapter-Two/01-D.md", "No status yet.\n");
        put(
            &d,
            "manuscript/02-Chapter-Two/02-E.md",
            "---\nstatus: Done\n---\n\nWords.\n",
        );
        put(&d, "notes/idea.md", "A note, not a scene to revise.\n");
        d
    }

    #[test]
    fn the_next_draft_skips_revised_done_and_empty_scenes_and_wraps() {
        let d = book("next");
        let p = Project::load(&d).unwrap();
        let find = |name: &str| p.nodes.iter().position(|n| n.path.ends_with(name)).unwrap();
        let (a, b, dd, e) = (
            find("01-A.md"),
            find("02-B.md"),
            find("01-D.md"),
            find("02-E.md"),
        );
        assert_eq!(drafts(&p), vec![b, dd]);
        assert_eq!(next_in_draft(&p, Some(a)), Some(b));
        assert_eq!(next_in_draft(&p, Some(b)), Some(dd));
        assert_eq!(next_in_draft(&p, Some(dd)), Some(b));
        assert_eq!(next_in_draft(&p, Some(e)), Some(b));
        assert_eq!(next_in_draft(&p, None), Some(b));
        let _ = fs::remove_dir_all(&d);
    }
}
