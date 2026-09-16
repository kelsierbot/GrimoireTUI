# Changelog

## 0.2.0 (unreleased)

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
