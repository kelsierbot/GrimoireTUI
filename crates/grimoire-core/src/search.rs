//! Finding and replacing text, in a scene or across the whole book, and
//! catching name drift: near-miss spellings of the names in the notebook.
//!
//! The find bar matches by [`Opts`] — whole words and exact case unless the
//! writer loosens either, because what the bar finds is what "replace all"
//! changes. [`matches`] on its own is "smart case": all-lowercase finds any
//! case, a capital in the query means exactly that. Matches never span
//! paragraphs, and positions are char indices so they line up with the editor.

use std::path::PathBuf;

use crate::project::{Kind, Project};

/// How the find bar matches. The default is the safe one for replacing:
/// whole words in their exact case, so replacing "ann" never touches
/// "planned" and replacing "the" never touches "The".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opts {
    /// Only where the query stands as a word of its own.
    pub whole_words: bool,
    /// Capitals count: "Lantern" and "lantern" are different.
    pub match_case: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            whole_words: true,
            match_case: true,
        }
    }
}

/// Char ranges of `query` in `line`, smart case, inside words too.
pub fn matches(line: &str, query: &str) -> Vec<(usize, usize)> {
    let exact = query.chars().any(char::is_uppercase);
    find_in(line, query, exact, false)
}

/// Char ranges of `query` in `line`, the way `opts` says.
pub fn matches_with(line: &str, query: &str, opts: Opts) -> Vec<(usize, usize)> {
    find_in(line, query, opts.match_case, opts.whole_words)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn find_in(line: &str, query: &str, exact: bool, whole: bool) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    // A boundary only matters where the query itself starts or ends in a
    // letter: "Mr." still matches before a space.
    let (starts_word, ends_word) = (
        query.chars().next().is_some_and(is_word),
        query.chars().last().is_some_and(is_word),
    );
    let fold = |s: &str| -> Vec<char> {
        if exact {
            s.chars().collect()
        } else {
            s.chars().flat_map(char::to_lowercase).collect()
        }
    };
    // Lowercasing can change length for a few characters; map back through
    // per-char lowercasing so indices stay aligned with the original.
    let hay: Vec<char> = line.chars().collect();
    let hay_folded: Vec<Vec<char>> = hay
        .iter()
        .map(|c| {
            if exact {
                vec![*c]
            } else {
                c.to_lowercase().collect()
            }
        })
        .collect();
    let needle = fold(query);
    let mut out = Vec::new();
    let mut i = 0;
    while i < hay.len() {
        // Try to match `needle` starting at char i.
        let mut k = 0usize;
        let mut j = i;
        while j < hay.len() && k < needle.len() {
            let f = &hay_folded[j];
            if needle.len() - k < f.len() || needle[k..k + f.len()] != f[..] {
                break;
            }
            k += f.len();
            j += 1;
        }
        let bounded = !whole
            || ((!starts_word || i == 0 || !is_word(hay[i - 1]))
                && (!ends_word || j == hay.len() || !is_word(hay[j])));
        if k == needle.len() && bounded {
            out.push((i, j));
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

/// Replace every match in `text` (paragraph by paragraph). Returns the new
/// text and how many were replaced.
pub fn replace_all(text: &str, query: &str, with: &str, opts: Opts) -> (String, usize) {
    let mut count = 0;
    let lines: Vec<String> = text
        .split('\n')
        .map(|line| {
            let hits = matches_with(line, query, opts);
            if hits.is_empty() {
                return line.to_string();
            }
            count += hits.len();
            splice(line, &hits, with)
        })
        .collect();
    (lines.join("\n"), count)
}

fn splice(line: &str, hits: &[(usize, usize)], with: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut at = 0;
    for &(s, e) in hits {
        out.extend(&chars[at..s]);
        out.push_str(with);
        at = e;
    }
    out.extend(&chars[at..]);
    out
}

/// One match somewhere in the book.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub path: PathBuf,
    /// "Act One › Chapter Three › The Lantern", or "Characters › Kaelen".
    pub place: String,
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Where a node lives, by its containers' names.
pub fn place_of(p: &Project, parents: &[Option<usize>], idx: usize) -> String {
    let mut trail = vec![p.nodes[idx].title.clone()];
    let mut up = parents[idx];
    while let Some(pi) = up {
        if p.nodes[pi].kind == Kind::Container {
            trail.push(p.nodes[pi].title.clone());
        }
        up = parents[pi];
    }
    trail.reverse();
    trail.join(" › ")
}

/// Every match in the manuscript and notebook, in book order. The trash is
/// left out.
pub fn book(p: &Project, parents: &[Option<usize>], query: &str, opts: Opts) -> Vec<Hit> {
    let mut out = Vec::new();
    if query.is_empty() {
        return out;
    }
    for &r in &p.roots {
        walk(p, parents, r, query, opts, &mut out);
    }
    out
}

fn walk(
    p: &Project,
    parents: &[Option<usize>],
    idx: usize,
    query: &str,
    opts: Opts,
    out: &mut Vec<Hit>,
) {
    let n = &p.nodes[idx];
    if p.in_trash(idx) {
        return;
    }
    if n.kind == Kind::Scene {
        let place = place_of(p, parents, idx);
        for (li, line) in n.body.split('\n').enumerate() {
            for (s, e) in matches_with(line, query, opts) {
                out.push(Hit {
                    path: n.path.clone(),
                    place: place.clone(),
                    line: li,
                    start: s,
                    end: e,
                    text: line.to_string(),
                });
            }
        }
    }
    for &c in &n.children {
        walk(p, parents, c, query, opts, out);
    }
}

// ── name drift ───────────────────────────────────────────────────────

/// A name the notebook knows: a note's title, and which section it's in.
#[derive(Debug, Clone, PartialEq)]
pub struct Name {
    pub name: String,
    pub section: String,
}

/// Names from the notebook: each note's title, and each capitalised word of a
/// multi-word title ("Ashen Reach" gives "Ashen Reach", "Ashen" and "Reach").
pub fn names(p: &Project, parents: &[Option<usize>]) -> Vec<Name> {
    let mut out: Vec<Name> = Vec::new();
    for (i, n) in p.nodes.iter().enumerate() {
        if n.kind != Kind::Scene || !n.area.is_notebook() || p.in_trash(i) {
            continue;
        }
        // The top-level folder the note sits in: Characters, Regions…
        let mut section = String::new();
        let mut up = parents[i];
        while let Some(pi) = up {
            if p.nodes[pi].kind == Kind::Container {
                section = p.nodes[pi].title.clone();
            }
            up = parents[pi];
        }
        let mut add = |name: &str| {
            if !out.iter().any(|x| x.name == name) {
                out.push(Name {
                    name: name.to_string(),
                    section: section.clone(),
                });
            }
        };
        add(n.title.trim());
        let words: Vec<&str> = n.title.split_whitespace().collect();
        if words.len() > 1 {
            for w in words {
                let w = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '’');
                if w.chars().next().is_some_and(char::is_uppercase) && w.chars().count() >= 4 {
                    add(w);
                }
            }
        }
    }
    out
}

/// A spelling in the manuscript that is probably a slip for a notebook name.
#[derive(Debug, Clone, PartialEq)]
pub struct Drift {
    pub variant: String,
    pub name: Name,
    pub hits: Vec<Hit>,
}

/// Capitalised words in the manuscript that are one or two letters away from
/// a notebook name but aren't that name. `known` says whether a word is an
/// ordinary word (from the spellchecker's dictionary), so "Reach" and "Peach"
/// never get flagged against each other.
pub fn drift(
    p: &Project,
    parents: &[Option<usize>],
    names: &[Name],
    known: &dyn Fn(&str) -> bool,
) -> Vec<Drift> {
    let single: Vec<&Name> = names
        .iter()
        .filter(|n| !n.name.contains(' ') && n.name.chars().count() >= 4)
        .collect();
    let mut out: Vec<Drift> = Vec::new();
    for (i, n) in p.nodes.iter().enumerate() {
        if n.kind != Kind::Scene || !n.in_manuscript || p.in_trash(i) {
            continue;
        }
        let place = place_of(p, parents, i);
        for (li, line) in n.body.split('\n').enumerate() {
            for (s, e, word) in capitalised_words(line) {
                if word.chars().count() < 4 || single.iter().any(|x| x.name == word) || known(&word)
                {
                    continue;
                }
                let Some(name) = single
                    .iter()
                    .filter(|x| close(&x.name, &word))
                    .min_by_key(|x| distance(&x.name.to_lowercase(), &word.to_lowercase()))
                else {
                    continue;
                };
                let hit = Hit {
                    path: n.path.clone(),
                    place: place.clone(),
                    line: li,
                    start: s,
                    end: e,
                    text: line.to_string(),
                };
                match out
                    .iter_mut()
                    .find(|d| d.variant == word && d.name.name == name.name)
                {
                    Some(d) => d.hits.push(hit),
                    None => out.push(Drift {
                        variant: word,
                        name: (*name).clone(),
                        hits: vec![hit],
                    }),
                }
            }
        }
    }
    out
}

/// Close enough to be a slip: one edit for short names, two for long ones,
/// and never just a difference in case.
fn close(name: &str, word: &str) -> bool {
    let (a, b) = (name.to_lowercase(), word.to_lowercase());
    if a == b {
        return false;
    }
    let len = a.chars().count().max(b.chars().count());
    let d = distance(&a, &b);
    d >= 1 && d <= if len >= 7 { 2 } else { 1 }
}

/// Capitalised words with their char ranges; a trailing possessive is left
/// off ("Kaelan's" → "Kaelan").
fn capitalised_words(line: &str) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_alphabetic() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len()
            && (chars[i].is_alphabetic()
                || ((chars[i] == '\'' || chars[i] == '’')
                    && chars.get(i + 1).is_some_and(|c| c.is_alphabetic())))
        {
            i += 1;
        }
        let mut end = i;
        let word: String = chars[start..end].iter().collect();
        if let Some(stem) = word.strip_suffix("'s").or_else(|| word.strip_suffix("’s")) {
            end -= 2;
            if chars[start].is_uppercase() {
                out.push((start, end, stem.to_string()));
            }
            continue;
        }
        if chars[start].is_uppercase() {
            out.push((start, end, word));
        }
    }
    out
}

/// Damerau–Levenshtein distance (adjacent swaps count as one edit).
pub fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[n][m]
}

/// Replace a whole word, exactly as spelled, keeping a possessive intact.
pub fn replace_word(text: &str, from: &str, to: &str) -> (String, usize) {
    let mut count = 0;
    let lines: Vec<String> = text
        .split('\n')
        .map(|line| {
            let hits: Vec<(usize, usize)> = capitalised_words(line)
                .into_iter()
                .filter(|(_, _, w)| w == from)
                .map(|(s, e, _)| (s, e))
                .collect();
            count += hits.len();
            if hits.is_empty() {
                line.to_string()
            } else {
                splice(line, &hits, to)
            }
        })
        .collect();
    (lines.join("\n"), count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lowercase_finds_any_case_and_a_capital_means_exactly() {
        assert_eq!(
            matches("The lantern, the Lantern", "lantern"),
            vec![(4, 11), (17, 24)]
        );
        assert_eq!(
            matches("The lantern, the Lantern", "Lantern"),
            vec![(17, 24)]
        );
        assert_eq!(matches("aaa", "aa"), vec![(0, 2)], "matches don't overlap");
        assert!(matches("anything", "").is_empty());
    }

    #[test]
    fn positions_are_chars_not_bytes() {
        // Curly quotes and an em dash are multi-byte.
        let line = "“Kaelen”—the lantern";
        assert_eq!(matches(line, "lantern"), vec![(13, 20)]);
        assert_eq!(line.chars().skip(13).take(7).collect::<String>(), "lantern");
    }

    #[test]
    fn replacing_counts_and_keeps_paragraphs() {
        let loose = Opts {
            whole_words: false,
            match_case: false,
        };
        let (out, n) = replace_all(
            "a lantern\nno match\nLantern lantern",
            "lantern",
            "lamp",
            loose,
        );
        assert_eq!(out, "a lamp\nno match\nlamp lamp");
        assert_eq!(n, 3);
    }

    #[test]
    fn replacing_by_default_touches_only_whole_words_in_their_case() {
        let text = "Ann planned it. ann's too. The ANN banner. Ann's coat.";
        let (out, n) = replace_all(text, "Ann", "Mara", Opts::default());
        assert_eq!(
            out,
            "Mara planned it. ann's too. The ANN banner. Mara's coat."
        );
        assert_eq!(n, 2);
        let (out, _) = replace_all("the theme. The end.", "the", "a", Opts::default());
        assert_eq!(
            out, "a theme. The end.",
            "neither inside a word nor a capital"
        );
        // Loosened on purpose, it does what it says.
        let inside = Opts {
            whole_words: false,
            ..Opts::default()
        };
        assert_eq!(replace_all("planned", "ann", "X", inside).0, "plXed");
        let any_case = Opts {
            match_case: false,
            ..Opts::default()
        };
        assert_eq!(replace_all("The end", "the", "a", any_case).0, "a end");
        // Punctuation at the query's edge needs no boundary.
        assert_eq!(
            matches_with("Mr.Holt", "Mr.", Opts::default()),
            vec![(0, 3)]
        );
    }

    #[test]
    fn swaps_and_slips_are_close_but_different_names_are_not() {
        assert!(close("Kaelen", "Kaelan"));
        assert!(close("Kaelen", "Kealen"), "a swap is one edit");
        assert!(!close("Kaelen", "Kaelen"));
        assert!(!close("Wren", "Oren's"), "short names allow one edit only");
        assert!(close("Vael'thar", "Vaelthar"));
        assert!(!close("Kaelen", "Darren"));
    }

    fn book_with(tag: &str, scene: &str, note_title: &str) -> (PathBuf, Project) {
        let d = std::env::temp_dir().join(format!("grimoire-search-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("manuscript/01-Act-One/01-Chapter-One")).unwrap();
        fs::create_dir_all(d.join("notes/01-Characters")).unwrap();
        fs::write(
            d.join("manuscript/01-Act-One/01-Chapter-One/01-Ashfall.md"),
            scene,
        )
        .unwrap();
        fs::write(
            d.join(format!(
                "notes/01-Characters/01-{}.md",
                note_title.replace(' ', "-")
            )),
            format!("---\ntitle: {note_title}\n---\n\nFerryman's son.\n"),
        )
        .unwrap();
        let p = Project::load(&d).unwrap();
        (d, p)
    }

    #[test]
    fn drift_finds_a_misspelt_name_and_where_it_is() {
        let (d, p) = book_with(
            "drift",
            "Kaelen lifted it.\nThen Kaelan's hand shook. Kaelan ran.\nDarren watched.",
            "Kaelen",
        );
        let parents = p.parents();
        let names = names(&p, &parents);
        let found = drift(&p, &parents, &names, &|_| false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].variant, "Kaelan");
        assert_eq!(found[0].name.name, "Kaelen");
        assert_eq!(found[0].name.section, "Characters");
        assert_eq!(found[0].hits.len(), 2);
        assert_eq!(found[0].hits[0].place, "Act One › Chapter One › Ashfall");
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn ordinary_words_are_never_drift() {
        let (d, p) = book_with("ordinary", "The Peach tree.", "Ashen Reach");
        let parents = p.parents();
        let names = names(&p, &parents);
        assert!(names.iter().any(|n| n.name == "Reach"));
        let known = |w: &str| w == "Peach";
        assert!(drift(&p, &parents, &names, &known).is_empty());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn fixing_a_name_keeps_possessives_and_leaves_other_words() {
        let (out, n) = replace_word("Kaelan's hand. Kaelan ran. Kaelanish.", "Kaelan", "Kaelen");
        assert_eq!(out, "Kaelen's hand. Kaelen ran. Kaelanish.");
        assert_eq!(n, 2);
    }

    #[test]
    fn book_search_skips_the_trash_and_names_the_place() {
        let (d, p) = book_with("book", "the lantern\nand another lantern", "Kaelen");
        let parents = p.parents();
        let hits = book(&p, &parents, "lantern", Opts::default());
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[1].line, hits[1].start), (1, 12));
        assert_eq!(hits[0].place, "Act One › Chapter One › Ashfall");
        fs::remove_dir_all(&d).unwrap();
    }
}
