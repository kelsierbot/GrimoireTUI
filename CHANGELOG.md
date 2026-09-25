# Changelog

## 0.4.0

### Keeping every word

- **Changes made elsewhere are never overwritten.** If Obsidian, a sync app or
  the phone changes a scene while Grimoire is open, Grimoire shows the new
  version; if you had unsaved words in it too, they're kept beside it as
  "Scene (from bazzite, …)", marked *parked copy* in the tree. Scenes added,
  moved or deleted outside Grimoire show up by themselves; a deleted scene is
  never brought back, and any unsaved words from it go to the trash.
- **One file that isn't UTF-8 no longer stops a book opening.** It's shown
  read-only and never rewritten.
- **The recovered-words box never throws words away on a keystroke:** choose
  with ↑↓ and `Enter`. "Keep the saved versions" puts the recovered words in
  the trash rather than deleting them.
- **Moving a section right after typing** no longer loses the last few
  seconds of writing.
- **Two things with the same name deleted in the same second** both stay in
  the trash.
- **A move that can't finish** (a locked folder, a sync app holding a file)
  puts everything back, and anything a crash left mid-move reappears in the
  tree next time the book opens.
- **Undoing a move keeps its link fixes**, and a new scene no longer inherits
  a deleted scene's undo or history.
- **Find and replace match whole words in their exact case** by default —
  `Ctrl-W` for inside words, `Ctrl-E` for any case — and one `Ctrl-Z` undoes a
  replace across the whole book.

### Writing

- **Focus mode** (`Ctrl-D`): just your prose, centred, with a quiet status
  line. `Esc` still opens the menu; `Ctrl-D` again brings everything back.
- **A readable line length**: prose wraps at 72 columns, centred. *Line
  width* in Settings changes it (60 / 72 / 80 / 100 / full).
- **Typewriter scrolling**: in focus mode the line you're writing stays
  mid-screen; elsewhere you're never drafting on the bottom row.
- **Notes that never reach the book**: `%% like this %%` (Obsidian's comment
  syntax, may cross lines) and TK — dimmed in the editor, left out of every
  word count, compile and export, kept on disk exactly as typed. **`Ctrl-T`**
  lists every note and TK in the book; type to filter, `Enter` goes there.
- **Sprints**: *Start a sprint…* sets a word goal and minutes, starts the
  timer, and the status bar counts `sprint 212/500 · 14:32`. A scene's
  `target:` shows as progress in the editor's title.
- **Revision passes**: *Next scene still in draft*, and *Echo words* (the same
  word again within forty).
- **A scene beside**: `v` in the tree shows another scene, read-only, next to
  the one you're writing. `Esc` closes it; `Enter` switches to writing in it.
- **An empty scene says how to start.**

### Around the desk

- **The Esc menu has the new tools**: focus mode, a scene beside, notes &
  TKs, next scene in draft, echo words and sprints; Settings has line width
  and typewriter scrolling. Rows that can't do anything right now are hidden,
  and the menu scrolls on a short terminal. From the editor, the menu's
  New/Rename/Delete act on the scene you're writing, as `Ctrl-K`'s always did.
  Lists answer PgUp/PgDn and Home/End throughout.
- **Export… is in the menu** — Word, EPUB or Markdown from one dialog, which
  now fits an 80-column terminal. Compile and the project map are still in
  `Ctrl-K`.
- **A new book opens on its Novel Format guide**, written as plain prose.
- **New books count in parts again** (Part One, `p` new part). Older books
  keep their folders' names — a book an earlier version switched to pages
  keeps its pages, and one with its own label (Act, Book) is untouched.
- **The spectrum works on Linux**: it hears whatever the machine plays through
  PipeWire (`pw-record`) or PulseAudio (`parec`), only while the view is
  showing; with neither installed it says so instead of sitting blank.
  `GRIMOIRE_MONITOR=<sink>` listens to a different output.
- The menu leaves out *Music player* while music is off; history compares each
  version with now ("6 words shorter"); resuming an empty scene says it was
  left open; corkboard cards without a synopsis show the scene's first line; a
  character note's title names its section.

### Under the hood

- The book's logic lives in `grimoire-core`, shared with the phone app, which
  now builds from the same repository.
- Tests run on Linux, macOS and Windows for every change, with clippy and
  rustfmt enforced.

## 0.3.6

- **`Esc` closes an open character note** whichever pane you're in — 0.3.5
  promised this but only did it with the note itself focused; from the editor
  it opened the menu and left the note up. `Ctrl-O` on the same name (or on
  none) now puts the note away too.
- **The status bar leads with what `Esc` does**, so an 80-column terminal no
  longer cuts "Esc menu" off the end.

## 0.3.5

- **`Esc` opens the menu**, from any pane — no more reaching for `F1`, which
  many keyboards only send with `Fn` held (it still works). `Esc` again closes
  it and puts you back where you were, so it no longer drops you from the
  editor into the tree; `Shift-Tab` does that. A selection in the editor, or
  an open note beside it, is closed by the first `Esc`.
- **Quit Grimoire** is in the menu, at the bottom. It saves everything first,
  like `Ctrl-Q`.

## 0.3.4

- **"today" always moves when you write.** It's the day's net change in the
  manuscript, shown below zero (`today −29`) after cutting more than you've
  written, instead of sitting at 0 until you'd written the cut words back.
  Deleting a scene or bringing it back with `Ctrl-Z` doesn't change it, cut and
  paste within the book comes out even, and it starts over at midnight even if
  Grimoire stays open.

## 0.3.3

- **Closing the terminal window really closes Grimoire.** It used to be left
  running, stuck at full CPU on the closed terminal (a crossterm reader that
  never returns), without the save-on-close ever happening. Keys are now read on
  their own thread, so closing the window saves and exits in a moment.

## 0.3.2

- **Everything in the tree moves**: drag a section, or press `K`/`J` on it, to
  reorder the sections (saved in `novel.toml`; the trash stays last). Notes
  without a number in their filename can be moved too — the folder is numbered
  on the first move. All of it undoes with `Ctrl-Z`.
- **A shorter menu**: themes, music source, music, spellcheck and tree icons
  are in `F1` → **Settings…**, which opens a menu of its own.

## 0.3.1

- **Undo in the tree**: `Ctrl-Z` outside the editor takes back a delete,
  rename, move (a whole drag at once) or new item; `Ctrl-Y` does it again. A
  deleted item comes back out of the trash to where it was. Typing is still
  undone from the editor.
- **Tree icons are optional** and start off: F1 → *Turn tree icons on*.

## 0.3.0

- **A tree in sections**: Novel Format, Manuscript, Characters, Places, Front
  Matter, Notes, Research, Template Sheets and Trash, each with its own symbol,
  every section and folder foldable, headed by the book's title.
- **Pages** are the book's largest division (they were parts): `p page`.
- **Front matter by edition** — Manuscript Format, Paperback, Ebook — with
  starter title, copyright and dedication pages, left out of exports until
  you include them. The EPUB takes the Ebook pages; Word takes the Manuscript
  Format ones.
- **Template Sheets** for a character and a setting, and a Novel Format guide.
- **Older books are upgraded once, on opening**: notebook folders move into
  their sections, parts become pages, links follow.
- Popup keys stand out in the accent colour; the clearing pane has a view
  switcher (`◂ clearing spectrum garden ▸`); the garden is titled.

## 0.2.0

Nothing written is one keystroke from gone, and a hundred scenes are easy to
find and shape.

- **Autosave**, two seconds after you stop typing, through temporary files so
  a crash can't tear a scene; closing the window or quitting saves first.
- **Recovery**: words that couldn't be saved are offered back next launch.
- **Undo and redo**, a word at a time; typing replaces a selection; paste is
  one step; Ctrl-X cuts.
- **Scene history** (`H`): dated versions with a word diff; restore any.
- **Command palette** (`Ctrl-K`): every action, scene, note and theme by name.
- **Find and replace** in a scene (`Ctrl-F`) or across the book (`/`), with a
  confirm before replacing across scenes.
- **Name drift**: near-miss spellings of notebook names, fixed in one go.
- **Spellcheck** with a bundled en_US dictionary that knows your notebook's
  names; `F8` for suggestions; words you add go in `dictionary.txt`.
- **Move scenes and chapters** (`Alt-↑/↓`, `K`/`J`, or drag in the tree),
  across chapters and acts, with `[[links]]` kept pointing at the right files.
- **The corkboard** (`b`): index cards with POV colours, status, synopsis and
  words against target; filter by POV; edit on the card.
- **The codex** (`Ctrl-O`): a notebook note beside the scene, with every scene
  that mentions it.
- **Export for readers**: Word (standard manuscript format), EPUB and Markdown,
  whole book or chosen acts; also `grimoire export`.
- **Resume** where you left off, on any computer the book reaches.
- **Writing sessions**: optional Git history labelled like a diary, with
  background backup to a remote.
- **Music is off by default**; turn it on from the menu or palette.

## 0.1.0

The first release, and the first with ready-to-run downloads — no Rust
toolchain needed.

Built for Windows (x64), macOS (Apple Silicon and Intel) and Linux (x86_64 and
ARM64); the one-line installers are below.

**What's in it**

- A manuscript that is just folders and Markdown files — the directory tree is
  the outline. New books start as three acts of nine chapters with three scenes
  each, plus a notebook and a visible trash.
- Create, rename and delete parts, chapters and scenes from the tree; deleting
  moves things to the trash rather than destroying them.
- Word counts and daily progress, a project map, and `grimoire compile` to
  assemble the manuscript.
- Music while you write: YouTube Music, Spotify (macOS and Linux), Jellyfin and
  Plex. `grimoire music-setup youtube-music` installs and connects YouTube Music
  on all three systems.
- Themes, switched with `F9`.
