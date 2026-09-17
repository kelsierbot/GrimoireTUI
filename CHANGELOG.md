# Changelog

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
