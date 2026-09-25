//! Spellcheck for prose.
//!
//! A bundled en_US Hunspell dictionary (SCOWL, see `assets/dict/DICTIONARY-LICENSE`) is
//! checked with `spellbook`, a pure-Rust Hunspell reimplementation, so there is
//! nothing to install and nothing to link on any platform.
//!
//! Most of this file is the tokeniser, because that is where spellcheckers
//! annoy novelists: contractions, possessives, hyphenated compounds, curly
//! quotes, em dashes, Markdown emphasis and chapter numerals all pass, and the
//! book's own invented names (notebook titles plus `dictionary.txt` at the
//! project root) are never flagged.

use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

const EN_US_AFF: &str = include_str!("../../../assets/dict/en_US.aff");
const EN_US_DIC: &str = include_str!("../../../assets/dict/en_US.dic");
/// Real words the standard-size list lacks: "grey", "axe", "mana", "wyvern".
const EXTRA_WORDS: &str = include_str!("../../../assets/dict/extra_words.dic");

/// The book's own word list, at the project root.
const BOOK_DICTIONARY: &str = "dictionary.txt";
const BOOK_DICTIONARY_HEADER: &str = "# Words the spellchecker accepts in this book (names, places, invented terms), one per line; lines starting with # are ignored.\n";
const USER_DICTIONARY_HEADER: &str = "# Words the spellchecker accepts in every book, one per line; lines starting with # are ignored.\n";

/// Checked words remembered before the cache starts over.
const CACHE_LIMIT: usize = 50_000;

/// Contraction and possessive endings. If the whole word isn't known, the
/// stem before one of these is checked instead ("Kaelen's", "Vael'thar'll").
const ENDINGS: [&str; 7] = ["'s", "n't", "'ll", "'re", "'ve", "'d", "'m"];

/// Prefixes accepted before a hyphen even though the dictionary doesn't list
/// them as words on their own ("pre-war", "non-magical", "un-American").
const HYPHEN_PREFIXES: &[&str] = &[
    "anti", "bi", "co", "counter", "de", "ex", "extra", "hyper", "inter", "intra", "mega", "micro",
    "mid", "mini", "mis", "multi", "neo", "non", "over", "post", "pre", "pro", "proto", "pseudo",
    "quasi", "re", "semi", "sub", "super", "tri", "uber", "ultra", "un", "under", "vice",
];

/// File extensions and top-level domains that mark a dotted token as a web
/// address or file name rather than two words missing a space.
const DOTTED_ENDINGS: &[&str] = &[
    "com", "org", "net", "io", "ai", "dev", "edu", "gov", "app", "uk", "info", "md", "txt", "html",
    "htm", "pdf", "png", "jpg", "json", "toml",
];

/// Checks prose against the bundled dictionary plus the book's own words.
/// `Send + Sync`: build it on a worker thread and share it behind an `Arc`.
pub struct Speller {
    dict: spellbook::Dictionary,
    /// Words accepted on top of the dictionary: lowercased, apostrophes
    /// straightened, possessive dropped.
    added: HashSet<String>,
    /// Word (apostrophes straightened) -> correct?
    cache: Mutex<HashMap<String, bool>>,
}

impl Speller {
    /// A speller over the bundled en_US dictionary. Parsing it takes a few
    /// milliseconds in a release build (far longer in debug), so build it
    /// off the UI thread.
    pub fn new() -> Speller {
        let mut dict = spellbook::Dictionary::new(EN_US_AFF, EN_US_DIC)
            .expect("the bundled en_US dictionary parses");
        for line in EXTRA_WORDS.lines().map(str::trim) {
            if !line.is_empty() && !line.starts_with('#') {
                dict.add(line)
                    .expect("extra_words.dic uses en_US.aff flags");
            }
        }
        Speller {
            dict,
            added: HashSet::new(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Accept these words as correct in any capitalisation: notebook names,
    /// the book's own word list. Each string is tokenised like prose, so
    /// "Ashen Reach" adds both words and "Kaelen's" adds "Kaelen".
    pub fn add_words<I: IntoIterator<Item = String>>(&mut self, words: I) {
        let mut changed = false;
        for text in words {
            let chars: Vec<char> = text.chars().collect();
            for part in parts(&chars) {
                let word = drop_possessive(&straighten(&chars[part.start..part.end]));
                if word.chars().count() < 2 || !self.added.insert(word_key(&word)) {
                    continue;
                }
                changed = true;
                // Also teach the dictionary, so a mistyped name gets the real
                // one as a suggestion. `add` reads `.dic` syntax, where '/'
                // starts flags; the tokeniser never yields one.
                if !self.dict.check(&word) {
                    let _ = self.dict.add(&word);
                }
            }
        }
        if changed {
            // Only "misspelled" verdicts can change.
            self.cache
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner)
                .retain(|_, ok| *ok);
        }
    }

    /// Whether a word (or short phrase) has no misspellings. Punctuation,
    /// numbers and web addresses are ignored, so `is_correct("")` is true.
    pub fn is_correct(&self, word: &str) -> bool {
        let chars: Vec<char> = word.chars().collect();
        parts(&chars).iter().all(|p| self.part_ok(&chars, p))
    }

    /// Up to `max` corrections, best first. A possessive or contraction keeps
    /// its ending ("Kaelan's" -> "Kaelen's"), and curly apostrophes stay curly.
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        let trimmed = word.trim_matches(|c: char| !is_letter(c));
        if max == 0 || trimmed.is_empty() {
            return Vec::new();
        }
        let curly = trimmed.contains('’');
        let chars: Vec<char> = trimmed.chars().collect();
        let word = straighten(&chars);

        let (stem, ending) = match split_ending(&word) {
            Some((stem, ending)) if !self.known(&word) => (stem, ending),
            _ => (word.as_str(), ""),
        };
        let mut raw = Vec::new();
        self.dict.suggest(stem, &mut raw);

        let mut out: Vec<String> = Vec::new();
        for s in raw {
            // Hunspell also proposes splits like "Kaelenn" -> "Kaelen n"; a
            // stray letter is never what the writer meant ("a lot" is fine).
            if s.split(' ')
                .any(|w| w.chars().count() == 1 && !matches!(w, "a" | "A" | "I"))
            {
                continue;
            }
            let mut s = s + ending;
            if curly {
                s = s.replace('\'', "’");
            }
            if s != trimmed && !out.contains(&s) {
                out.push(s);
            }
            if out.len() == max {
                break;
            }
        }
        out
    }

    /// Char ranges `[start, end)` of the misspelled words in one paragraph.
    pub fn misspellings(&self, line: &str) -> Vec<(usize, usize)> {
        let chars: Vec<char> = line.chars().collect();
        parts(&chars)
            .into_iter()
            .filter(|p| !self.part_ok(&chars, p))
            .map(|p| (p.start, p.end))
            .collect()
    }

    fn part_ok(&self, chars: &[char], part: &Part) -> bool {
        let word = straighten(&chars[part.start..part.end]);
        if is_roman_numeral(&word) {
            return true;
        }
        if part.before_hyphen && HYPHEN_PREFIXES.contains(&word.to_lowercase().as_str()) {
            return true;
        }
        let lock = || self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let cached = lock().get(&word).copied();
        let ok = cached.unwrap_or_else(|| {
            let ok = self.check(&word);
            let mut cache = lock();
            if cache.len() >= CACHE_LIMIT {
                cache.clear();
            }
            cache.insert(word.clone(), ok);
            ok
        });
        ok || (part.after_hyphen && self.noun_ed(&word))
    }

    /// The "-hearted" of "kind-hearted": a noun plus -ed that only exists
    /// after a hyphen ("good-natured", "silver-hilted").
    fn noun_ed(&self, word: &str) -> bool {
        let cut = word.len().saturating_sub(2);
        let Some(stem) = word
            .get(cut..)
            .filter(|tail| tail.eq_ignore_ascii_case("ed"))
            .map(|_| &word[..cut])
            .filter(|stem| stem.chars().count() >= 2)
        else {
            return false;
        };
        self.known(stem) || self.known(&format!("{stem}e"))
    }

    /// The uncached verdict for one word with straight apostrophes. Single
    /// letters always pass ("ö", a rune, the "n" of "rock 'n' roll").
    fn check(&self, word: &str) -> bool {
        if word.chars().count() < 2 || self.known(word) {
            return true;
        }
        if let Some((stem, _)) = split_ending(word)
            && self.check(stem)
        {
            return true;
        }
        // The bundled dictionary spells "café" and "naïve" without accents.
        let folded = fold_accents(word);
        folded != word && self.check(&folded)
    }

    /// Dictionary or added word. Hunspell's casing rules do the rest: a
    /// Capitalised word (sentence start, heading) passes if its lowercase
    /// form is listed, and ALL CAPS matches any casing ("PARIS", "DON'T",
    /// "IPHONE"). Lowercase never matches a proper noun ("english").
    fn known(&self, word: &str) -> bool {
        self.dict.check(word) || self.added.contains(&word.to_lowercase())
    }
}

impl Default for Speller {
    fn default() -> Self {
        Speller::new()
    }
}

/// Char range of the word under the cursor, or ending right at it (the word
/// just typed, or one followed by punctuation). The same units the checker
/// flags: hyphenated compounds give the part the cursor is in.
pub fn word_at(line: &str, char_idx: usize) -> Option<(usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let parts = parts(&chars);
    parts
        .iter()
        .find(|p| p.start <= char_idx && char_idx < p.end)
        .or_else(|| parts.iter().find(|p| p.end == char_idx))
        .map(|p| (p.start, p.end))
}

/// Words to accept from notebook entry titles: "Ashen Reach" -> "Ashen",
/// "Reach"; "Vael'thar" stays whole; "Kaelen's Blade" -> "Kaelen", "Blade".
/// Case-insensitive duplicates are dropped; the first spelling wins.
pub fn names_from_titles(titles: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for title in titles {
        let chars: Vec<char> = title.chars().collect();
        for part in parts(&chars) {
            let word = &chars[part.start..part.end];
            let key = word_key(&drop_possessive(&straighten(word)));
            if key.chars().count() < 2 || !seen.insert(key) {
                continue;
            }
            let written: String = word.iter().collect();
            out.push(drop_possessive(&written));
        }
    }
    out
}

/// The book's own word list: `<root>/dictionary.txt`, one word per line,
/// `#` starts a comment. A missing or unreadable file is an empty list.
pub fn book_words(root: &Path) -> Vec<String> {
    words_in(&root.join(BOOK_DICTIONARY))
}

fn parse_word_list(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let word = line.split('#').next().unwrap_or_default();
            let word = word.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
            (!word.is_empty()).then(|| word.to_string())
        })
        .collect()
}

/// Append a word to `<root>/dictionary.txt` unless it's already there (in any
/// capitalisation), creating the file with a comment header first. A
/// possessive is stored as its stem, which covers both forms.
pub fn add_book_word(root: &Path, word: &str) -> Result<()> {
    add_word_to(&root.join(BOOK_DICTIONARY), BOOK_DICTIONARY_HEADER, word)
}

/// The writer's own word list, accepted in every book:
/// `~/.config/grimoire/dictionary.txt`.
pub fn user_dictionary() -> PathBuf {
    crate::paths::home()
        .join(".config")
        .join("grimoire")
        .join(BOOK_DICTIONARY)
}

/// The words in a word list file; a missing or unreadable file is empty.
pub fn words_in(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .map(|text| parse_word_list(&text))
        .unwrap_or_default()
}

/// Append a word to the writer's own list (see [`user_dictionary`]), making
/// its folder if need be.
pub fn add_user_word(path: &Path, word: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    }
    add_word_to(path, USER_DICTIONARY_HEADER, word)
}

/// A suggestion in the shape of the word it replaces: `Teh` → `The`,
/// `TEH` → `THE`, `teh` → `the`. A suggestion that is itself capitalised (a
/// proper noun) keeps its capital.
pub fn match_case(original: &str, suggestion: &str) -> String {
    let letters: Vec<char> = original.chars().filter(|c| c.is_alphabetic()).collect();
    let all_caps = letters.len() > 1 && letters.iter().all(|c| c.is_uppercase());
    if all_caps {
        return suggestion.to_uppercase();
    }
    let first_up = original.chars().next().is_some_and(|c| c.is_uppercase());
    let mut chars = suggestion.chars();
    match chars.next() {
        Some(c) if first_up => c.to_uppercase().chain(chars).collect(),
        _ => suggestion.to_string(),
    }
}

fn add_word_to(path: &Path, header: &str, word: &str) -> Result<()> {
    let word = drop_possessive(word.trim());
    if word.is_empty() || word.contains(['\n', '\r', '#']) {
        bail!("{word:?} can't go in the book's word list");
    }
    let existing = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let key = word_key(&word);
    if parse_word_list(&existing)
        .iter()
        .any(|w| word_key(w) == key)
    {
        return Ok(());
    }

    let mut text = String::new();
    if existing.trim().is_empty() {
        text.push_str(header);
    } else if !existing.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&word);
    text.push('\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    file.write_all(text.as_bytes())
        .with_context(|| format!("writing {}", path.display()))
}

/// One checkable word: a char range, and whether a hyphen joins it to the
/// previous or next word of a compound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Part {
    start: usize,
    end: usize,
    before_hyphen: bool,
    after_hyphen: bool,
}

/// Split prose into the words worth checking.
///
/// Words are runs of letters with interior apostrophes; hyphens split a
/// compound into parts. Everything else separates words (spaces, em and en
/// dashes, quotes, brackets, `#`). Quotes, apostrophes and Markdown `*`/`_`
/// at a word's edges are not part of it. Parts containing digits or interior
/// `*`/`_` are skipped, as are code spans, web addresses and emails.
fn parts(chars: &[char]) -> Vec<Part> {
    let skip = skip_mask(chars);
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if skip[i] || !is_run_char(chars[i]) {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < chars.len() && !skip[i] && is_run_char(chars[i]) {
            i += 1;
        }
        let (start, end) = trim_to_word(chars, run_start, i);

        let mut seg_start = start;
        for j in start..=end {
            if j < end && !is_hyphen(chars[j]) {
                continue;
            }
            let (s, e) = trim_to_word(chars, seg_start, j);
            let checkable = s < e
                && chars[s..e]
                    .iter()
                    .all(|&c| is_letter(c) || is_apostrophe(c));
            if checkable {
                out.push(Part {
                    start: s,
                    end: e,
                    before_hyphen: chars.get(e).is_some_and(|&c| is_hyphen(c)),
                    after_hyphen: s > 0 && is_hyphen(chars[s - 1]),
                });
            }
            seg_start = j + 1;
        }
    }
    out
}

fn is_letter(c: char) -> bool {
    c.is_alphabetic() || is_combining_mark(c)
}

fn is_combining_mark(c: char) -> bool {
    matches!(c, '\u{0300}'..='\u{036F}' | '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}' | '\u{20D0}'..='\u{20FF}' | '\u{FE20}'..='\u{FE2F}')
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '’' | '‘' | 'ʼ')
}

fn is_hyphen(c: char) -> bool {
    matches!(c, '-' | '\u{2010}' | '\u{2011}')
}

/// Characters that can belong to a whitespace-free word-ish run. Digits and
/// markup are included so a run like `3rd` or `snake_case` can be skipped
/// whole instead of yielding "rd" or "snake".
fn is_run_char(c: char) -> bool {
    is_letter(c) || c.is_numeric() || is_apostrophe(c) || is_hyphen(c) || c == '_' || c == '*'
}

/// Shrink `[start, end)` to begin and end on a letter or digit.
fn trim_to_word(chars: &[char], mut start: usize, mut end: usize) -> (usize, usize) {
    let core = |c: char| is_letter(c) || c.is_numeric();
    while start < end && !core(chars[start]) {
        start += 1;
    }
    while end > start && !core(chars[end - 1]) {
        end -= 1;
    }
    (start, end)
}

/// Chars that are not prose: Markdown code spans, URLs, emails, domains and
/// file names.
fn skip_mask(chars: &[char]) -> Vec<bool> {
    let mut skip = vec![false; chars.len()];
    let n = chars.len();
    let at = |i: usize, s: &str| {
        s.chars()
            .enumerate()
            .all(|(k, c)| chars.get(i + k) == Some(&c))
    };
    let url_char =
        |c: char| !c.is_whitespace() && !matches!(c, '—' | '–' | '"' | '“' | '”' | '<' | '>');

    // `code spans`, delimited by equal-length backtick runs.
    let mut i = 0;
    while i < n {
        if chars[i] != '`' {
            i += 1;
            continue;
        }
        let open = i;
        while i < n && chars[i] == '`' {
            i += 1;
        }
        let ticks = i - open;
        let mut j = i;
        let mut close = None;
        while j < n {
            if chars[j] == '`' {
                let run = j;
                while j < n && chars[j] == '`' {
                    j += 1;
                }
                if j - run == ticks {
                    close = Some(j);
                    break;
                }
            } else {
                j += 1;
            }
        }
        if let Some(close) = close {
            skip[open..close].fill(true);
            i = close;
        }
    }

    for i in 0..n {
        // scheme://anything and www.anything
        let scheme = at(i, "://");
        let www = at(i, "www.") && (i == 0 || !chars[i - 1].is_alphanumeric());
        if scheme || www {
            let mut start = i;
            if scheme {
                while start > 0
                    && (chars[start - 1].is_ascii_alphanumeric()
                        || matches!(chars[start - 1], '+' | '.' | '-'))
                {
                    start -= 1;
                }
            }
            let mut end = i;
            while end < n && url_char(chars[end]) {
                end += 1;
            }
            skip[start..end].fill(true);
        }
        // name@host.tld
        if chars[i] == '@' {
            let local = |c: char| c.is_alphanumeric() || matches!(c, '.' | '_' | '%' | '+' | '-');
            let host = |c: char| c.is_alphanumeric() || matches!(c, '.' | '-');
            let mut start = i;
            while start > 0 && local(chars[start - 1]) {
                start -= 1;
            }
            let mut end = i + 1;
            while end < n && host(chars[end]) {
                end += 1;
            }
            if start < i && chars[i + 1..end].contains(&'.') {
                skip[start..end].fill(true);
            }
        }
    }

    // joshking.ai, chapter-01.md
    let dotted = |c: char| c.is_alphanumeric() || matches!(c, '.' | '-' | '_');
    let mut i = 0;
    while i < n {
        if !dotted(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && dotted(chars[i]) {
            i += 1;
        }
        let token: String = chars[start..i].iter().collect();
        let token = token.trim_end_matches('.');
        if let Some((head, tail)) = token.rsplit_once('.')
            && !head.is_empty()
            && DOTTED_ENDINGS.contains(&tail.to_lowercase().as_str())
        {
            skip[start..i].fill(true);
        }
    }
    skip
}

/// Curly and modifier apostrophes become `'`, as the dictionary spells them.
fn straighten(chars: &[char]) -> String {
    chars
        .iter()
        .map(|&c| if is_apostrophe(c) { '\'' } else { c })
        .collect()
}

/// The stem and ending of a contraction or possessive, if the word has one.
fn split_ending(word: &str) -> Option<(&str, &'static str)> {
    ENDINGS.iter().find_map(|&ending| {
        let cut = word.len().checked_sub(ending.len())?;
        let tail = word.get(cut..)?;
        (cut > 0 && tail.eq_ignore_ascii_case(ending)).then(|| (&word[..cut], ending))
    })
}

/// How added words compare: any capitalisation, any apostrophe.
fn word_key(word: &str) -> String {
    straighten(&word.chars().collect::<Vec<_>>()).to_lowercase()
}

/// "Kaelen's" / "Kaelen’s" -> "Kaelen"; anything else unchanged.
fn drop_possessive(word: &str) -> String {
    let key = word_key(word);
    if key.len() > 2 && key.ends_with("'s") {
        let keep = word.chars().count() - 2;
        return word.chars().take(keep).collect();
    }
    word.to_string()
}

/// Strip common Latin diacritics: "café" -> "cafe", "Æsir" -> "AEsir".
fn fold_accents(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        let base = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
            'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ă' | 'Ą' => "A",
            'ç' | 'ć' | 'č' => "c",
            'Ç' | 'Ć' | 'Č' => "C",
            'ď' => "d",
            'Ď' => "D",
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => "e",
            'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ė' | 'Ę' | 'Ě' => "E",
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' => "i",
            'Ì' | 'Í' | 'Î' | 'Ï' | 'Ī' | 'Į' => "I",
            'ñ' | 'ń' | 'ň' => "n",
            'Ñ' | 'Ń' | 'Ň' => "N",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => "o",
            'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ō' | 'Ő' => "O",
            'ř' => "r",
            'Ř' => "R",
            'ś' | 'š' => "s",
            'Ś' | 'Š' => "S",
            'ť' => "t",
            'Ť' => "T",
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => "u",
            'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ū' | 'Ů' | 'Ű' => "U",
            'ý' | 'ÿ' => "y",
            'Ý' | 'Ÿ' => "Y",
            'ź' | 'ż' | 'ž' => "z",
            'Ź' | 'Ż' | 'Ž' => "Z",
            'æ' => "ae",
            'Æ' => "AE",
            'œ' => "oe",
            'Œ' => "OE",
            c if is_combining_mark(c) => "",
            _ => {
                out.push(c);
                continue;
            }
        };
        out.push_str(base);
    }
    out
}

/// "II", "XIV", "MMXXVI" — chapter and part numbers in headings.
fn is_roman_numeral(word: &str) -> bool {
    const TABLE: [(u32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let value = |c: char| match c {
        'I' => Some(1),
        'V' => Some(5),
        'X' => Some(10),
        'L' => Some(50),
        'C' => Some(100),
        'D' => Some(500),
        'M' => Some(1000),
        _ => None,
    };
    let Some(digits) = word.chars().map(value).collect::<Option<Vec<i64>>>() else {
        return false;
    };
    let total: i64 = digits
        .iter()
        .enumerate()
        .map(|(i, &d)| {
            if digits.get(i + 1).is_some_and(|&next| next > d) {
                -d
            } else {
                d
            }
        })
        .sum();
    if !(1..4000).contains(&total) {
        return false;
    }
    // Only the canonical spelling counts, so "IIII" and "VX" are words.
    let mut rest = total as u32;
    let mut canonical = String::new();
    for (n, s) in TABLE {
        while rest >= n {
            canonical.push_str(s);
            rest -= n;
        }
    }
    canonical == word
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use std::time::Instant;

    #[test]
    fn a_suggestion_takes_the_shape_of_the_word_it_replaces() {
        assert_eq!(match_case("Teh", "the"), "The");
        assert_eq!(match_case("TEH", "the"), "THE");
        assert_eq!(match_case("teh", "the"), "the");
        // A proper noun keeps its capital even for a lowercase slip.
        assert_eq!(match_case("englsh", "English"), "English");
        // One capital letter isn't shouting.
        assert_eq!(match_case("I", "a"), "A");
    }

    #[test]
    fn the_writers_own_list_is_made_where_it_goes_and_read_back() {
        let dir = std::env::temp_dir().join(format!("grimoire-userdict-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("config").join("dictionary.txt");
        assert!(words_in(&path).is_empty(), "missing is empty");
        add_user_word(&path, "Wyvernkin").unwrap();
        add_user_word(&path, "wyvernkin").unwrap(); // already there, any case
        assert_eq!(words_in(&path), vec!["Wyvernkin".to_string()]);
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .starts_with("# Words the spellchecker accepts in every book")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// One shared speller with a couple of the book's names added.
    fn speller() -> &'static Speller {
        static SPELLER: OnceLock<Speller> = OnceLock::new();
        SPELLER.get_or_init(|| {
            let mut s = Speller::new();
            s.add_words(["Vael'thar".to_string(), "Kaelen".to_string()]);
            s
        })
    }

    /// The misspelled words of a line, as text.
    fn flagged(line: &str) -> Vec<String> {
        let chars: Vec<char> = line.chars().collect();
        speller()
            .misspellings(line)
            .into_iter()
            .map(|(s, e)| chars[s..e].iter().collect())
            .collect()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("grimoire-spell-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn common_misspellings_are_flagged() {
        assert_eq!(
            flagged("I definately did not recieve teh letter."),
            ["definately", "recieve", "teh"]
        );
        for word in ["recieve", "teh", "definately", "seperate", "wierd"] {
            assert!(!speller().is_correct(word), "{word}");
        }
    }

    #[test]
    fn correct_prose_passes() {
        let prose = [
            "Don't go; it's late, and we'd rather you didn't.",
            "The wizard's staff and the witches' brooms lay by the door.",
            "A well-known, hard-won, twenty-first-century mother-in-law.",
            "“Stay,” she whispered—“the road isn’t safe”—and the night–long rain fell.",
            "‘I can’t,’ he said. 'You shouldn't either.'",
            "They'll say you're mad, but I've heard what they'd do; I'm sure.",
            "*Nothing* moved in the __silent__ halls. # Chapter One",
            "RUN, he shouted. The Tower stood silent.",
            "Chapter XLVII: Part II, Book MCMXC. She carved ᚱ into the ö-rune.",
            "It cost 3rd-rate coin, 40 marks, at 5pm on 2026-09-16.",
            "Write to vaeloria.quill@example.com or see https://grimoire.joshking.ai/docs?x=1 and www.qwzx.org today.",
            "Use `fn main() { recieve(); }` in the joshking.ai file named chaptr-01.md.",
            "A naïve café owner met a pre-war non-magical co-worker.",
            "The grey-bearded mage axed the lich amongst kind-hearted, good-natured dwarves.",
            "Heavy-browed wyverns scried the elven sigils; the grimoire’s mana was spent.",
            "e.g. the U.S. army, Mr. and Mrs. Smith, Dr. Who.",
            "...and then---nothing. Rock 'n' roll, o'clock, ne'er-do-well.",
        ];
        for line in prose {
            assert_eq!(flagged(line), Vec::<String>::new(), "{line}");
        }
    }

    #[test]
    fn book_names_pass_once_added() {
        let line = "Vael'thar rode past Kaelen's tower; Vael’thar’s ravens followed Kaelen.";
        let mut fresh = Speller::new();
        assert!(!fresh.is_correct("Vael'thar"));
        assert!(!fresh.is_correct("Kaelen's"));
        assert_eq!(fresh.misspellings(line).len(), 4);

        fresh.add_words(["Vael'thar".to_string(), "Kaelen's".to_string()]);
        assert_eq!(fresh.misspellings(line), Vec::<(usize, usize)>::new());
        // Case-insensitive, both apostrophe styles, possessives and ALL CAPS.
        for word in ["vael'thar", "VAEL’THAR", "Kaelen", "KAELEN'S", "kaelen’s"] {
            assert!(fresh.is_correct(word), "{word}");
        }
        // Adding is idempotent and multi-word strings add each word.
        fresh.add_words(["Kaelen".to_string(), "Ashen Reach of Mournvale".to_string()]);
        assert!(fresh.is_correct("Mournvale"));
        assert!(!fresh.is_correct("Kaelan"));
    }

    #[test]
    fn capitalisation_rules() {
        let s = speller();
        assert!(s.is_correct("Receive")); // sentence start
        assert!(s.is_correct("RECEIVE")); // shouting
        assert!(s.is_correct("PARIS"));
        assert!(s.is_correct("NASA"));
        assert!(s.is_correct("DON'T"));
        assert!(!s.is_correct("RECIEVE"));
        assert!(!s.is_correct("Recieve"));
        assert!(!s.is_correct("english")); // proper nouns keep their capital
        assert!(!s.is_correct("rEceive"));
    }

    #[test]
    fn suggestions() {
        let s = speller();
        let got = s.suggest("recieve", 5);
        assert!(got.contains(&"receive".to_string()), "{got:?}");
        assert!(got.len() <= 5);
        assert!(s.suggest("teh", 5).contains(&"the".to_string()));
        assert!(
            s.suggest("definately", 5)
                .contains(&"definitely".to_string())
        );
        assert!(s.suggest("Recieve", 3).contains(&"Receive".to_string()));
        assert!(s.suggest("recieve", 0).is_empty());
        // Possessive endings and curly apostrophes survive.
        assert!(s.suggest("wizzard’s", 5).contains(&"wizard’s".to_string()));
        // Added names are suggested for near misses.
        assert!(s.suggest("Kaelan", 5).contains(&"Kaelen".to_string()));
        // Splits that strand a letter are dropped; real two-word fixes stay.
        let got = s.suggest("Kaelenn", 10);
        assert!(got.contains(&"Kaelen".to_string()), "{got:?}");
        assert!(!got.iter().any(|w| w.contains(' ')), "{got:?}");
        assert!(s.suggest("alot", 5).contains(&"a lot".to_string()));
    }

    #[test]
    fn ranges_are_chars_not_bytes() {
        //          0123456789012345678901234567890123
        let line = "Señor—café “recieve” naïve teh ✨ yes";
        let ranges = speller().misspellings(line);
        assert_eq!(ranges, [(12, 19), (27, 30)]);
        let chars: Vec<char> = line.chars().collect();
        assert_eq!(chars[12..19].iter().collect::<String>(), "recieve");
        assert_eq!(chars[27..30].iter().collect::<String>(), "teh");
        // Surrounding punctuation and Markdown stay outside the range.
        assert_eq!(
            speller().misspellings("**Wrold**, “Teh”"),
            [(2, 7), (12, 15)]
        );
        // Hyphenated compounds flag only the bad part.
        assert_eq!(speller().misspellings("well-recieved"), [(5, 13)]);
        // Noun+ed and bare prefixes only pass inside a compound.
        assert_eq!(
            speller().misspellings("hearted kind-hearted pre pre-war"),
            [(0, 7), (21, 24)]
        );
    }

    #[test]
    fn word_at_cursor() {
        //          01234567890123456789012345
        let line = "“Hello,” Kaelen’s well-known world";
        assert_eq!(word_at(line, 1), Some((1, 6))); // start
        assert_eq!(word_at(line, 3), Some((1, 6))); // middle
        assert_eq!(word_at(line, 6), Some((1, 6))); // just after, on the comma
        assert_eq!(word_at(line, 7), None); // closing quote
        assert_eq!(word_at(line, 8), None); // space
        assert_eq!(word_at(line, 0), None); // opening quote
        assert_eq!(word_at(line, 9), Some((9, 17))); // possessive is one word
        assert_eq!(word_at(line, 16), Some((9, 17)));
        assert_eq!(word_at(line, 21), Some((18, 22))); // compounds give the part
        assert_eq!(word_at(line, 22), Some((18, 22))); // on the hyphen
        assert_eq!(word_at(line, 23), Some((23, 28)));
        assert_eq!(word_at(line, 34), Some((29, 34))); // end of line
        assert_eq!(word_at(line, 99), None);
        assert_eq!(word_at("", 0), None);
    }

    #[test]
    fn names_from_notebook_titles() {
        let titles = [
            "Ashen Reach".to_string(),
            "Vael'thar".to_string(),
            "Kaelen’s Blade".to_string(),
            "the ashen gate".to_string(),
            "Order of the 7th Flame".to_string(),
        ];
        assert_eq!(
            names_from_titles(&titles),
            [
                "Ashen",
                "Reach",
                "Vael'thar",
                "Kaelen",
                "Blade",
                "the",
                "gate",
                "Order",
                "of",
                "Flame"
            ]
        );
    }

    #[test]
    fn book_word_list_round_trip() {
        let root = temp_dir("words");
        assert!(book_words(&root).is_empty());

        add_book_word(&root, "Vael'thar").unwrap();
        add_book_word(&root, "Kaelen's").unwrap();
        add_book_word(&root, "  vael’thar ").unwrap();
        add_book_word(&root, "KAELEN").unwrap();
        add_book_word(&root, "Mournvale").unwrap();
        assert_eq!(book_words(&root), ["Vael'thar", "Kaelen", "Mournvale"]);

        let text = fs::read_to_string(root.join("dictionary.txt")).unwrap();
        assert!(text.starts_with("# "));
        assert_eq!(text.lines().filter(|l| l.starts_with('#')).count(), 1);

        // Hand edits: comments, blank lines, no trailing newline.
        fs::write(
            root.join("dictionary.txt"),
            "# mine\n\nAshen  # the reach\nGloam",
        )
        .unwrap();
        add_book_word(&root, "Thessaly").unwrap();
        add_book_word(&root, "gloam").unwrap();
        assert_eq!(book_words(&root), ["Ashen", "Gloam", "Thessaly"]);

        assert!(add_book_word(&root, "  ").is_err());
        assert!(add_book_word(&root, "two\nlines").is_err());

        let mut s = Speller::new();
        s.add_words(book_words(&root));
        assert!(s.is_correct("GLOAM"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn roman_numerals() {
        for n in ["II", "IV", "XIV", "XL", "MMXXVI"] {
            assert!(is_roman_numeral(n), "{n}");
        }
        for n in ["IIII", "VX", "IC", "ii", "Mix", ""] {
            assert!(!is_roman_numeral(n), "{n}");
        }
    }

    #[test]
    fn speller_can_move_between_threads() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<Speller>();
    }

    /// `cargo test --release spell::tests::timings -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn timings() {
        let t = Instant::now();
        let s = Speller::new();
        println!("Speller::new: {:?}", t.elapsed());

        let paragraph = "The rain had not stopped for three days when Kaelen finally reached the \
            gates of Ashen Reach. He'd expected guards—there were none—only the wind, the \
            crows, and a single lantern swinging above the well-worn road. “Vael'thar?” he \
            called, and the name came back to him off the wet stones, hollow and strange. \
            Nobody answered. He tightened his cloak, checked the knife at his belt, and \
            stepped through the arch into the city he'd sworn he would never recieve again.";
        let words = paragraph.split_whitespace().count();
        let t = Instant::now();
        let bad = s.misspellings(paragraph);
        println!(
            "misspellings, cold cache: {:?} ({words} words, {} flagged)",
            t.elapsed(),
            bad.len()
        );
        let rounds = 1000;
        let t = Instant::now();
        for _ in 0..rounds {
            std::hint::black_box(s.misspellings(paragraph));
        }
        println!(
            "misspellings, warm cache: {:?} per paragraph",
            t.elapsed() / rounds
        );
        let t = Instant::now();
        let sugg = s.suggest("recieve", 5);
        println!("suggest(recieve): {:?} {sugg:?}", t.elapsed());
        let t = Instant::now();
        let sugg = s.suggest("Kaelenn", 5);
        println!("suggest(Kaelenn): {:?} {sugg:?}", t.elapsed());
    }
}
