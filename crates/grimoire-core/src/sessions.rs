//! Session history: every writing session saved as a git commit whose message
//! reads like a line in a diary — "Tuesday evening · Act Two · 1,240 words" —
//! and pushed off-site when the book has a remote.
//!
//! Git is driven through its command-line tool rather than a library. That
//! keeps the binary lean, and it keeps the promise that the book is plain
//! files: the history is an ordinary repository any git client can open.
//!
//! Nothing here may leave git waiting for an answer nobody can give. Every
//! call runs with no terminal prompt, no askpass, no editor and no stdin, and
//! every call has a deadline after which git (and anything it started, such as
//! ssh) is killed. When git isn't installed, everything reports that plainly
//! instead of failing somewhere deeper.

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Local, NaiveDateTime, Timelike};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

/// How long a push may take before we give up on the remote.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(20);
/// How long anything that stays on this machine may take. Generous — a hook
/// or a slow disk shouldn't fail a save — but still finite.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(30);

/// Who a session is recorded as when git has no name or email configured.
const FALLBACK_NAME: &str = "Grimoire";
const FALLBACK_EMAIL: &str = "grimoire@localhost";

/// The message on the commit that starts a book's history.
const FIRST_LABEL: &str = "Session history begins";

/// `.grimoire/` is per-machine state and the trash. All of it stays out of the
/// history except the note of where the writer left off.
const IGNORE_STATE: &str = ".grimoire/*";
const KEEP_RESUME: &str = "!.grimoire/resume.md";
/// Files made by Export, which can always be made again.
const IGNORE_EXPORTS: &str = "exports/";

/// Inherited variables that would point git at some other repository than the
/// book — set, for instance, when Grimoire is launched from inside a git hook.
const FOREIGN_REPO_ENV: [&str; 8] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Whether the `git` command can be run at all. Checked once and remembered.
pub fn git_available() -> bool {
    #[cfg(test)]
    if TEST_GIT_PROGRAM.with(|p| p.get().is_some()) {
        return probe_git();
    }
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(probe_git)
}

/// Whether session history is on for this book: the book folder is itself the
/// root of a git work tree. A book that merely sits somewhere inside another
/// repository doesn't count — its sessions would land in someone else's history.
pub fn is_enabled(root: &Path) -> bool {
    if !root.join(".git").exists() || !git_available() {
        return false;
    }
    let Ok(Some(top)) = git(root, ["rev-parse", "--show-toplevel"]).lookup() else {
        return false;
    };
    match (fs::canonicalize(root), fs::canonicalize(&top)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Turn session history on: make the book a repository if it isn't one, keep
/// `.grimoire/` out of it except `resume.md`, and save everything already
/// there as the first session. Safe to call again.
pub fn enable(root: &Path) -> Result<()> {
    require_git()?;
    if !root.is_dir() {
        bail!("{} isn't a folder", root.display());
    }
    if !is_enabled(root) {
        let mut args: Vec<String> = Vec::new();
        // Name the first branch `main` unless the writer has chosen otherwise.
        if config(root, "init.defaultBranch")?.is_none() {
            args.extend(["-c".into(), "init.defaultBranch=main".into()]);
        }
        args.extend(["init".into(), "-q".into()]);
        git(root, args).stdout()?;
        if !is_enabled(root) {
            bail!("git init didn't make {} a repository", root.display());
        }
    }
    ensure_gitignore(root)?;
    stage_all(root, None)?;
    if staged_tally(root, None)?.is_some() {
        commit(root, FIRST_LABEL)?;
    }
    Ok(())
}

/// The label the next session would be saved under, or `None` when nothing has
/// changed since the last one. Looks without touching what git has staged.
pub fn pending_label(root: &Path, now: DateTime<Local>) -> Result<Option<String>> {
    require_repo(root)?;
    let index = ScratchIndex::copy_of(root)?;
    stage_all(root, Some(index.path()))?;
    let tally = staged_tally(root, Some(index.path()))?;
    Ok(tally.map(|t| label(now.naive_local(), &t, &part_noun(root))))
}

/// Save the session: stage everything `.gitignore` allows, commit it under the
/// session's label, and return the short hash. `None` when there was nothing
/// to save.
pub fn commit_session(root: &Path, now: DateTime<Local>) -> Result<Option<String>> {
    require_repo(root)?;
    stage_all(root, None)?;
    let Some(tally) = staged_tally(root, None)? else {
        return Ok(None);
    };
    commit(root, &label(now.naive_local(), &tally, &part_noun(root)))?;
    Ok(Some(git(root, ["rev-parse", "--short", "HEAD"]).text()?))
}

/// What happened when we tried to send the history off-site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    /// The book has no remote to push to. Not an error — history stays local.
    NoRemote,
    Pushed,
    /// Why it didn't go, in git's words (one line).
    Failed(String),
}

/// Push the current branch, setting its upstream the first time.
pub fn push(root: &Path) -> PushOutcome {
    push_within(root, NETWORK_TIMEOUT)
}

/// How many saved sessions haven't reached the remote yet, as of the last push
/// or fetch. `None` when the branch has no upstream (or history is off).
pub fn unpushed(root: &Path) -> Option<usize> {
    if !is_enabled(root) {
        return None;
    }
    git(root, ["rev-list", "--count", "@{upstream}..HEAD"])
        .lookup()
        .ok()
        .flatten()
        .and_then(|n| n.parse().ok())
}

/// Whether the book has anywhere to back sessions up to.
pub fn has_remote(root: &Path) -> bool {
    is_enabled(root) && git(root, ["remote"]).text().is_ok_and(|s| !s.is_empty())
}

/// Whether anything a session is about — scenes, notes, the book's settings —
/// has changed since the last one, as opposed to only resume.md.
pub fn has_writing(root: &Path) -> bool {
    pending_label(root, Local::now())
        .ok()
        .flatten()
        .is_some_and(|l| l.contains(" · "))
}

/// One saved session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub hash: String,
    pub when: DateTime<Local>,
    pub label: String,
}

/// The most recent `limit` sessions, newest first.
pub fn sessions(root: &Path, limit: usize) -> Result<Vec<Session>> {
    require_repo(root)?;
    if limit == 0 || head_commit(root)?.is_none() {
        return Ok(Vec::new());
    }
    let raw = git(
        root,
        [
            "log".to_string(),
            "-z".into(),
            "--no-color".into(),
            "--no-show-signature".into(),
            "--format=%h%x1f%at%x1f%s".into(),
            format!("--max-count={limit}"),
            "HEAD".into(),
            "--".into(),
        ],
    )
    .stdout()?;
    Ok(parse_log(&raw))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed { from: PathBuf },
}

/// A scene or note that a session touched. Word counts leave out frontmatter;
/// an added file has no words before, a deleted one none after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub kind: ChangeKind,
    pub words_before: usize,
    pub words_after: usize,
}

/// The scenes and notes a session changed, compared with the session before
/// it (the first session is compared with an empty book).
pub fn changes(root: &Path, hash: &str) -> Result<Vec<Change>> {
    require_repo(root)?;
    let commit = resolve_commit(root, hash)?;
    let base = match git(
        root,
        ["rev-parse", "--verify", "-q", &format!("{commit}^1")],
    )
    .lookup()?
    {
        Some(parent) => parent,
        None => empty_tree(root)?,
    };
    let raw = git(
        root,
        [
            "diff-tree",
            "-r",
            "-M",
            "-z",
            "--name-status",
            &base,
            &commit,
        ],
    )
    .stdout()?;
    let entries: Vec<Entry> = parse_name_status(&raw)
        .into_iter()
        .filter(|e| is_writing(&e.path) || e.from.as_deref().is_some_and(is_writing))
        .collect();

    let mut specs = Vec::new();
    for e in &entries {
        if e.status != Status::Added {
            specs.push(format!("{base}:{}", e.old_path()));
        }
        if e.status != Status::Deleted {
            specs.push(format!("{commit}:{}", e.path));
        }
    }
    let mut blobs = cat_blobs(root, &specs, None)?.into_iter();
    let mut words = || {
        blobs
            .next()
            .flatten()
            .map_or(0, |b| body_words(&String::from_utf8_lossy(&b)))
    };

    let mut out = Vec::with_capacity(entries.len());
    for e in entries {
        let words_before = if e.status == Status::Added {
            0
        } else {
            words()
        };
        let words_after = if e.status == Status::Deleted {
            0
        } else {
            words()
        };
        let kind = match e.status {
            Status::Added => ChangeKind::Added,
            Status::Deleted => ChangeKind::Deleted,
            Status::Modified => ChangeKind::Modified,
            Status::Renamed => ChangeKind::Renamed {
                from: native_path(e.from.as_deref().unwrap_or_default()),
            },
        };
        out.push(Change {
            path: native_path(&e.path),
            kind,
            words_before,
            words_after,
        });
    }
    Ok(out)
}

/// A file as it stood in that session, or `None` if it didn't exist then.
/// `rel` is relative to the book folder (an absolute path inside it works too).
pub fn file_at(root: &Path, hash: &str, rel: &Path) -> Result<Option<String>> {
    require_repo(root)?;
    let commit = resolve_commit(root, hash)?;
    let rel = rel.strip_prefix(root).unwrap_or(rel);
    let spec = format!("{commit}:{}", git_path(rel)?);
    let blob = cat_blobs(root, &[spec], None)?.into_iter().next().flatten();
    Ok(blob.map(|b| String::from_utf8_lossy(&b).into_owned()))
}

// ---------------------------------------------------------------------------
// Pushing
// ---------------------------------------------------------------------------

fn push_within(root: &Path, timeout: Duration) -> PushOutcome {
    match try_push(root, timeout) {
        Ok(outcome) => outcome,
        Err(e) => PushOutcome::Failed(e.to_string()),
    }
}

fn try_push(root: &Path, timeout: Duration) -> Result<PushOutcome> {
    require_repo(root)?;
    let remotes: Vec<String> = git(root, ["remote"])
        .text()?
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if remotes.is_empty() {
        return Ok(PushOutcome::NoRemote);
    }
    let Some(branch) = git(root, ["symbolic-ref", "--quiet", "--short", "HEAD"]).lookup()? else {
        bail!("not on a branch, so there's nothing to push");
    };
    if head_commit(root)?.is_none() {
        bail!("no sessions saved yet");
    }

    let tracked_remote = config(root, &format!("branch.{branch}.remote"))?;
    let tracked_ref = config(root, &format!("branch.{branch}.merge"))?;
    let (remote, args): (String, Vec<String>) = match (tracked_remote, tracked_ref) {
        (Some(remote), Some(merge)) if remotes.contains(&remote) => {
            let args = vec!["push".into(), remote.clone(), format!("HEAD:{merge}")];
            (remote, args)
        }
        _ => {
            let remote = if remotes.iter().any(|r| r == "origin") {
                "origin".to_string()
            } else if let [only] = remotes.as_slice() {
                only.clone()
            } else {
                bail!(
                    "this book has several remotes and none is set for {branch}; \
                     push once with git push -u <remote> {branch}"
                );
            };
            let args = vec!["push".into(), "-u".into(), remote.clone(), branch.clone()];
            (remote, args)
        }
    };

    let mut cmd = git(root, args).timeout(timeout);
    // ssh asks about unknown hosts and key passphrases on the terminal, which
    // belongs to the editor. Batch mode makes it fail instead — unless the
    // writer has set up their own ssh command, which we leave alone.
    if std::env::var_os("GIT_SSH_COMMAND").is_none()
        && std::env::var_os("GIT_SSH").is_none()
        && config(root, "core.sshCommand")?.is_none()
    {
        cmd = cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    Ok(match cmd.run() {
        Ok(out) if out.success => PushOutcome::Pushed,
        Ok(out) => PushOutcome::Failed(summary(&out.stderr)),
        Err(Failure::TimedOut(t)) => {
            PushOutcome::Failed(format!("{remote} didn't answer within {} s", t.as_secs()))
        }
        Err(f) => PushOutcome::Failed(f.describe("push")),
    })
}

// ---------------------------------------------------------------------------
// Staging, committing, reading history
// ---------------------------------------------------------------------------

fn require_git() -> Result<()> {
    if git_available() {
        Ok(())
    } else {
        bail!("git isn't installed, so session history isn't available")
    }
}

fn require_repo(root: &Path) -> Result<()> {
    require_git()?;
    if !is_enabled(root) {
        bail!("session history isn't turned on for {}", root.display());
    }
    Ok(())
}

/// A config value, or `None` when it isn't set (or is empty).
fn config(root: &Path, key: &str) -> Result<Option<String>> {
    Ok(git(root, ["config", "--get", key])
        .lookup()?
        .filter(|v| !v.is_empty()))
}

fn head_commit(root: &Path) -> Result<Option<String>> {
    git(root, ["rev-parse", "--verify", "-q", "HEAD^{commit}"]).lookup()
}

/// The id of the empty tree in this repository's hash (SHA-1 or SHA-256).
fn empty_tree(root: &Path) -> Result<String> {
    git(root, ["hash-object", "-t", "tree", "--stdin"])
        .input(Vec::new())
        .text()
}

fn resolve_commit(root: &Path, hash: &str) -> Result<String> {
    let hash = hash.trim();
    if hash.is_empty() || hash.starts_with('-') || hash.contains(char::is_whitespace) {
        bail!("{hash:?} isn't a session");
    }
    git(
        root,
        ["rev-parse", "--verify", "-q", &format!("{hash}^{{commit}}")],
    )
    .lookup()?
    .ok_or_else(|| anyhow!("no session {hash} in this book's history"))
}

fn stage_all(root: &Path, index: Option<&Path>) -> Result<()> {
    git(root, ["add", "-A", "--", "."])
        .index(index)
        .stdout()
        .map(drop)
}

/// Commit what's staged. A name and email are supplied only for whichever of
/// the two git has no setting for, so a writer's own identity always wins.
fn commit(root: &Path, message: &str) -> Result<()> {
    let mut args: Vec<String> = Vec::new();
    if config(root, "user.name")?.is_none() {
        args.extend(["-c".into(), format!("user.name={FALLBACK_NAME}")]);
    }
    if config(root, "user.email")?.is_none() {
        args.extend(["-c".into(), format!("user.email={FALLBACK_EMAIL}")]);
    }
    args.extend(["commit".into(), "-q".into(), "-m".into(), message.into()]);
    git(root, args).stdout().map(drop)
}

/// A private copy of the index, so a label can be worked out by staging
/// everything without disturbing what the writer (or another tool) staged.
struct ScratchIndex(PathBuf);

impl ScratchIndex {
    fn copy_of(root: &Path) -> Result<Self> {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let path = std::env::temp_dir().join(format!("grimoire-index-{}-{n}", std::process::id()));
        let real = root.join(git(root, ["rev-parse", "--git-path", "index"]).text()?);
        let scratch = ScratchIndex(path);
        if real.is_file() {
            fs::copy(&real, &scratch.0).context("copying git's index")?;
        } else {
            let _ = fs::remove_file(&scratch.0);
        }
        Ok(scratch)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchIndex {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        let mut lock = self.0.clone().into_os_string();
        lock.push(".lock");
        let _ = fs::remove_file(lock);
    }
}

/// What the staged changes amount to, for the label. `None` if nothing is staged.
fn staged_tally(root: &Path, index: Option<&Path>) -> Result<Option<Tally>> {
    let head = head_commit(root)?;
    let base = match &head {
        Some(h) => h.clone(),
        None => empty_tree(root)?,
    };
    let raw = git(
        root,
        ["diff-index", "--cached", "-M", "-z", "--name-status", &base],
    )
    .index(index)
    .stdout()?;
    let entries = parse_name_status(&raw);
    if entries.is_empty() {
        return Ok(None);
    }

    let mut tally = Tally::default();
    // Which blobs to count, and whether each is the before or after side.
    let mut specs = Vec::new();
    let mut after = Vec::new();
    for e in &entries {
        let old = e.old_path();
        tally.note_place(old);
        tally.note_place(&e.path);
        if e.status != Status::Added && is_scene(old) {
            specs.push(format!("{base}:{old}"));
            after.push(false);
        }
        if e.status != Status::Deleted && is_scene(&e.path) {
            // Stage 0 of the index: what is about to be committed.
            specs.push(format!(":0:{}", e.path));
            after.push(true);
        }
    }
    for (blob, is_after) in cat_blobs(root, &specs, index)?.into_iter().zip(after) {
        let words = blob.map_or(0, |b| body_words(&String::from_utf8_lossy(&b)));
        if is_after {
            tally.words_after += words;
        } else {
            tally.words_before += words;
        }
    }
    Ok(Some(tally))
}

/// Fetch many blobs in one git process. `None` for anything that doesn't exist
/// or isn't a file.
fn cat_blobs(root: &Path, specs: &[String], index: Option<&Path>) -> Result<Vec<Option<Vec<u8>>>> {
    // One spec per line, so a path with a newline in it can't be asked for.
    let sendable: Vec<&String> = specs.iter().filter(|s| !s.contains(['\n', '\r'])).collect();
    let mut found = if sendable.is_empty() {
        Vec::new()
    } else {
        let mut input = Vec::new();
        for s in &sendable {
            input.extend_from_slice(s.as_bytes());
            input.push(b'\n');
        }
        let raw = git(root, ["cat-file", "--batch"])
            .index(index)
            .input(input)
            .stdout()?;
        parse_batch(&raw, sendable.len())
    }
    .into_iter();
    Ok(specs
        .iter()
        .map(|s| {
            if s.contains(['\n', '\r']) {
                None
            } else {
                found.next().flatten()
            }
        })
        .collect())
}

fn part_noun(root: &Path) -> String {
    fs::read_to_string(root.join("novel.toml"))
        .ok()
        .and_then(|s| toml::from_str::<toml::Table>(&s).ok())
        .and_then(|t| {
            t.get("part_label")?
                .as_str()
                .map(|s| s.trim().to_lowercase())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "part".into())
}

// ---------------------------------------------------------------------------
// .gitignore
// ---------------------------------------------------------------------------

fn ensure_gitignore(root: &Path) -> Result<()> {
    let path = root.join(".gitignore");
    let existing = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let wanted = gitignore_keeping_resume(&existing);
    if wanted != existing {
        fs::write(&path, wanted).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// `.gitignore` with `.grimoire/` ignored except `resume.md`. A bare
/// `.grimoire/` line (which git can't carve an exception out of) becomes the
/// pair in place; every other line is kept as it was.
fn gitignore_keeping_resume(existing: &str) -> String {
    let nl = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut lines: Vec<String> = Vec::new();
    let mut last_star: Option<usize> = None;
    let mut kept = false;
    for line in existing.lines() {
        match line.trim() {
            ".grimoire" | ".grimoire/" | "/.grimoire" | "/.grimoire/" => {
                last_star = Some(lines.len());
                lines.push(IGNORE_STATE.into());
                lines.push(KEEP_RESUME.into());
                kept = true;
            }
            ".grimoire/*" | "/.grimoire/*" => {
                last_star = Some(lines.len());
                kept = false;
                lines.push(line.into());
            }
            "!.grimoire/resume.md" | "!/.grimoire/resume.md" => {
                // Only an exception after the ignore line counts.
                kept |= last_star.is_some();
                lines.push(line.into());
            }
            _ => lines.push(line.into()),
        }
    }
    match last_star {
        None => {
            lines.push(IGNORE_STATE.into());
            lines.push(KEEP_RESUME.into());
        }
        Some(i) if !kept => lines.insert(i + 1, KEEP_RESUME.into()),
        Some(_) => {}
    }
    // Exports are made from the manuscript on demand; history keeps the
    // manuscript, not copies of it.
    if !lines
        .iter()
        .any(|l| matches!(l.trim(), "exports" | "exports/" | "/exports" | "/exports/"))
    {
        lines.push(IGNORE_EXPORTS.into());
    }
    let mut out = lines.join(nl);
    out.push_str(nl);
    out
}

// ---------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------

/// What a session touched, as far as its label cares.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Tally {
    /// Top-level names under `manuscript/` whose scenes changed, as on disk.
    acts: BTreeSet<String>,
    /// Anything under `notes/` changed.
    notes: bool,
    /// Words in the changed manuscript scenes, before and after.
    words_before: usize,
    words_after: usize,
}

impl Tally {
    fn note_place(&mut self, path: &str) {
        if let Some(rest) = path.strip_prefix("manuscript/") {
            if is_scene(path) {
                self.acts
                    .insert(rest.split('/').next().unwrap_or(rest).to_string());
            }
        } else if NOTEBOOK.iter().any(|d| path.starts_with(d)) {
            self.notes = true;
        }
    }
}

/// "Tuesday evening · Act Two · 1,240 words".
fn label(when: NaiveDateTime, tally: &Tally, part_noun: &str) -> String {
    let mut parts = vec![when_phrase(when)];
    if !tally.acts.is_empty() {
        let titles: Vec<String> = tally.acts.iter().map(|a| display_title(a)).collect();
        parts.push(match titles.as_slice() {
            [one] => one.clone(),
            [a, b] => format!("{a} and {b}"),
            many => format!("{} {part_noun}s", many.len()),
        });
        parts.push(match tally.words_after.cmp(&tally.words_before) {
            Ordering::Greater => words(tally.words_after - tally.words_before),
            Ordering::Less => format!(
                "revised, {} cut",
                words(tally.words_before - tally.words_after)
            ),
            Ordering::Equal => "revised".into(),
        });
    } else if tally.notes {
        parts.push("notes".into());
    }
    parts.join(" · ")
}

/// "Tuesday evening". The small hours still belong to the night before: a
/// session at 1 a.m. on Wednesday is "Tuesday night", the way a writer says it.
fn when_phrase(t: NaiveDateTime) -> String {
    let (day, part) = match t.hour() {
        5..=11 => (t.date(), "morning"),
        12..=13 => (t.date(), "lunchtime"),
        14..=16 => (t.date(), "afternoon"),
        17..=20 => (t.date(), "evening"),
        21..=23 => (t.date(), "night"),
        _ => (t.date().pred_opt().unwrap_or(t.date()), "night"),
    };
    format!("{} {part}", day.format("%A"))
}

/// "1 word", "1,240 words".
fn words(n: usize) -> String {
    format!("{} {}", thousands(n), if n == 1 { "word" } else { "words" })
}

fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Words in a scene, not counting its frontmatter: whitespace-separated
/// tokens, as the rest of the app counts them. Accepts CRLF fences too, so a
/// Windows checkout and the stored copy of the same scene count the same.
fn body_words(raw: &str) -> usize {
    let s = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let body = match s
        .strip_prefix("---\n")
        .or_else(|| s.strip_prefix("---\r\n"))
    {
        Some(rest) => {
            let mut offset = 0;
            let mut body = s; // an unclosed block isn't frontmatter
            for line in rest.split_inclusive('\n') {
                if line.trim_end_matches(['\n', '\r']) == "---" {
                    body = &rest[offset + line.len()..];
                    break;
                }
                offset += line.len();
            }
            body
        }
        None => s,
    };
    body.split_whitespace().count()
}

/// `01-Act-One` -> `Act One`. The same rule as the tree's titles in
/// `project.rs`, which a folder name has no frontmatter to override.
fn display_title(name: &str) -> String {
    let stem = Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let stripped = stem
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit() && *c != '-' && *c != '_' && *c != ' ')
        .map(|(i, _)| &stem[i..])
        .unwrap_or(&stem);
    let words: Vec<String> = stripped
        .split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect();
    rejoin_numbers(&words)
}

/// "Twenty Seven" -> "Twenty-Seven", as in `project.rs`.
fn rejoin_numbers(words: &[String]) -> String {
    const TENS: [&str; 8] = [
        "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    const UNITS: [&str; 9] = [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let w = words[i].to_lowercase();
        let next = words.get(i + 1).map(|n| n.to_lowercase());
        match next {
            Some(n) if TENS.contains(&w.as_str()) && UNITS.contains(&n.as_str()) => {
                out.push(format!("{}-{}", words[i], words[i + 1]));
                i += 2;
            }
            _ => {
                out.push(words[i].clone());
                i += 1;
            }
        }
    }
    out.join(" ")
}

// ---------------------------------------------------------------------------
// Paths and parsing git's output
// ---------------------------------------------------------------------------

/// The notebook's sections on disk, as git names them.
const NOTEBOOK: [&str; 4] = ["notes/", "characters/", "places/", "research/"];

/// A manuscript scene: a Markdown file under `manuscript/`.
fn is_scene(path: &str) -> bool {
    path.starts_with("manuscript/") && path.ends_with(".md")
}

/// A scene or a note.
fn is_writing(path: &str) -> bool {
    (path.starts_with("manuscript/") || NOTEBOOK.iter().any(|d| path.starts_with(d)))
        && path.ends_with(".md")
}

/// git's `a/b/c.md` as a path for this platform.
fn native_path(git: &str) -> PathBuf {
    git.split('/').collect()
}

/// A book-relative path as git names it: forward slashes on every platform.
fn git_path(rel: &Path) -> Result<String> {
    let mut parts = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(s) => parts.push(
                s.to_str()
                    .ok_or_else(|| anyhow!("{} isn't a name git can look up", rel.display()))?,
            ),
            Component::CurDir => {}
            _ => bail!("{} isn't a path inside the book", rel.display()),
        }
    }
    if parts.is_empty() {
        bail!("no file named");
    }
    Ok(parts.join("/"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    status: Status,
    path: String,
    from: Option<String>,
}

impl Entry {
    fn old_path(&self) -> &str {
        self.from.as_deref().unwrap_or(&self.path)
    }
}

/// `--name-status -z`: `M\0path\0`, `R087\0old\0new\0`.
fn parse_name_status(raw: &[u8]) -> Vec<Entry> {
    let mut fields = raw
        .split(|b| *b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned());
    let mut out = Vec::new();
    while let Some(code) = fields.next() {
        let Some(letter) = code.chars().next() else {
            continue;
        };
        let entry = match letter {
            'R' | 'C' => {
                let (Some(from), Some(path)) = (fields.next(), fields.next()) else {
                    break;
                };
                if letter == 'R' {
                    Entry {
                        status: Status::Renamed,
                        path,
                        from: Some(from),
                    }
                } else {
                    Entry {
                        status: Status::Added,
                        path,
                        from: None,
                    }
                }
            }
            _ => {
                let Some(path) = fields.next() else { break };
                let status = match letter {
                    'A' => Status::Added,
                    'D' => Status::Deleted,
                    _ => Status::Modified,
                };
                Entry {
                    status,
                    path,
                    from: None,
                }
            }
        };
        out.push(entry);
    }
    out
}

/// `cat-file --batch`: `<oid> <type> <size>\n<content>\n` per object, or
/// `<spec> missing\n`.
fn parse_batch(raw: &[u8], count: usize) -> Vec<Option<Vec<u8>>> {
    let mut out = Vec::with_capacity(count);
    let mut pos = 0;
    while out.len() < count && pos < raw.len() {
        let Some(nl) = raw[pos..].iter().position(|b| *b == b'\n') else {
            break;
        };
        let header = String::from_utf8_lossy(&raw[pos..pos + nl]).into_owned();
        pos += nl + 1;
        let mut fields = header.rsplitn(3, ' ');
        let size = fields.next().and_then(|s| s.parse::<usize>().ok());
        let kind = fields.next();
        match (size, kind) {
            (Some(size), Some(kind)) if pos + size <= raw.len() => {
                let body = raw[pos..pos + size].to_vec();
                pos += size + 1;
                out.push((kind == "blob").then_some(body));
            }
            _ => out.push(None),
        }
    }
    out.resize(count, None);
    out
}

/// `log -z --format=%h%x1f%at%x1f%s`.
fn parse_log(raw: &[u8]) -> Vec<Session> {
    raw.split(|b| *b == 0)
        .filter_map(|record| {
            let record = String::from_utf8_lossy(record);
            let mut f = record.trim_matches('\n').splitn(3, '\u{1f}');
            let hash = f.next()?.trim().to_string();
            let secs: i64 = f.next()?.trim().parse().ok()?;
            let label = f.next().unwrap_or_default().to_string();
            let when = DateTime::from_timestamp(secs, 0)?.with_timezone(&Local);
            (!hash.is_empty()).then_some(Session { hash, when, label })
        })
        .collect()
}

/// git's reason for failing, in one line.
fn summary(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if let Some(l) = lines
        .iter()
        .find(|l| l.contains("[rejected]") || l.contains("[remote rejected]"))
    {
        return (*l).to_string();
    }
    for prefix in ["fatal: ", "error: "] {
        if let Some(l) = lines.iter().find_map(|l| l.strip_prefix(prefix)) {
            return l.to_string();
        }
    }
    lines
        .last()
        .map_or_else(|| "git gave no reason".into(), |l| (*l).to_string())
}

// ---------------------------------------------------------------------------
// Running git
// ---------------------------------------------------------------------------

#[cfg(test)]
thread_local! {
    /// Lets a test pretend git isn't installed, without touching other tests.
    static TEST_GIT_PROGRAM: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

fn git_program() -> &'static str {
    #[cfg(test)]
    if let Some(p) = TEST_GIT_PROGRAM.with(|p| p.get()) {
        return p;
    }
    "git"
}

fn probe_git() -> bool {
    git(&std::env::temp_dir(), ["--version"])
        .timeout(Duration::from_secs(10))
        .run()
        .is_ok_and(|o| o.success)
}

/// One git invocation, run in the book folder.
struct Git {
    dir: PathBuf,
    args: Vec<OsString>,
    env: Vec<(&'static str, OsString)>,
    input: Option<Vec<u8>>,
    timeout: Duration,
}

struct Output {
    success: bool,
    stdout: Vec<u8>,
    stderr: String,
}

enum Failure {
    Missing,
    TimedOut(Duration),
    Io(io::Error),
}

impl Failure {
    fn describe(&self, what: &str) -> String {
        match self {
            Failure::Missing => "git isn't installed".into(),
            Failure::TimedOut(t) => format!("git {what} didn't finish within {} s", t.as_secs()),
            Failure::Io(e) => format!("couldn't run git {what}: {e}"),
        }
    }
}

fn git<I, S>(dir: &Path, args: I) -> Git
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    Git {
        dir: dir.to_path_buf(),
        args: args.into_iter().map(Into::into).collect(),
        env: Vec::new(),
        input: None,
        timeout: LOCAL_TIMEOUT,
    }
}

impl Git {
    fn env(mut self, key: &'static str, value: impl Into<OsString>) -> Self {
        self.env.push((key, value.into()));
        self
    }

    /// Use this index file instead of the repository's own.
    fn index(self, index: Option<&Path>) -> Self {
        match index {
            Some(path) => self.env("GIT_INDEX_FILE", path),
            None => self,
        }
    }

    fn input(mut self, data: Vec<u8>) -> Self {
        self.input = Some(data);
        self
    }

    fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The subcommand, for messages: `commit` in `git -c k=v commit -m …`.
    fn subcommand(&self) -> String {
        let mut args = self.args.iter();
        while let Some(a) = args.next() {
            if a == "-c" {
                args.next();
            } else {
                return a.to_string_lossy().into_owned();
            }
        }
        String::new()
    }

    /// Stdout of a command that has to succeed.
    fn stdout(self) -> Result<Vec<u8>> {
        let what = self.subcommand();
        let out = self.run().map_err(|f| anyhow!(f.describe(&what)))?;
        if !out.success {
            bail!("git {what} failed: {}", summary(&out.stderr));
        }
        Ok(out.stdout)
    }

    /// Trimmed stdout of a command that has to succeed.
    fn text(self) -> Result<String> {
        let out = self.stdout()?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    /// A question git answers by exiting non-zero for "no": trimmed stdout, or
    /// `None`. Only a git that couldn't run at all is an error.
    fn lookup(self) -> Result<Option<String>> {
        let what = self.subcommand();
        let out = self.run().map_err(|f| anyhow!(f.describe(&what)))?;
        Ok(out
            .success
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()))
    }

    fn run(self) -> Result<Output, Failure> {
        if !self.dir.is_dir() {
            // Spawning in a missing folder also reports NotFound, which would
            // otherwise read as "git isn't installed".
            return Err(Failure::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} doesn't exist", self.dir.display()),
            )));
        }
        let mut cmd = Command::new(git_program());
        cmd.args(&self.args)
            .current_dir(&self.dir)
            .stdin(if self.input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in FOREIGN_REPO_ENV {
            cmd.env_remove(key);
        }
        // Never ask. No terminal prompt, no askpass program, no credential
        // manager window, no editor; don't take optional locks that could trip
        // up another git tool the writer has open on the same book.
        cmd.env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "")
            .env("GCM_INTERACTIVE", "never")
            .env("GIT_EDITOR", ":")
            .env("GIT_SEQUENCE_EDITOR", ":")
            .env("GIT_OPTIONAL_LOCKS", "0");
        #[cfg(test)]
        tests::hermetic(&mut cmd);
        for (key, value) in &self.env {
            cmd.env(key, value);
        }
        // Its own process group: a timeout can kill git and whatever it
        // started (ssh, a credential helper) together, and the terminal the
        // editor owns won't hand them keystrokes.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }

        let mut child = cmd.spawn().map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Failure::Missing,
            _ => Failure::Io(e),
        })?;
        if let (Some(data), Some(mut stdin)) = (self.input, child.stdin.take()) {
            thread::spawn(move || {
                let _ = stdin.write_all(&data);
            });
        }
        let pid = child.id();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(child.wait_with_output());
        });
        match rx.recv_timeout(self.timeout) {
            Ok(Ok(out)) => Ok(Output {
                success: out.status.success(),
                stdout: out.stdout,
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            }),
            Ok(Err(e)) => Err(Failure::Io(e)),
            Err(RecvTimeoutError::Timeout) => {
                kill_tree(pid);
                // Give the waiter a moment to reap it; don't wait on it forever.
                let _ = rx.recv_timeout(Duration::from_secs(2));
                Err(Failure::TimedOut(self.timeout))
            }
            Err(RecvTimeoutError::Disconnected) => Err(Failure::Io(io::Error::other(
                "lost track of the git process",
            ))),
        }
    }
}

/// Kill a git process and everything it started.
fn kill_tree(pid: u32) {
    #[cfg(unix)]
    let mut cmd = {
        let mut c = Command::new("kill");
        c.args(["-s", "KILL", "--", &format!("-{pid}")]);
        c
    };
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("taskkill");
        c.args(["/F", "/T", "/PID", &pid.to_string()]);
        c
    };
    #[cfg(any(unix, windows))]
    let _ = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    #[cfg(not(any(unix, windows)))]
    let _ = pid;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, NaiveDate, TimeZone, Weekday};
    use std::io::Read;
    use std::net::TcpListener;
    use std::time::Instant;

    /// Tests never read the machine's own git config (identity, signing,
    /// hooks, default branch) — each repository is exactly what the test made.
    pub(super) fn hermetic(cmd: &mut Command) {
        static CONFIG: OnceLock<PathBuf> = OnceLock::new();
        let path = CONFIG.get_or_init(|| {
            let p = std::env::temp_dir().join("grimoire-sessions-test.gitconfig");
            let _ = fs::write(
                &p,
                "[core]\n\texcludesFile = grimoire-test-no-global-ignore\n",
            );
            p
        });
        cmd.env("GIT_CONFIG_GLOBAL", path)
            .env("GIT_CONFIG_NOSYSTEM", "1");
        for key in [
            "GIT_AUTHOR_NAME",
            "GIT_AUTHOR_EMAIL",
            "GIT_COMMITTER_NAME",
            "GIT_COMMITTER_EMAIL",
            "EMAIL",
        ] {
            cmd.env_remove(key);
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("grimoire-sessions-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn cleanup(dirs: &[&Path]) {
        for d in dirs {
            let _ = fs::remove_dir_all(d);
        }
    }

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(native_path(rel));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    /// A scene file: frontmatter the word counts must ignore, then the prose.
    fn scene(words: usize) -> String {
        format!(
            "---\ntitle: \"A Scene\"\npov: Wren\nstatus: draft\n---\n\n{}\n",
            prose(words)
        )
    }

    fn prose(words: usize) -> String {
        (0..words)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    const A1: &str = "manuscript/01-Act-One/01-Chapter-One/01-Scene-One.md";
    const A2: &str = "manuscript/02-Act-Two/04-Chapter-Four/01-Scene-One.md";
    const A3: &str = "manuscript/03-Act-Three/07-Chapter-Seven/01-Scene-One.md";
    const WREN: &str = "notes/01-Characters/wren.md";

    /// A folder shaped like `grimoire new` leaves it, or `None` (with a note)
    /// when there's no git to test against.
    #[test]
    fn moving_the_cursor_alone_is_not_a_session() {
        let Some(d) = book("cursor-only") else { return };
        enable(&d).unwrap();
        assert!(!has_writing(&d), "nothing yet");
        write(&d, ".grimoire/resume.md", "---\nscene: x\nline: 3\n---\n");
        assert!(!has_writing(&d), "only where the cursor is");
        write(&d, A1, &scene(12));
        assert!(has_writing(&d), "words changed");
        assert!(!has_remote(&d));
        let _ = fs::remove_dir_all(&d);
    }

    fn book(tag: &str) -> Option<PathBuf> {
        if !git_available() {
            eprintln!("skipping {tag}: git isn't installed");
            return None;
        }
        let d = temp_dir(tag);
        write(&d, "novel.toml", "title = \"Test\"\npart_label = \"Act\"\n");
        write(&d, A1, &scene(10));
        write(&d, A2, &scene(0));
        write(&d, A3, &scene(0));
        write(&d, WREN, "---\nrole: lead\n---\nWren keeps the archive.\n");
        write(&d, ".grimoire/trash/01-Old-Scene.md", "words nobody wanted");
        write(&d, ".grimoire/progress.toml", "today = 0\n");
        write(&d, ".grimoire/resume.md", "Act One, Scene One\n");
        write(&d, ".gitignore", ".grimoire/\n");
        Some(d)
    }

    /// A fixed local time; the label reads the wall clock, so no time zone
    /// can change what these tests expect.
    fn at(month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Local> {
        let naive = NaiveDate::from_ymd_opt(2026, month, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap();
        Local.from_local_datetime(&naive).earliest().unwrap()
    }

    fn naive(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    /// Tuesday 15 September 2026, 7:30 p.m.
    fn tuesday_evening() -> DateTime<Local> {
        at(9, 15, 19, 30)
    }

    fn run(root: &Path, args: &[&str]) -> String {
        git(root, args.iter().copied()).text().unwrap()
    }

    fn committed_files(root: &Path) -> Vec<String> {
        run(root, &["ls-tree", "-r", "--name-only", "HEAD"])
            .lines()
            .map(String::from)
            .collect()
    }

    fn tally(acts: &[&str], notes: bool, before: usize, after: usize) -> Tally {
        Tally {
            acts: acts.iter().map(|a| a.to_string()).collect(),
            notes,
            words_before: before,
            words_after: after,
        }
    }

    // --- pure pieces --------------------------------------------------------

    #[test]
    fn parts_of_the_day_turn_over_on_the_hour() {
        assert_eq!(
            NaiveDate::from_ymd_opt(2026, 9, 15).unwrap().weekday(),
            Weekday::Tue
        );
        let cases = [
            (naive(15, 5, 0), "Tuesday morning"),
            (naive(15, 11, 59), "Tuesday morning"),
            (naive(15, 12, 0), "Tuesday lunchtime"),
            (naive(15, 13, 59), "Tuesday lunchtime"),
            (naive(15, 14, 0), "Tuesday afternoon"),
            (naive(15, 16, 59), "Tuesday afternoon"),
            (naive(15, 17, 0), "Tuesday evening"),
            (naive(15, 20, 59), "Tuesday evening"),
            (naive(15, 21, 0), "Tuesday night"),
            (naive(15, 23, 59), "Tuesday night"),
            // Past midnight is still the night before...
            (naive(16, 0, 0), "Tuesday night"),
            (naive(16, 4, 59), "Tuesday night"),
            // ...until morning.
            (naive(16, 5, 0), "Wednesday morning"),
        ];
        for (when, want) in cases {
            assert_eq!(when_phrase(when), want, "at {when}");
        }
    }

    #[test]
    fn numbers_get_thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1240), "1,240");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(words(1), "1 word");
        assert_eq!(words(12_000), "12,000 words");
    }

    #[test]
    fn labels_read_like_a_diary() {
        let eve = naive(15, 19, 30);
        assert_eq!(
            label(eve, &tally(&["02-Act-Two"], false, 300, 1540), "act"),
            "Tuesday evening · Act Two · 1,240 words"
        );
        assert_eq!(
            label(eve, &tally(&["02-Act-Two"], false, 0, 1), "act"),
            "Tuesday evening · Act Two · 1 word"
        );
        assert_eq!(
            label(eve, &tally(&["02-Act-Two"], true, 1000, 688), "act"),
            "Tuesday evening · Act Two · revised, 312 words cut"
        );
        assert_eq!(
            label(eve, &tally(&["02-Act-Two"], false, 500, 500), "act"),
            "Tuesday evening · Act Two · revised"
        );
        assert_eq!(
            label(eve, &tally(&[], true, 0, 0), "act"),
            "Tuesday evening · notes"
        );
        assert_eq!(
            label(
                naive(15, 9, 0),
                &tally(&["01-Act-One", "02-Act-Two"], true, 10, 60),
                "act"
            ),
            "Tuesday morning · Act One and Act Two · 50 words"
        );
        assert_eq!(
            label(
                eve,
                &tally(
                    &["01-Act-One", "02-Act-Two", "03-Act-Three"],
                    false,
                    10,
                    1010
                ),
                "act"
            ),
            "Tuesday evening · 3 acts · 1,000 words"
        );
        assert_eq!(
            label(
                eve,
                &tally(
                    &[
                        "01-part-one",
                        "02-part-two",
                        "03-part-three",
                        "04-part-four"
                    ],
                    false,
                    9,
                    1
                ),
                "part"
            ),
            "Tuesday evening · 4 parts · revised, 8 words cut"
        );
        // A scene straight under manuscript/ stands for itself.
        assert_eq!(
            label(eve, &tally(&["00-Prologue.md"], false, 0, 200), "act"),
            "Tuesday evening · Prologue · 200 words"
        );
        assert_eq!(
            label(
                eve,
                &tally(&["27-Chapter-Twenty-Seven"], false, 0, 2),
                "act"
            ),
            "Tuesday evening · Chapter Twenty-Seven · 2 words"
        );
        // Nothing a writer would call writing: just when.
        assert_eq!(
            label(eve, &tally(&[], false, 0, 0), "act"),
            "Tuesday evening"
        );
    }

    #[test]
    fn frontmatter_is_not_words() {
        assert_eq!(
            body_words("---\ntitle: \"Three Word Title\"\npov: Wren\n---\n\nOne two three.\n"),
            3
        );
        assert_eq!(
            body_words("---\r\ntitle: \"Three Word Title\"\r\n---\r\n\r\nOne two three.\r\n"),
            3
        );
        assert_eq!(body_words("\u{feff}---\ntitle: x\n---\nOne two\n"), 2);
        assert_eq!(body_words("No frontmatter here at all\n"), 5);
        // An unclosed block is prose, as the tree treats it.
        assert_eq!(body_words("---\nnot closed\n"), 3);
        assert_eq!(body_words(""), 0);
    }

    #[test]
    fn a_bare_grimoire_line_becomes_the_pair_and_everything_else_stays() {
        assert_eq!(
            gitignore_keeping_resume(".grimoire/\n"),
            ".grimoire/*\n!.grimoire/resume.md\nexports/\n"
        );
        assert_eq!(
            gitignore_keeping_resume("# mine\n*.docx\n.grimoire/\n.DS_Store\n"),
            "# mine\n*.docx\n.grimoire/*\n!.grimoire/resume.md\n.DS_Store\nexports/\n"
        );
        assert_eq!(
            gitignore_keeping_resume(""),
            ".grimoire/*\n!.grimoire/resume.md\nexports/\n"
        );
        assert_eq!(
            gitignore_keeping_resume("*.pdf"),
            "*.pdf\n.grimoire/*\n!.grimoire/resume.md\nexports/\n"
        );
        assert_eq!(
            gitignore_keeping_resume(".grimoire/*\n*.pdf\n"),
            ".grimoire/*\n!.grimoire/resume.md\n*.pdf\nexports/\n"
        );
        assert_eq!(
            gitignore_keeping_resume("!.grimoire/resume.md\n.grimoire/*\n"),
            "!.grimoire/resume.md\n.grimoire/*\n!.grimoire/resume.md\nexports/\n"
        );
        assert_eq!(
            gitignore_keeping_resume("*.pdf\r\n.grimoire/\r\n"),
            "*.pdf\r\n.grimoire/*\r\n!.grimoire/resume.md\r\nexports/\r\n"
        );
        let done = ".grimoire/*\n!.grimoire/resume.md\nexports/\n";
        assert_eq!(
            gitignore_keeping_resume(done),
            done,
            "already right: unchanged"
        );
    }

    #[test]
    fn git_output_parses() {
        let raw =
            b"M\0manuscript/a.md\0R087\0notes/old name.md\0notes/new name.md\0A\0x.md\0D\0y.md\0";
        let e = parse_name_status(raw);
        assert_eq!(e.len(), 4);
        assert_eq!(e[1].status, Status::Renamed);
        assert_eq!(e[1].from.as_deref(), Some("notes/old name.md"));
        assert_eq!(e[1].path, "notes/new name.md");
        assert_eq!((e[2].status, e[3].status), (Status::Added, Status::Deleted));

        let batch = b"abc blob 5\nhello\nHEAD:my scene.md missing\ndef tree 3\nxyz\nfff blob 0\n\n";
        assert_eq!(
            parse_batch(batch, 4),
            vec![Some(b"hello".to_vec()), None, None, Some(Vec::new())]
        );

        let log = "1a2b3c4\x1f1789000000\x1fTuesday evening · Act Two, Act Three · 1,240 words\0\
                   9f8e7d6\x1f1788900000\x1fSession history begins";
        let s = parse_log(log.as_bytes());
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].hash, "1a2b3c4");
        assert_eq!(
            s[0].label,
            "Tuesday evening · Act Two, Act Three · 1,240 words"
        );
        assert_eq!(s[0].when.timestamp(), 1_789_000_000);
        assert_eq!(s[1].label, "Session history begins");
    }

    // --- with real git -----------------------------------------------------

    #[test]
    fn enabling_a_new_book_ignores_machine_state_and_saves_the_first_session() {
        let Some(d) = book("enable") else { return };
        assert!(!is_enabled(&d));
        enable(&d).unwrap();
        assert!(is_enabled(&d));
        assert_eq!(
            fs::read_to_string(d.join(".gitignore")).unwrap(),
            ".grimoire/*\n!.grimoire/resume.md\nexports/\n"
        );
        assert_eq!(run(&d, &["symbolic-ref", "--short", "HEAD"]), "main");

        let history = sessions(&d, 10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].label, "Session history begins");
        assert!((Local::now() - history[0].when).num_minutes().abs() < 5);

        let files = committed_files(&d);
        for want in [
            A1,
            A2,
            A3,
            WREN,
            "novel.toml",
            ".gitignore",
            ".grimoire/resume.md",
        ] {
            assert!(
                files.iter().any(|f| f == want),
                "{want} should be committed: {files:?}"
            );
        }
        assert!(
            !files
                .iter()
                .any(|f| f.starts_with(".grimoire/trash") || f == ".grimoire/progress.toml")
        );
        // No identity configured anywhere: the fallback signs it.
        assert_eq!(
            run(&d, &["log", "-1", "--format=%an <%ae>"]),
            "Grimoire <grimoire@localhost>"
        );

        // Again changes nothing.
        enable(&d).unwrap();
        assert_eq!(sessions(&d, 10).unwrap().len(), 1);

        // The trash stays out of later sessions; resume.md comes along.
        write(&d, ".grimoire/trash/02-Cut-Chapter.md", "gone");
        write(&d, ".grimoire/resume.md", "Act Two, Scene One\n");
        assert_eq!(
            pending_label(&d, tuesday_evening()).unwrap().as_deref(),
            Some("Tuesday evening")
        );
        commit_session(&d, tuesday_evening()).unwrap().unwrap();
        let files = committed_files(&d);
        assert!(!files.iter().any(|f| f.starts_with(".grimoire/trash")));
        assert_eq!(
            file_at(&d, "HEAD", Path::new(".grimoire/resume.md"))
                .unwrap()
                .as_deref(),
            Some("Act Two, Scene One\n")
        );
        cleanup(&[&d]);
    }

    #[test]
    fn nothing_changed_means_no_session() {
        let Some(d) = book("unchanged") else { return };
        enable(&d).unwrap();
        assert_eq!(pending_label(&d, tuesday_evening()).unwrap(), None);
        assert_eq!(commit_session(&d, tuesday_evening()).unwrap(), None);
        // Rewriting a file with the same words, or touching only ignored
        // state, isn't a session either.
        fs::write(d.join(native_path(A1)), scene(10)).unwrap();
        write(&d, ".grimoire/progress.toml", "today = 900\n");
        assert_eq!(pending_label(&d, tuesday_evening()).unwrap(), None);
        assert_eq!(commit_session(&d, tuesday_evening()).unwrap(), None);
        assert_eq!(sessions(&d, 10).unwrap().len(), 1);
        cleanup(&[&d]);
    }

    #[test]
    fn labels_come_from_what_actually_changed() {
        let Some(d) = book("labels") else { return };
        enable(&d).unwrap();

        // Words added in one act.
        write(&d, A2, &scene(1240));
        let want = "Tuesday evening · Act Two · 1,240 words";
        assert_eq!(
            pending_label(&d, tuesday_evening()).unwrap().as_deref(),
            Some(want)
        );
        // Looking didn't stage anything.
        assert!(
            git(&d, ["diff", "--cached", "--quiet"])
                .run()
                .ok()
                .unwrap()
                .success
        );
        let hash = commit_session(&d, tuesday_evening()).unwrap().unwrap();
        let latest = &sessions(&d, 1).unwrap()[0];
        assert_eq!(
            (latest.hash.as_str(), latest.label.as_str()),
            (hash.as_str(), want)
        );

        // Words cut.
        write(&d, A2, &scene(928));
        let wed_morning = at(9, 16, 8, 0);
        assert_eq!(
            commit_label(&d, wed_morning),
            "Wednesday morning · Act Two · revised, 312 words cut"
        );

        // Only the frontmatter changed: text changed, words didn't.
        write(
            &d,
            A2,
            &scene(928).replace("status: draft", "status: revised"),
        );
        assert_eq!(
            commit_label(&d, at(9, 16, 12, 30)),
            "Wednesday lunchtime · Act Two · revised"
        );

        // Only notes.
        write(
            &d,
            WREN,
            "---\nrole: lead\n---\nWren keeps the archive, and its secrets.\n",
        );
        assert_eq!(
            commit_label(&d, at(9, 16, 15, 0)),
            "Wednesday afternoon · notes"
        );

        // Two acts, and notes alongside don't change the where.
        write(&d, A1, &scene(30));
        write(&d, A3, &scene(5));
        write(&d, WREN, "Rewritten.\n");
        assert_eq!(
            commit_label(&d, at(9, 16, 22, 0)),
            "Wednesday night · Act One and Act Three · 25 words"
        );

        // Three acts.
        write(&d, A1, &scene(31));
        write(&d, A2, &scene(929));
        write(&d, A3, &scene(6));
        assert_eq!(
            commit_label(&d, at(9, 17, 2, 0)),
            "Wednesday night · 3 acts · 3 words"
        );

        // A scene moved from Act One to Act Two, unchanged.
        let moved = "manuscript/02-Act-Two/05-Chapter-Five/01-Scene-One.md";
        fs::create_dir_all(d.join(native_path(moved)).parent().unwrap()).unwrap();
        fs::rename(d.join(native_path(A1)), d.join(native_path(moved))).unwrap();
        assert_eq!(
            commit_label(&d, tuesday_evening()),
            "Tuesday evening · Act One and Act Two · revised"
        );

        // A scene deleted, and a new one added.
        fs::remove_file(d.join(native_path(A3))).unwrap();
        assert_eq!(
            commit_label(&d, tuesday_evening()),
            "Tuesday evening · Act Three · revised, 6 words cut"
        );
        write(
            &d,
            "manuscript/03-Act-Three/08-Chapter-Eight/01-Scene-One.md",
            &scene(12),
        );
        assert_eq!(
            commit_label(&d, tuesday_evening()),
            "Tuesday evening · Act Three · 12 words"
        );

        let labels: Vec<String> = sessions(&d, 100)
            .unwrap()
            .into_iter()
            .map(|s| s.label)
            .collect();
        assert_eq!(labels.len(), 10);
        assert_eq!(labels[0], "Tuesday evening · Act Three · 12 words");
        assert_eq!(labels[9], "Session history begins");
        assert_eq!(sessions(&d, 3).unwrap().len(), 3);
        cleanup(&[&d]);
    }

    /// Check the pending label matches what gets committed, then return it.
    fn commit_label(d: &Path, now: DateTime<Local>) -> String {
        let pending = pending_label(d, now).unwrap().expect("something to commit");
        let hash = commit_session(d, now).unwrap().expect("a commit");
        let latest = sessions(d, 1).unwrap().remove(0);
        assert_eq!(latest.hash, hash);
        assert_eq!(latest.label, pending);
        pending
    }

    #[test]
    fn changes_count_words_both_sides_and_follow_renames() {
        let Some(d) = book("changes") else { return };
        let long: String = (1..=20)
            .map(|i| format!("Line {i} of a scene that moves.\n"))
            .collect();
        let d_old = "manuscript/01-Act-One/01-Chapter-One/02-Scene-Two.md";
        let gone = "manuscript/01-Act-One/01-Chapter-One/03-Scene-Three.md";
        write(&d, d_old, &format!("---\ntitle: Two\n---\n{long}"));
        // Nothing like the scene added later, so git can't pair the two up
        // as a rename.
        write(&d, gone, "Five words that were cut.\n");
        enable(&d).unwrap();

        // The first session is everything, compared with nothing.
        let first = sessions(&d, 1).unwrap().remove(0);
        let root_changes = changes(&d, &first.hash).unwrap();
        assert_eq!(root_changes.len(), 6, "{root_changes:?}");
        assert!(
            root_changes
                .iter()
                .all(|c| c.kind == ChangeKind::Added && c.words_before == 0)
        );
        let a1 = root_changes
            .iter()
            .find(|c| c.path == native_path(A1))
            .unwrap();
        assert_eq!(a1.words_after, 10);

        write(&d, A1, &scene(25));
        let added = "manuscript/02-Act-Two/04-Chapter-Four/02-Scene-Two.md";
        write(&d, added, &scene(7));
        fs::remove_file(d.join(native_path(gone))).unwrap();
        let d_new = "manuscript/02-Act-Two/04-Chapter-Four/03-The-Move.md";
        fs::remove_file(d.join(native_path(d_old))).unwrap();
        write(
            &d,
            d_new,
            &format!("---\ntitle: Two\n---\n{long}One more line.\n"),
        );
        write(
            &d,
            "novel.toml",
            "title = \"Retitled\"\npart_label = \"Act\"\n",
        );
        let hash = commit_session(&d, tuesday_evening()).unwrap().unwrap();

        let got = changes(&d, &hash).unwrap();
        let want = vec![
            Change {
                path: native_path(A1),
                kind: ChangeKind::Modified,
                words_before: 10,
                words_after: 25,
            },
            Change {
                path: native_path(gone),
                kind: ChangeKind::Deleted,
                words_before: 5,
                words_after: 0,
            },
            Change {
                path: native_path(added),
                kind: ChangeKind::Added,
                words_before: 0,
                words_after: 7,
            },
            Change {
                path: native_path(d_new),
                kind: ChangeKind::Renamed {
                    from: native_path(d_old),
                },
                words_before: 140,
                words_after: 143,
            },
        ];
        let mut got_sorted = got.clone();
        got_sorted.sort_by(|a, b| a.path.cmp(&b.path));
        let mut want_sorted = want;
        want_sorted.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(got_sorted, want_sorted);
        assert!(changes(&d, "no-such-session").is_err());
        assert!(changes(&d, "--output=oops").is_err());
        cleanup(&[&d]);
    }

    #[test]
    fn a_file_as_it_stood_in_a_session() {
        let Some(d) = book("file-at") else { return };
        enable(&d).unwrap();
        let first = sessions(&d, 1).unwrap().remove(0).hash;
        write(&d, A1, &scene(3));
        fs::remove_file(d.join(native_path(A3))).unwrap();
        let second = commit_session(&d, tuesday_evening()).unwrap().unwrap();

        // Native separators in, forward slashes to git.
        let a1 = Path::new("manuscript")
            .join("01-Act-One")
            .join("01-Chapter-One")
            .join("01-Scene-One.md");
        assert_eq!(file_at(&d, &first, &a1).unwrap(), Some(scene(10)));
        assert_eq!(file_at(&d, &second, &a1).unwrap(), Some(scene(3)));
        assert_eq!(file_at(&d, &second, &d.join(&a1)).unwrap(), Some(scene(3)));
        assert_eq!(
            file_at(&d, &first, &native_path(A3)).unwrap(),
            Some(scene(0))
        );
        assert_eq!(file_at(&d, &second, &native_path(A3)).unwrap(), None);
        assert_eq!(
            file_at(&d, &first, Path::new("manuscript/nowhere.md")).unwrap(),
            None
        );
        // A folder isn't a file.
        assert_eq!(file_at(&d, &first, Path::new("manuscript")).unwrap(), None);
        assert!(file_at(&d, &first, Path::new("../outside.md")).is_err());
        assert!(file_at(&d, "0000000", &a1).is_err());
        cleanup(&[&d]);
    }

    #[test]
    fn pushing_to_a_bare_repository() {
        let Some(d) = book("push") else { return };
        enable(&d).unwrap();
        assert_eq!(push(&d), PushOutcome::NoRemote);
        assert_eq!(unpushed(&d), None);

        let remote = temp_dir("push-remote");
        run(&remote, &["init", "--bare", "-q", "."]);
        git(&d, ["remote", "add", "origin"])
            .arg_path(&remote)
            .stdout()
            .unwrap();
        assert_eq!(unpushed(&d), None, "no upstream until the first push");

        assert_eq!(push(&d), PushOutcome::Pushed);
        assert_eq!(unpushed(&d), Some(0));

        write(&d, A2, &scene(50));
        commit_session(&d, tuesday_evening()).unwrap().unwrap();
        write(&d, A2, &scene(60));
        commit_session(&d, tuesday_evening()).unwrap().unwrap();
        assert_eq!(unpushed(&d), Some(2));

        assert_eq!(push(&d), PushOutcome::Pushed);
        assert_eq!(unpushed(&d), Some(0));
        assert_eq!(
            run(&remote, &["rev-parse", "refs/heads/main"]),
            run(&d, &["rev-parse", "HEAD"])
        );
        // Nothing new still counts as pushed.
        assert_eq!(push(&d), PushOutcome::Pushed);
        cleanup(&[&d, &remote]);
    }

    #[test]
    fn a_broken_remote_fails_quickly() {
        let Some(d) = book("push-broken") else { return };
        enable(&d).unwrap();
        let nowhere = std::env::temp_dir().join(format!(
            "grimoire-sessions-no-remote-{}.git",
            std::process::id()
        ));
        git(&d, ["remote", "add", "origin"])
            .arg_path(&nowhere)
            .stdout()
            .unwrap();
        let started = Instant::now();
        let outcome = push(&d);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "took {:?}",
            started.elapsed()
        );
        match outcome {
            PushOutcome::Failed(why) => assert!(!why.is_empty()),
            other => panic!("expected a failure, got {other:?}"),
        }
        assert_eq!(unpushed(&d), None);
        cleanup(&[&d]);
    }

    #[test]
    fn a_remote_that_never_answers_is_given_up_on() {
        let Some(d) = book("push-silent") else { return };
        enable(&d).unwrap();
        // Accepts the connection (the OS does that), then says nothing, ever.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://127.0.0.1:{}/book.git",
            listener.local_addr().unwrap().port()
        );
        run(&d, &["remote", "add", "origin", &url]);
        let started = Instant::now();
        let outcome = push_within(&d, Duration::from_secs(2));
        let took = started.elapsed();
        assert!(took < Duration::from_secs(10), "took {took:?}");
        match outcome {
            PushOutcome::Failed(why) => assert!(why.contains("didn't answer"), "{why}"),
            other => panic!("expected a failure, got {other:?}"),
        }
        // Giving up has to mean git and its HTTP helper are gone, not left
        // hanging on the line: the far end of the connection is closed. (A
        // system that drops a dead connection before it's accepted is fine
        // too — a live client is always still waiting to be accepted.)
        listener.set_nonblocking(true).unwrap();
        let waiting = Instant::now();
        let conn = loop {
            match listener.accept() {
                Ok((conn, _)) => break Some(conn),
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        && waiting.elapsed() < Duration::from_secs(3) =>
                {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(_) => break None,
            }
        };
        if let Some(mut conn) = conn {
            conn.set_nonblocking(false).unwrap();
            conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            match conn.read_to_end(&mut request) {
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
                    ) => {}
                Err(e) => panic!("git is still holding the connection open ({e})"),
            }
        }
        cleanup(&[&d]);
    }

    #[test]
    fn the_writers_own_identity_wins_over_the_fallback() {
        let Some(d) = book("identity") else { return };
        enable(&d).unwrap();
        run(&d, &["config", "user.name", "Wren Marsh"]);
        run(&d, &["config", "user.email", "wren@example.com"]);
        write(&d, A2, &scene(20));
        commit_session(&d, tuesday_evening()).unwrap().unwrap();
        assert_eq!(
            run(&d, &["log", "-1", "--format=%an <%ae>"]),
            "Wren Marsh <wren@example.com>"
        );
        cleanup(&[&d]);
    }

    #[test]
    fn a_book_inside_someone_elses_repository_is_left_alone_until_enabled() {
        if !git_available() {
            eprintln!("skipping nested: git isn't installed");
            return;
        }
        let outer = temp_dir("outer");
        run(&outer, &["init", "-q", "."]);
        let d = outer.join("book");
        write(&d, A1, &scene(10));
        assert!(!is_enabled(&d));
        assert!(commit_session(&d, tuesday_evening()).is_err());
        assert!(pending_label(&d, tuesday_evening()).is_err());
        assert!(sessions(&d, 10).is_err());
        assert_eq!(
            push(&d),
            PushOutcome::Failed(format!(
                "session history isn't turned on for {}",
                d.display()
            ))
        );
        assert!(
            git(&outer, ["rev-parse", "--verify", "-q", "HEAD"])
                .lookup()
                .unwrap()
                .is_none()
        );

        enable(&d).unwrap();
        assert!(is_enabled(&d));
        assert_eq!(sessions(&d, 10).unwrap().len(), 1);
        // The outer repository still has no commits and nothing staged.
        assert!(
            git(&outer, ["rev-parse", "--verify", "-q", "HEAD"])
                .lookup()
                .unwrap()
                .is_none()
        );
        assert_eq!(run(&outer, &["ls-files"]), "");
        cleanup(&[&outer]);
    }

    #[test]
    fn without_git_everything_says_so() {
        TEST_GIT_PROGRAM.with(|p| p.set(Some("grimoire-test-no-such-git")));
        let d = temp_dir("no-git");
        write(&d, A1, &scene(10));
        write(&d, ".gitignore", ".grimoire/\n");

        assert!(!git_available());
        assert!(!is_enabled(&d));
        let err = enable(&d).unwrap_err().to_string();
        assert!(err.contains("git isn't installed"), "{err}");
        assert!(pending_label(&d, tuesday_evening()).is_err());
        assert!(commit_session(&d, tuesday_evening()).is_err());
        assert!(sessions(&d, 10).is_err());
        assert!(changes(&d, "HEAD").is_err());
        assert!(file_at(&d, "HEAD", Path::new(A1)).is_err());
        assert_eq!(unpushed(&d), None);
        match push(&d) {
            PushOutcome::Failed(why) => assert!(why.contains("git isn't installed"), "{why}"),
            other => panic!("expected a failure, got {other:?}"),
        }
        // Nothing was written on the way to failing.
        assert!(!d.join(".git").exists());
        assert_eq!(
            fs::read_to_string(d.join(".gitignore")).unwrap(),
            ".grimoire/\n"
        );

        TEST_GIT_PROGRAM.with(|p| p.set(None));
        cleanup(&[&d]);
    }

    impl Git {
        fn arg_path(mut self, p: &Path) -> Self {
            self.args.push(p.as_os_str().to_owned());
            self
        }
    }
}
